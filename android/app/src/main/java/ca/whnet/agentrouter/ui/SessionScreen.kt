package ca.whnet.agentrouter.ui

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.Saver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.api.LivePrompt
import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.data.AppState
import ca.whnet.agentrouter.data.Router
import ca.whnet.agentrouter.feed.LiveFeed
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SessionScreen(app: AppState, router: Router, opened: LiveSession, onBack: (() -> Unit)?) {
    val state = app.state(router)
    // Switching between sessions in the same project happens in place, so which session
    // is open is state here rather than a navigation argument.
    var session by rememberSaveable(opened.sessionId, stateSaver = LiveSessionSaver) {
        mutableStateOf(opened)
    }
    // From the view model, so a rotation does not drop the stream and replay the backfill.
    val feed = app.feed(router, session)
    val scope = rememberCoroutineScope()
    val listState = rememberLazyListState()
    val snackbar = remember { SnackbarHostState() }
    // Survives rotation: a half-typed message is the last thing that should be lost.
    var composing by rememberSaveable(session.sessionId) { mutableStateOf("") }
    var menu by remember { mutableStateOf(false) }
    var asking by remember { mutableStateOf(false) }
    var asideText by remember { mutableStateOf("") }
    var asideAnswer by remember { mutableStateOf<String?>(null) }
    var asideRunning by remember { mutableStateOf(false) }
    var confirmBypass by remember { mutableStateOf(false) }
    val clipboard = LocalClipboardManager.current

    // Following is a state, not a per-append test: once the reader has scrolled up to
    // read something, new rows must not yank them back down. Only a scroll that has
    // *settled* changes it — the backfill arrives as a burst that grows the content under
    // a still offset, and judging "at the bottom" during that growth stops the feed
    // following before the first pin ever happens. That bug was found on iOS; the fix is
    // the same shape here.
    var following by remember(session.sessionId) { mutableStateOf(true) }
    LaunchedEffect(listState.isScrollInProgress) {
        if (listState.isScrollInProgress) return@LaunchedEffect
        val info = listState.layoutInfo
        val last = info.visibleItemsInfo.lastOrNull()
        following = last == null || last.index >= info.totalItemsCount - 2
    }

    // A session restored across a rotation, or opened from a notification, carries only
    // its id — so its title reads as the short id until the router's listing catches up.
    LaunchedEffect(state.sessions, session.sessionId) {
        if (session.title == null) {
            state.sessions.firstOrNull { it.sessionId == session.sessionId }
                ?.let { session = it }
        }
    }

    val siblings = state.sessions.filter { it.projectName == session.projectName }

    LaunchedEffect(feed.rows.size) {
        if (feed.rows.isNotEmpty() && following) listState.scrollToItem(feed.rows.size - 1)
    }

    // A permission request is a control frame between the CLI and its editor, never a
    // transcript entry, so it cannot arrive on the stream the feed is built from.
    LaunchedEffect(session.sessionId) {
        while (true) {
            feed.loadPrompt()
            delay(2000)
        }
    }

    LaunchedEffect(feed.notice) {
        feed.notice?.let { snackbar.showSnackbar(it.text) }
    }

    Scaffold(
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(session.label, style = MaterialTheme.typography.titleMedium, maxLines = 1)
                        val s = feed.status
                        Text(
                            when {
                                s == null && !feed.connected -> "connecting…"
                                s == null -> session.projectName
                                else -> s.detail.ifEmpty { s.state }
                            },
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                },
                navigationIcon = {
                    if (onBack != null) {
                        IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
                    }
                },
                actions = {
                    if (feed.status?.isBusy == true && session.wrapped) {
                        TextButton(enabled = !feed.stopping, onClick = { scope.launch { feed.interrupt() } }) {
                            Text("Stop")
                        }
                    }
                    StateDot(feed.status?.state ?: session.state)
                    IconButton(onClick = { menu = true }) { Icon(Icons.Default.MoreVert, "More") }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        feed.stats?.let { st ->
                            DropdownMenuItem(
                                text = {
                                    Text(
                                        "${st.model ?: "model unknown"} · ${Fmt.compact(st.total)} tokens · ${Fmt.usd(st.cost)}",
                                        style = MaterialTheme.typography.labelMedium,
                                    )
                                },
                                onClick = {},
                                enabled = false,
                            )
                        }
                        feed.project?.let { pr ->
                            DropdownMenuItem(
                                text = {
                                    Text(
                                        "project: ${pr.sessions} sessions · ${Fmt.usd(pr.cost)}",
                                        style = MaterialTheme.typography.labelMedium,
                                    )
                                },
                                onClick = {},
                                enabled = false,
                            )
                        }
                        HorizontalDivider()

                        // Switching model or effort needs the wrapper: both are control
                        // frames on the CLI's stdin, which only the wrapper holds.
                        if (session.wrapped) {
                            feed.options?.models?.forEach { m ->
                                DropdownMenuItem(
                                    text = { Text(m) },
                                    trailingIcon = {
                                        if (feed.stats?.model == m) Icon(Icons.Default.Check, "current")
                                    },
                                    onClick = { menu = false; scope.launch { feed.setModel(m) } },
                                )
                            }
                            if (feed.options?.efforts?.isNotEmpty() == true) HorizontalDivider()
                            feed.options?.efforts?.forEach { e ->
                                DropdownMenuItem(
                                    text = { Text("effort: $e") },
                                    trailingIcon = {
                                        if (feed.stats?.effort == e) Icon(Icons.Default.Check, "current")
                                    },
                                    onClick = { menu = false; scope.launch { feed.setEffort(e) } },
                                )
                            }
                            if (feed.options?.modes?.isNotEmpty() == true) HorizontalDivider()
                            feed.options?.modes?.forEach { m ->
                                DropdownMenuItem(
                                    text = { Text("mode: ${'$'}{m.label}") },
                                    trailingIcon = {
                                        if (feed.stats?.mode == m.value) Icon(Icons.Default.Check, "current")
                                    },
                                    onClick = {
                                        menu = false
                                        // One tap would otherwise stop this session asking
                                        // about anything, on a Mac nobody may be sitting at.
                                        if (m.isBypass) confirmBypass = true
                                        else scope.launch { feed.setMode(m.value) }
                                    },
                                )
                            }
                            HorizontalDivider()
                        }
                        DropdownMenuItem(
                            text = { Text("Ask on the side") },
                            onClick = { menu = false; asking = true },
                        )
                        DropdownMenuItem(
                            text = { Text("Put away") },
                            onClick = {
                                menu = false
                                // In a pane there is nothing to go back to; the list beside
                                // it simply stops showing the session as current.
                                scope.launch { state.archive(session, true); onBack?.invoke() }
                            },
                        )
                        DropdownMenuItem(
                            text = { Text("Copy session id") },
                            onClick = {
                                menu = false
                                clipboard.setText(AnnotatedString(session.sessionId))
                            },
                        )
                    }
                },
            )
        },
        bottomBar = {
            Surface(tonalElevation = 3.dp) {
              Column(Modifier.fillMaxWidth()) {
                feed.pendingPrompt?.let { p -> PermissionCard(p) { allow, suggestion, said ->
                    scope.launch { feed.answerPermission(allow, suggestion, said) }
                } }
                Row(
                    // imePadding for the keyboard, bottomBarInsets for the gesture pill or
                    // the three-button bar underneath it.
                    Modifier.fillMaxWidth().imePadding().bottomBarInsets().padding(10.dp),
                    verticalAlignment = Alignment.Bottom,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    OutlinedTextField(
                        value = composing,
                        onValueChange = { composing = it },
                        placeholder = { Text(if (session.wrapped) "Message" else "Message (not wrapped)") },
                        modifier = Modifier.weight(1f),
                        maxLines = 5,
                    )
                    IconButton(
                        enabled = composing.isNotBlank() && !feed.sending,
                        onClick = {
                            val text = composing
                            composing = ""
                            scope.launch { if (!feed.send(text)) composing = text }
                        },
                    ) { Icon(Icons.AutoMirrored.Filled.Send, "Send") }
                }
              }
            }
        },
    ) { pad ->
        Column(Modifier.fillMaxSize().padding(pad)) {
        if (siblings.size > 1) {
            Row(
                Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())
                    .padding(horizontal = 10.dp, vertical = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                siblings.forEach { s ->
                    val on = s.sessionId == session.sessionId
                    AssistChip(
                        onClick = { if (!on) { composing = ""; session = s } },
                        label = { Text(s.label, maxLines = 1, style = MaterialTheme.typography.labelMedium) },
                        leadingIcon = { StateDot(s.state, size = 8) },
                        colors = AssistChipDefaults.assistChipColors(
                            containerColor = if (on) MaterialTheme.colorScheme.secondaryContainer
                            else MaterialTheme.colorScheme.surface,
                        ),
                    )
                }
            }
            HorizontalDivider()
        }
        feed.stats?.let { st ->
            Row(
                Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())
                    .padding(horizontal = 14.dp, vertical = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                FactChip("model", st.model ?: "—")
                st.effort?.let { FactChip("effort", it) }
                st.mode?.let { value ->
                    FactChip("mode", feed.options?.modes?.firstOrNull { it.value == value }?.label ?: value)
                }
                FactChip("tokens", Fmt.compact(st.total))
                FactChip("turns", st.turns.toString())
                FactChip("cost", Fmt.usd(st.cost))
                feed.project?.let { FactChip("project", "${it.sessions} sessions · ${Fmt.usd(it.cost)}") }
            }
            HorizontalDivider()
        }
        LazyColumn(
            Modifier.fillMaxSize(),
            state = listState,
            contentPadding = screenPadding(),
        ) {
            items(feed.rows, key = { it.id }) { row ->
                FeedRow(row) { answers, parts ->
                    scope.launch { feed.answer(answers, parts, row.id) }
                }
            }
        }
        }
    }

    if (confirmBypass) {
        AlertDialog(
            onDismissRequest = { confirmBypass = false },
            title = { Text("Bypass permissions?") },
            text = { Text("This session will stop asking before it runs commands or edits files.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmBypass = false
                    scope.launch { feed.setMode("bypassPermissions") }
                }) { Text("Bypass") }
            },
            dismissButton = { TextButton(onClick = { confirmBypass = false }) { Text("Cancel") } },
        )
    }

    if (asking) {
        AlertDialog(
            onDismissRequest = { if (!asideRunning) { asking = false; asideAnswer = null } },
            title = { Text("Ask on the side") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    Text(
                        "Asks a forked copy of this session, with no tools. The session itself " +
                            "is not disturbed and never sees the question.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    OutlinedTextField(
                        value = asideText,
                        onValueChange = { asideText = it },
                        enabled = !asideRunning,
                        label = { Text("Question") },
                        modifier = Modifier.fillMaxWidth(),
                        maxLines = 4,
                    )
                    if (asideRunning) {
                        Row(
                            Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            CircularProgressIndicator(Modifier.padding(2.dp), strokeWidth = 2.dp)
                            Text("Forking the session — this can take a minute.",
                                style = MaterialTheme.typography.bodySmall)
                        }
                    }
                    asideAnswer?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
                }
            },
            confirmButton = {
                TextButton(
                    enabled = asideText.isNotBlank() && !asideRunning,
                    onClick = {
                        asideRunning = true
                        asideAnswer = null
                        scope.launch {
                            val reply = feed.aside(asideText)
                            asideAnswer = reply?.answer ?: reply?.reason ?: "No answer came back."
                            asideRunning = false
                        }
                    },
                ) { Text("Ask") }
            },
            dismissButton = {
                TextButton(enabled = !asideRunning, onClick = { asking = false; asideAnswer = null }) {
                    Text("Close")
                }
            },
        )
    }
}


