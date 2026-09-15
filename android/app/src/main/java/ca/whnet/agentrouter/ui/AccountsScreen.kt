package ca.whnet.agentrouter.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.FilterChip
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.Button
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.api.Account
import ca.whnet.agentrouter.api.AccountEditBody
import ca.whnet.agentrouter.api.AccountRefBody
import ca.whnet.agentrouter.api.AddAccountBody
import ca.whnet.agentrouter.api.LoginStartBody
import ca.whnet.agentrouter.api.LoginStatus
import ca.whnet.agentrouter.api.UsageTokenBody
import ca.whnet.agentrouter.api.get
import ca.whnet.agentrouter.api.OkReply
import ca.whnet.agentrouter.api.post
import ca.whnet.agentrouter.data.AppState
import ca.whnet.agentrouter.data.Router
import ca.whnet.agentrouter.data.RouterState
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AccountsScreen(app: AppState, router: Router, onBack: () -> Unit) {
    val state = app.state(router)
    val scope = rememberCoroutineScope()
    var editing by remember { mutableStateOf<Account?>(null) }
    var removing by remember { mutableStateOf<Account?>(null) }
    var notice by remember { mutableStateOf<String?>(null) }
    var adding by remember { mutableStateOf(false) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Accounts") },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
                },
            )
        },
    ) { pad ->
        Column(Modifier.fillMaxSize().padding(pad).readableWidth(760.dp).verticalScroll(rememberScrollState())) {
            notice?.let {
                SettingsSection { Text(it, Modifier.padding(16.dp), style = MaterialTheme.typography.bodyMedium) }
            }

            SettingsSection(
                header = "Configured",
                footer = "Tap an account to make it the one that serves. A switch takes effect " +
                    "on the next request the router proxies.",
            ) {
                Column {
                    if (state.accounts.isEmpty()) {
                        Text(
                            "The router has no accounts, or has not been asked yet.",
                            Modifier.padding(16.dp),
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    state.accounts.forEachIndexed { i, acc ->
                        if (i > 0) HorizontalDivider()
                        Row(
                            Modifier.fillMaxWidth()
                                .clickable(enabled = state.switching == null && !acc.isActive) {
                                    scope.launch { state.activate(acc.id) }
                                }
                                .padding(16.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            StateDot(
                                when {
                                    acc.isActive -> "working"
                                    acc.ready == false -> "failed"
                                    else -> null
                                }
                            )
                            Column(Modifier.weight(1f)) {
                                Text(acc.title, style = MaterialTheme.typography.bodyLarge)
                                val bits = buildList {
                                    acc.subscriptionType?.let { add(it) }
                                    acc.orgName?.let { add(it) }
                                    acc.detail?.let { add(it) }
                                    if (acc.hasUsageKey) add("usage key set")
                                }
                                if (bits.isNotEmpty()) {
                                    Text(
                                        bits.joinToString(" · "),
                                        style = MaterialTheme.typography.bodySmall,
                                        color = if (acc.ready == false) MaterialTheme.colorScheme.error
                                        else MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                            }
                            if (state.switching == acc.id) {
                                CircularProgressIndicator(Modifier.padding(4.dp), strokeWidth = 2.dp)
                            } else {
                                TextButton(onClick = { editing = acc }) { Text("Edit") }
                            }
                        }
                    }
                }
            }

            SettingsSection(
                footer = "Signing in opens Anthropic's page in this phone's browser; the router " +
                    "runs the CLI that waits for it and keeps the token.",
            ) {
                Column(Modifier.padding(16.dp)) {
                    Button(onClick = { adding = true }, modifier = Modifier.fillMaxWidth()) {
                        Text("Add an account")
                    }
                }
            }
        }
    }

    if (adding) {
        AddAccountDialog(app, router) { result ->
            adding = false
            if (result != null) notice = result
            scope.launch { state.refresh(includeAccounts = true) }
        }
    }

    editing?.let { acc ->
        var label by remember(acc.id) { mutableStateOf(acc.label) }
        var email by remember(acc.id) { mutableStateOf(acc.email.orEmpty()) }
        var token by remember(acc.id) { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { editing = null },
            title = { Text("Edit ${acc.title}") },
            text = {
                Column(
                    Modifier.verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    OutlinedTextField(
                        value = label,
                        onValueChange = { label = it },
                        label = { Text("Label") },
                        singleLine = true,
                    )
                    OutlinedTextField(
                        value = email,
                        onValueChange = { email = it },
                        label = { Text("Who it is") },
                        placeholder = { Text("name@example.com") },
                        singleLine = true,
                    )
                    OutlinedTextField(
                        value = token,
                        onValueChange = { token = it },
                        label = { Text("Rotate credential") },
                        placeholder = { Text("a fresh setup-token value") },
                        singleLine = true,
                    )
                    Text(
                        "A fresh `claude setup-token` value, if the current one was revoked or " +
                            "you want to rotate it. Leave blank to keep the one stored.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    OutlinedTextFieldForUsageKey(app, router, acc) { notice = it }
                    if (!acc.isActive) {
                        TextButton(onClick = { editing = null; removing = acc }) {
                            Text("Remove this account", color = MaterialTheme.colorScheme.error)
                        }
                    } else {
                        // The router refuses it anyway; saying so beats a failed call.
                        Text(
                            "The serving account cannot be removed. Switch to another first.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    val client = state.liveClient
                    editing = null
                    scope.launch {
                        notice = runCatching {
                            client?.post<AccountEditBody, OkReply>(
                                "/api/accounts/edit",
                                AccountEditBody(
                                    id = acc.id,
                                    label = label.trim(),
                                    email = email.trim().ifEmpty { null },
                                    token = token.trim().ifEmpty { null },
                                ),
                            )?.problem ?: if (token.isNotBlank()) "Saved, credential rotated." else "Saved."
                        }.getOrElse { RouterState.describe(it) }
                        state.refresh(includeAccounts = true)
                    }
                }) { Text("Save") }
            },
            dismissButton = { TextButton(onClick = { editing = null }) { Text("Cancel") } },
        )
    }

    removing?.let { acc ->
        AlertDialog(
            onDismissRequest = { removing = null },
            title = { Text("Remove ${acc.title}?") },
            text = { Text("The router forgets this account and its stored credential.") },
            confirmButton = {
                TextButton(onClick = {
                    val client = state.liveClient
                    removing = null
                    scope.launch {
                        notice = runCatching {
                            client?.post<AccountRefBody, OkReply>(
                                "/api/accounts/remove", AccountRefBody(acc.id)
                            )?.problem ?: "Removed."
                        }.getOrElse { RouterState.describe(it) }
                        state.refresh(includeAccounts = true)
                    }
                }) { Text("Remove", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { removing = null }) { Text("Cancel") } },
        )
    }
}


/**
 * Adding an account, three ways, as the dashboard offers them: sign in through the router,
 * paste a token, or point at a profile directory the CLI already logged into.
 */
@Composable
private fun AddAccountDialog(app: AppState, router: Router, onDone: (String?) -> Unit) {
    val state = app.state(router)
    val scope = rememberCoroutineScope()
    val clipboard = LocalClipboardManager.current
    var source by remember { mutableStateOf("login") }
    var label by remember { mutableStateOf("") }
    var token by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var login by remember { mutableStateOf<LoginStatus?>(null) }

    // While a sign-in is running the router drives the CLI; this just watches it.
    LaunchedEffect(busy, source) {
        if (!busy || source != "login") return@LaunchedEffect
        while (busy) {
            runCatching { state.liveClient?.get<LoginStatus>("/api/login/status") }
                .onSuccess { s ->
                    login = s
                    if (s?.phase == "done" || s?.phase == "failed") busy = false
                }
            delay(1500)
        }
    }

    AlertDialog(
        onDismissRequest = { if (!busy) onDone(null) },
        title = { Text("Add an account") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    listOf("login" to "Sign in", "token" to "Token", "profile" to "Profile")
                        .forEach { (key, name) ->
                            FilterChip(
                                selected = source == key,
                                enabled = !busy,
                                onClick = { source = key },
                                label = { Text(name) },
                            )
                        }
                }
                OutlinedTextField(
                    value = label,
                    onValueChange = { label = it },
                    enabled = !busy,
                    label = { Text("Label") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                when (source) {
                    "token" -> OutlinedTextField(
                        value = token,
                        onValueChange = { token = it },
                        enabled = !busy,
                        label = { Text("sk-ant-… token") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    "profile" -> OutlinedTextField(
                        value = token,
                        onValueChange = { token = it },
                        enabled = !busy,
                        label = { Text("Profile directory on the Mac") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    else -> {
                        val l = login
                        Text(
                            when (l?.phase) {
                                null -> "The router runs `claude setup-token` and gives you a link."
                                "starting" -> "Starting the CLI…"
                                "awaiting_sign_in" -> "Open the link, sign in, and this will finish itself."
                                "done" -> "Signed in as ${l.email ?: l.label ?: "the new account"}."
                                "failed" -> l.error ?: "The sign-in failed."
                                else -> l.phase
                            },
                            style = MaterialTheme.typography.bodySmall,
                            color = if (l?.phase == "failed") MaterialTheme.colorScheme.error
                            else MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        l?.url?.let { url ->
                            TextButton(onClick = { clipboard.setText(AnnotatedString(url)) }) {
                                Text("Copy the sign-in link")
                            }
                        }
                        l?.tail?.let {
                            Text(
                                it.takeLast(200),
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.outline,
                            )
                        }
                    }
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = !busy && label.isNotBlank() &&
                    (source == "login" || token.isNotBlank()),
                onClick = {
                    busy = true
                    scope.launch {
                        val client = state.liveClient
                        val problem = runCatching {
                            if (source == "login") {
                                client?.post<LoginStartBody, OkReply>(
                                    "/api/login/start", LoginStartBody(label.trim())
                                )?.problem
                            } else {
                                val r = client?.post<AddAccountBody, OkReply>(
                                    "/api/accounts",
                                    AddAccountBody(label.trim(), source, token.trim()),
                                )
                                busy = false
                                r?.problem
                            }
                        }.getOrElse { busy = false; RouterState.describe(it) }
                        if (source != "login") onDone(problem ?: "Added.")
                        else if (problem != null) { busy = false; onDone(problem) }
                    }
                },
            ) { Text(if (source == "login") "Start sign-in" else "Add") }
        },
        dismissButton = {
            TextButton(onClick = { onDone(null) }) { Text(if (busy) "Leave it running" else "Cancel") }
        },
    )
}

/** A claude.ai session key, which the router uses only to read quota for this account. */
@Composable
private fun OutlinedTextFieldForUsageKey(
    app: AppState,
    router: Router,
    account: ca.whnet.agentrouter.api.Account,
    onNotice: (String) -> Unit,
) {
    val state = app.state(router)
    val scope = rememberCoroutineScope()
    var key by remember(account.id) { mutableStateOf("") }
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        OutlinedTextField(
            value = key,
            onValueChange = { key = it },
            label = { Text(if (account.hasUsageKey) "Replace usage key" else "claude.ai usage key") },
            singleLine = true,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            TextButton(enabled = key.isNotBlank(), onClick = {
                scope.launch {
                    onNotice(
                        runCatching {
                            state.liveClient?.post<UsageTokenBody, OkReply>(
                                "/api/usage-token", UsageTokenBody(account.id, key.trim())
                            )?.problem ?: "Usage key saved."
                        }.getOrElse { RouterState.describe(it) }
                    )
                    key = ""
                }
            }) { Text("Save key") }
            if (account.hasUsageKey) {
                TextButton(onClick = {
                    scope.launch {
                        onNotice(
                            runCatching {
                                state.liveClient?.post<UsageTokenBody, OkReply>(
                                    "/api/usage-token", UsageTokenBody(account.id, null, clear = true)
                                )?.problem ?: "Usage key removed."
                            }.getOrElse { RouterState.describe(it) }
                        )
                    }
                }) { Text("Remove key") }
            }
        }
    }
}
