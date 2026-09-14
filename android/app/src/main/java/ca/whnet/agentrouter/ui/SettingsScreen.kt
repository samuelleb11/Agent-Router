package ca.whnet.agentrouter.ui

import android.content.Intent
import android.provider.Settings
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
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.api.ConfigToggleBody
import ca.whnet.agentrouter.api.OkReply
import ca.whnet.agentrouter.api.PasswordBody
import ca.whnet.agentrouter.api.PushStatus
import ca.whnet.agentrouter.api.ServiceBody
import ca.whnet.agentrouter.api.ShimAutoBody
import ca.whnet.agentrouter.api.ShimBody
import ca.whnet.agentrouter.api.post
import ca.whnet.agentrouter.api.PushTestReply
import ca.whnet.agentrouter.api.get
import ca.whnet.agentrouter.api.postEmpty
import ca.whnet.agentrouter.data.AppState
import ca.whnet.agentrouter.data.Router
import ca.whnet.agentrouter.data.RouterState
import ca.whnet.agentrouter.push.PushRegistrar
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(app: AppState, router: Router, onBack: () -> Unit, onEdit: () -> Unit) {
    val context = LocalContext.current
    val state = app.state(router)
    val scope = rememberCoroutineScope()
    var push by remember { mutableStateOf<PushStatus?>(null) }
    var testResult by remember { mutableStateOf<String?>(null) }
    var registering by remember { mutableStateOf(false) }
    var newPassword by remember { mutableStateOf("") }
    var passwordNotice by remember { mutableStateOf<String?>(null) }
    var projectPath by remember { mutableStateOf("") }
    var routeNotice by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(router.id) {
        runCatching { state.liveClient?.get<PushStatus>("/api/push") }.onSuccess { push = it }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Settings") },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
                },
                actions = { TextButton(onClick = onEdit) { Text("Edit") } },
            )
        },
    ) { pad ->
        Column(Modifier.fillMaxSize().padding(pad).verticalScroll(rememberScrollState())) {

            SettingsSection(
                header = "Notifications",
                footer = "A permission or a question in any session under the wrapper is " +
                    "announced here, once; tapping it opens that session. Answer it anywhere " +
                    "else and the notification takes itself down.",
            ) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    Row(Modifier.fillMaxWidth()) {
                        Text("This phone", Modifier.weight(1f))
                        Text(
                            PushRegistrar.phoneStatus(context),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    if (!PushRegistrar.allowed(context)) {
                        OutlinedButton(onClick = {
                            context.startActivity(
                                Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                                    .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
                            )
                        }) { Text("Open notification settings") }
                    }
                    Row(Modifier.fillMaxWidth()) {
                        Text("With this router", Modifier.weight(1f))
                        Text(
                            PushRegistrar.results[router.id] ?: "not registered yet",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    OutlinedButton(
                        enabled = !registering && PushRegistrar.configured,
                        onClick = {
                            registering = true
                            scope.launch {
                                PushRegistrar.register(app.store, router, context)
                                registering = false
                            }
                        },
                    ) { Text(if (registering) "Registering…" else "Register again") }
                }
            }

            SettingsSection(
                header = "Battery",
                footer = "Samsung and several other makers stop background delivery for apps " +
                    "they decide are unused — which silently stops notifications arriving at " +
                    "all. Exclude Agent Router from battery optimisation to prevent that.",
            ) {
                Column(Modifier.padding(16.dp)) {
                    OutlinedButton(onClick = {
                        context.startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
                    }) { Text("Battery optimisation settings") }
                }
            }

            val setup = state.setup
            SettingsSection(
                header = "Claude Code on that Mac",
                footer = "When this is on, every project on the Mac sends its requests through " +
                    "the router, which is what makes them show up here at all.",
            ) {
                Column {
                    Row(Modifier.fillMaxWidth().padding(16.dp)) {
                        Column(Modifier.weight(1f)) {
                            Text("Route every project")
                            Text(
                                setup?.globalSettingsPath ?: "asking the router…",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = setup?.globalRouted == true,
                            enabled = setup != null,
                            onCheckedChange = { on ->
                                scope.launch {
                                    runCatching {
                                        state.liveClient?.post<ConfigToggleBody, OkReply>(
                                            "/api/setup/config", ConfigToggleBody(enable = on)
                                        )
                                    }
                                    state.refresh(includeAccounts = true)
                                }
                            },
                        )
                    }
                    HorizontalDivider()
                    Row(Modifier.fillMaxWidth().padding(16.dp)) {
                        Column(Modifier.weight(1f)) {
                            Text("Start at login")
                            Text(
                                setup?.service?.let {
                                    if (it.supported) "${it.platform} login item" else "not supported here"
                                } ?: "—",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = setup?.service?.registered == true,
                            enabled = setup?.service?.supported == true,
                            onCheckedChange = { on ->
                                scope.launch {
                                    runCatching {
                                        state.liveClient?.post<ServiceBody, OkReply>(
                                            "/api/setup/service", ServiceBody(on)
                                        )
                                    }
                                    state.refresh(includeAccounts = true)
                                }
                            },
                        )
                    }
                }
            }

            val shim = state.shim
            SettingsSection(
                header = "The wrapper",
                footer = "A session started by an unwrapped extension can be read here but not " +
                    "answered — a typed message or a stop has nowhere to arrive. Let the router " +
                    "keep the wrapper installed and it wraps each new extension version itself.",
            ) {
                Column {
                    Row(Modifier.fillMaxWidth().padding(16.dp)) {
                        Column(Modifier.weight(1f)) {
                            Text("Keep it installed")
                            Text(
                                shim?.let { "${it.installed} of ${it.targets.size} wrapped" }
                                    ?: "asking the router…",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = shim?.auto == true,
                            enabled = shim?.available == true,
                            onCheckedChange = { on ->
                                scope.launch {
                                    runCatching {
                                        state.liveClient?.post<ShimAutoBody, OkReply>(
                                            "/api/setup/shim/auto", ShimAutoBody(on)
                                        )
                                    }
                                    state.refresh(includeAccounts = true)
                                }
                            },
                        )
                    }
                    shim?.targets?.forEach { t ->
                        HorizontalDivider()
                        Row(
                            Modifier.fillMaxWidth().padding(16.dp),
                            verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
                        ) {
                            Column(Modifier.weight(1f)) {
                                Text(
                                    t.ext.substringAfterLast('.').ifEmpty { t.ext },
                                    style = MaterialTheme.typography.bodyMedium,
                                )
                                Text(
                                    t.state.replace('_', ' '),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = if (t.state == "installed")
                                        MaterialTheme.colorScheme.onSurfaceVariant
                                    else MaterialTheme.colorScheme.error,
                                )
                            }
                            TextButton(onClick = {
                                val action = if (t.state == "installed") "remove" else "install"
                                scope.launch {
                                    runCatching {
                                        state.liveClient?.post<ShimBody, OkReply>(
                                            "/api/setup/shim", ShimBody(t.path, action)
                                        )
                                    }
                                    state.refresh(includeAccounts = true)
                                }
                            }) { Text(if (t.state == "installed") "Remove" else "Install") }
                        }
                    }
                }
            }

            SettingsSection(
                header = "Per-project routing",
                footer = "Point one project at this router without routing everything. The path " +
                    "is a directory on the Mac; the router writes that project's own " +
                    "settings.json and keeps a backup of what was there.",
            ) {
                Column {
                    val perProject = setup?.managed?.filter { it.scope != "global" }.orEmpty()
                    perProject.forEach { m ->
                        // `.claude/settings.json` hangs off the project directory; the row
                        // names the project, not the file.
                        val dir = m.path.removeSuffix("/.claude/settings.json").removeSuffix("/settings.json")
                        Row(
                            Modifier.fillMaxWidth().padding(16.dp),
                            verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
                        ) {
                            Column(Modifier.weight(1f)) {
                                Text(dir.substringAfterLast('/'), style = MaterialTheme.typography.bodyMedium)
                                Text(
                                    dir,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                            TextButton(onClick = {
                                scope.launch {
                                    runCatching {
                                        state.liveClient?.post<ConfigToggleBody, OkReply>(
                                            "/api/setup/config", ConfigToggleBody(enable = false, project = dir)
                                        )
                                    }
                                    state.refresh(includeAccounts = true)
                                }
                            }) { Text("Stop") }
                        }
                        HorizontalDivider()
                    }
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedTextField(
                            value = projectPath,
                            onValueChange = { projectPath = it },
                            label = { Text("Route a single project") },
                            placeholder = { Text("/path/to/code/something") },
                            singleLine = true,
                            modifier = Modifier.fillMaxWidth(),
                        )
                        OutlinedButton(
                            enabled = projectPath.trim().startsWith("/"),
                            onClick = {
                                val dir = projectPath.trim()
                                scope.launch {
                                    routeNotice = runCatching {
                                        state.liveClient?.post<ConfigToggleBody, OkReply>(
                                            "/api/setup/config", ConfigToggleBody(enable = true, project = dir)
                                        )?.problem ?: "Routing ${dir.substringAfterLast('/')}."
                                    }.getOrElse { RouterState.describe(it) }
                                    projectPath = ""
                                    state.refresh(includeAccounts = true)
                                }
                            },
                        ) { Text("Route it") }
                        routeNotice?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
                    }
                }
            }

            SettingsSection(
                header = "Router password",
                footer = "At least eight characters. Changing it here logs out every other " +
                    "device until they are given the new one — including this phone, which " +
                    "you would then re-enter under Edit.",
            ) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    OutlinedTextField(
                        value = newPassword,
                        onValueChange = { newPassword = it },
                        label = { Text("New password") },
                        singleLine = true,
                        visualTransformation = PasswordVisualTransformation(),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedButton(
                        enabled = newPassword.length >= 8,
                        onClick = {
                            scope.launch {
                                passwordNotice = runCatching {
                                    state.liveClient?.post<PasswordBody, OkReply>(
                                        "/api/auth", PasswordBody(password = newPassword)
                                    )?.problem ?: "Changed. Update it under Edit on this phone too."
                                }.getOrElse { RouterState.describe(it) }
                                newPassword = ""
                            }
                        },
                    ) { Text("Change password") }
                    passwordNotice?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
                }
            }

            SettingsSection(
                header = "Push credentials on the router",
                footer = "Drop the Firebase service-account JSON into ~/.agent-router/ on the " +
                    "Mac and the router takes it in by itself. See " +
                    "docs/google-push-requirements.md.",
            ) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    val fcm = push?.fcm
                    Row(Modifier.fillMaxWidth()) {
                        Text("Firebase", Modifier.weight(1f))
                        Text(
                            when {
                                push == null -> "asking the router…"
                                fcm?.configured == true -> fcm.projectId ?: "set"
                                else -> "not set"
                            },
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    fcm?.keyFile?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
                    fcm?.lastError?.let {
                        Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                    }
                    push?.devices?.forEach { d ->
                        Text(
                            "${d.name ?: "a phone"} · …${d.tokenTail} · ${d.lastResult ?: d.platform}",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    if (push?.anyConfigured == true) {
                        OutlinedButton(onClick = {
                            scope.launch {
                                testResult = runCatching {
                                    val r = state.liveClient?.postEmpty<PushTestReply>("/api/push/test")
                                    r?.error ?: r?.results?.joinToString("\n") { line ->
                                        "…${line.device}: ${if (line.ok) "delivered" else line.error ?: "failed"}"
                                    } ?: "no answer"
                                }.getOrElse { RouterState.describe(it) }
                            }
                        }) { Text("Send test") }
                    }
                    testResult?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
                }
            }
        }
    }
}