/**
 * Ask a forked copy of the session something without disturbing it.
 *
 * The router runs `claude --resume --fork-session` with no tools, which can take a while —
 * up to three minutes before it gives up — so this says so rather than looking hung.
 */


/** One fact from the session's stats strip. */
@Composable
private fun FactChip(label: String, value: String) {
    Column {
        Text(value, style = MaterialTheme.typography.labelLarge)
        Text(
            label,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}


/**
 * Which session is open, kept across a rotation. Only the id is worth saving — the row
 * itself is re-read from the router's listing a second later.
 */
private val LiveSessionSaver: Saver<LiveSession, String> = Saver(
    save = { it.sessionId },
    restore = { LiveSession(sessionId = it, wrapped = true) },
)

/**
 * What the session is blocked on, and the ways out of it.
 *
 * The offers come from the prompt itself — the router refuses an index it did not offer,
 * and the editor on the far end drops anything it was not asked to suggest.
 */
@Composable
private fun PermissionCard(p: LivePrompt, answer: (Boolean, Int?, String) -> Unit) {
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 10.dp).padding(top = 10.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Text(
            "${p.displayName ?: p.tool ?: "A tool"} is waiting for permission",
            style = MaterialTheme.typography.titleSmall,
        )
        p.description?.takeIf { it.isNotBlank() }?.let {
            Text(it, style = MaterialTheme.typography.bodySmall)
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = { answer(true, null, "Allowed.") }) { Text("Allow") }
            OutlinedButton(onClick = { answer(false, null, "Declined.") }) { Text("Deny") }
        }
        p.suggestions.forEach { s ->
            TextButton(onClick = { answer(true, s.index, s.label + ".") }) { Text(s.label) }
        }
    }
}
