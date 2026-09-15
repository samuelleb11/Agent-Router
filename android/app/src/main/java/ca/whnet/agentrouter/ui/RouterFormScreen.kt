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
    var name by remember { mutableStateOf(existing?.name ?: "") }
    var host by remember { mutableStateOf(existing?.host ?: "") }
    var port by remember { mutableStateOf((existing?.port ?: 8787).toString()) }
    var password by remember { mutableStateOf("") }
    var probe by remember { mutableStateOf<String?>(null) }
    var probeOk by remember { mutableStateOf(false) }
    var testing by remember { mutableStateOf(false) }
    var confirmRemove by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    val portNumber = port.toIntOrNull()
    val valid = name.isNotBlank() && host.isNotBlank() && portNumber in 1..65535 &&
        (existing != null || password.isNotEmpty())

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
                            val r = Router(
                                id = existing?.id ?: java.util.UUID.randomUUID().toString(),
                                name = name.trim(), host = host.trim(), port = portNumber!!,
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
            Modifier.fillMaxSize().padding(pad).readableWidth(640.dp).bottomInset().verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            SettingsSection(
                header = "Address",
                footer = "Use the Mac's LAN IP address, not its name. The router refuses a " +
                    "request from another device whose address is a hostname — that is how it " +
                    "guards against a web page in your browser reaching it.",
            ) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    OutlinedTextField(
                        value = name, onValueChange = { name = it },
                        label = { Text("Name") }, singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        OutlinedTextField(
                            value = host, onValueChange = { host = it; probe = null },
                            label = { Text("Host") }, singleLine = true,
                            placeholder = { Text("10.0.0.5") },
                            modifier = Modifier.weight(2f),
                        )
                        OutlinedTextField(
                            value = port, onValueChange = { port = it.filter(Char::isDigit); probe = null },
                            label = { Text("Port") }, singleLine = true,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                            modifier = Modifier.weight(1f),
                        )
                    }
                }
            }

            SettingsSection(
                header = "Password",
                footer = if (existing == null) {
                    "The router password, set on its dashboard on the Mac. It is kept in this " +
                        "phone's keystore, not in the file that holds the address."
                } else {
                    "Leave this blank to keep the password already stored."
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
                            enabled = !testing && host.isNotBlank() && portNumber != null,
                            onClick = {
                                testing = true
                                scope.launch {
                                    val pw = password.ifEmpty { existing?.let { app.store.password(it.id) } }
                                    val result = testConnection(host.trim(), portNumber!!, pw)
                                    probe = result.second
                                    probeOk = result.first
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
 */
private suspend fun testConnection(host: String, port: Int, password: String?): Pair<Boolean, String> {
    val client = RouterClient("http://$host:$port", password)
    val health = runCatching { client.get<Health>("/_router/health") }
        .getOrElse { return false to RouterState.describe(it) }
    val auth = runCatching { client.get<AuthStatus>("/api/auth") }
        .getOrElse {
            return when (it) {
                is RouterFailure.Unauthorized ->
                    false to "Agent Router ${health.version} answered, but that password is wrong"
                is RouterFailure.NoPasswordSet ->
                    false to "Agent Router ${health.version} answered, but it has no password set yet — set one on its dashboard"
                else -> false to RouterState.describe(it)
            }
        }
    if (!auth.remoteBind) {
        return false to "Agent Router ${health.version} answered, but it only listens on the Mac itself — restart it with AGENT_ROUTER_BIND=0.0.0.0"
    }
    return true to "Agent Router ${health.version} answered, and the password is right"
}
