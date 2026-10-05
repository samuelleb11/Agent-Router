package ca.whnet.agentrouter.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.api.AuthStatus
import ca.whnet.agentrouter.api.Health
import ca.whnet.agentrouter.api.RouterClient
import ca.whnet.agentrouter.api.RouterFailure
import ca.whnet.agentrouter.api.get
import ca.whnet.agentrouter.data.AppState
import ca.whnet.agentrouter.data.Router
import ca.whnet.agentrouter.data.RouterAddress
import ca.whnet.agentrouter.data.RouterState
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RouterFormScreen(
    app: AppState,
    existing: Router?,
    onDone: () -> Unit,
    onRemoved: () -> Unit = onDone,
) {
    var name by rememberSaveable { mutableStateOf(existing?.name ?: "") }
    // A hosted router is edited as the URL it was added by; a router on a Mac as host and port.
    var host by rememberSaveable {
        mutableStateOf(existing?.let { if (it.scheme == Router.Scheme.Https) it.baseUrl else it.host } ?: "")
    }
    var port by rememberSaveable {
        mutableStateOf((existing?.takeIf { it.scheme == Router.Scheme.Http }?.port ?: 8787).toString())
    }
    var password by remember { mutableStateOf("") }
    var probe by remember { mutableStateOf<String?>(null) }
    var probeOk by remember { mutableStateOf(false) }
    var testing by remember { mutableStateOf(false) }
    var confirmRemove by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    // What Test connection heard the router say about being hosted; a change of address
    // forgets it.
    var testedHosted by rememberSaveable { mutableStateOf<Boolean?>(null) }
    // Read once: asking AppState for it again rebuilds the client, password and all.
    val saved = remember(existing?.id) { existing?.let { app.state(it) } }

    val address = RouterAddress.parse(host, port)
    val urlTyped = RouterAddress.isUrl(host)
    val valid = name.isNotBlank() && address is RouterAddress.Valid &&
        (existing != null || password.isNotEmpty())

    // Whether the router at this address says it is hosted, which decides who the copy says
    // sets its password: as Test connection heard it, or, for a saved router still at its
    // own address, as its own page last did. Until the router has said, the copy is a
    // router on a Mac's, as it always was.
    val hosted = testedHosted
        ?: (saved?.isHosted == true && (address as? RouterAddress.Valid)?.baseUrl == existing?.baseUrl)

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(if (existing == null) "Add a router" else "Edit router") },
                navigationIcon = {
                    IconButton(onClick = onDone) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
                },
                actions = {
                    TextButton(
                        enabled = valid,
                        onClick = {
                            val a = address as? RouterAddress.Valid ?: return@TextButton
                            val r = Router(
                                id = existing?.id ?: java.util.UUID.randomUUID().toString(),
                                name = name.trim(), host = a.host, port = a.port, scheme = a.scheme,
                            )
                            if (existing == null) app.add(r, password) else app.update(r, password)
                            onDone()
                        },
                    ) { Text("Save") }
                },
            )
        },
    ) { pad ->
        Column(
            Modifier.fillMaxSize().padding(pad).readableWidth(640.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            SettingsSection(
                header = "Address",
                footer = "For a router on a Mac, use the Mac's LAN IP address, not its name. That " +
                    "router refuses a request from another device whose address is a hostname — " +
                    "that is how it guards against a web page in your browser reaching it. A " +
                    "hosted router is the other way round: it is reached by its name, so type " +
                    "its https URL, such as https://agent-router.example.com.",
            ) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    OutlinedTextField(
                        value = name, onValueChange = { name = it },
                        label = { Text("Name") }, singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        OutlinedTextField(
                            value = host, onValueChange = { host = it; probe = null; testedHosted = null },
                            label = { Text("Host or URL") }, singleLine = true,
                            placeholder = { Text("10.0.0.5") },
                            isError = address is RouterAddress.Invalid && !address.inPort,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                            modifier = Modifier.weight(2f),
                        )
                        // A URL brings its own port, so the field shows that one and stands aside.
                        OutlinedTextField(
                            value = if (urlTyped) (address as? RouterAddress.Valid)?.port?.toString().orEmpty() else port,
                            onValueChange = { port = it.filter(Char::isDigit); probe = null; testedHosted = null },
                            enabled = !urlTyped,
                            label = { Text("Port") }, singleLine = true,
                            isError = address is RouterAddress.Invalid && address.inPort,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                            modifier = Modifier.weight(1f),
                        )
                    }
                    if (address is RouterAddress.Invalid) {
                        Text(
                            address.why,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error,
                        )
                    }
                }
            }

            SettingsSection(
                header = "Password",
                footer = if (existing != null) {
                    "Leave this blank to keep the password already stored."
                } else if (hosted) {
                    "The router password, set by its deployment. It is kept in this phone's " +
                        "keystore, not in the file that holds the address."
                } else {
                    "The router password, set on its dashboard on the Mac. It is kept in this " +
                        "phone's keystore, not in the file that holds the address."
                },
            ) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    OutlinedTextField(
                        value = password, onValueChange = { password = it; probe = null },
                        label = { Text(if (existing == null) "Password" else "New password") },
                        singleLine = true,
                        visualTransformation = PasswordVisualTransformation(),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        OutlinedButton(
                            enabled = !testing && address is RouterAddress.Valid,
                            onClick = {
                                val base = (address as? RouterAddress.Valid)?.baseUrl ?: return@OutlinedButton
                                testing = true
                                scope.launch {
                                    val pw = password.ifEmpty { existing?.let { app.store.password(it.id) } }
                                    val result = testConnection(base, pw, hosted)
                                    probe = result.text
                                    probeOk = result.ok
                                    result.auth?.let { testedHosted = it.isHosted }
                                    testing = false
                                }
                            },
                        ) { Text(if (testing) "Testing…" else "Test connection") }
                    }
                    if (probe != null) {
                        Text(
                            probe!!,
                            style = MaterialTheme.typography.bodyMedium,
                            color = if (probeOk) MaterialTheme.colorScheme.primary
                            else MaterialTheme.colorScheme.error,
                        )
                    }
                }
            }

            if (existing != null) {
                SettingsSection(footer = "Removing a router forgets its password on this phone too.") {
                    Column(Modifier.padding(16.dp)) {
                        Button(
                            onClick = { confirmRemove = true },
                            colors = androidx.compose.material3.ButtonDefaults.buttonColors(
                                containerColor = MaterialTheme.colorScheme.errorContainer,
                                contentColor = MaterialTheme.colorScheme.onErrorContainer,
                            ),
                            modifier = Modifier.fillMaxWidth(),
                        ) { Text("Remove this router") }
                    }
                }
            }
        }
    }

    if (confirmRemove && existing != null) {
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { confirmRemove = false },
            title = { Text("Remove ${existing.name}?") },
            text = { Text("This phone forgets its address and password. The router itself is untouched.") },
            confirmButton = {
                TextButton(onClick = { app.remove(existing.id); confirmRemove = false; onRemoved() }) {
                    Text("Remove")
                }
            },
            dismissButton = { TextButton(onClick = { confirmRemove = false }) { Text("Cancel") } },
        )
    }
}

/**
 * Health first, which needs no password, so "wrong address" is told apart from "wrong
 * password" rather than both arriving as one vague failure.
 *
 * [baseUrl] is what the saved router would use, [Router.baseUrl], so a test that passes
 * here tests the address the router will really be reached at, scheme included.
 *
 * [hosted] is what the form already knows about the router being hosted. A router with no
 * password refuses `/api/auth` itself, so that refusal cannot say whether it is.
 */
private suspend fun testConnection(baseUrl: String, password: String?, hosted: Boolean): Probe {
    val client = RouterClient(baseUrl, password)
    val health = runCatching { client.get<Health>("/_router/health") }
        .getOrElse { return Probe(false, RouterState.describe(it)) }
    val auth = runCatching { client.get<AuthStatus>("/api/auth") }
        .getOrElse {
            return when (it) {
                is RouterFailure.Unauthorized ->
                    Probe(false, "Agent Router ${health.version} answered, but that password is wrong")
                is RouterFailure.NoPasswordSet -> Probe(
                    false,
                    "Agent Router ${health.version} answered, but it has no password set yet — " +
                        if (hosted) "a hosted router's password is set by its deployment" else "set one on its dashboard",
                )
                else -> Probe(false, RouterState.describe(it))
            }
        }
    // A hosted router reports `mode: "server"` and is always reached from off its machine;
    // only a router listening on its Mac's loopback alone fails here.
    if (!auth.reachableOffHost) {
        return Probe(false, "Agent Router ${health.version} answered, but it only listens on the Mac itself — turn on Listen on your network in its dashboard settings", auth)
    }
    return Probe(true, "Agent Router ${health.version} answered, and the password is right", auth)
}

/** What Test connection found: whether it passed, the sentence to show, and `/api/auth`'s reply when it got one. */
private class Probe(val ok: Boolean, val text: String, val auth: AuthStatus? = null)
