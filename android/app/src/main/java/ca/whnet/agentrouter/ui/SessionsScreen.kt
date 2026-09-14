package ca.whnet.agentrouter.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.data.AppState
import ca.whnet.agentrouter.data.Router
import ca.whnet.agentrouter.data.SessionGroups
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SessionsScreen(
    app: AppState,
    router: Router,
    onBack: () -> Unit,
    onOpen: (LiveSession) -> Unit,
    onSettings: () -> Unit,
) {
    val state = app.state(router)
    var showArchived by remember { mutableStateOf(false) }
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(router.id) {
        while (true) {
            state.refreshSessions()
            delay(5_000)
        }
    }

    val groups = SessionGroups.build(state.sessions)

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(router.name) },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
                },
                actions = { TextButton(onClick = onSettings) { Text("Settings") } },
            )
        },
    ) { pad ->
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = { scope.launch { refreshing = true; state.refreshSessions(); refreshing = false } },
            modifier = Modifier.fillMaxSize().padding(pad),
        ) {
        LazyColumn(Modifier.fillMaxSize(), contentPadding = ScreenPadding) {
            state.sessionsFailure?.let { why ->
                item {
                    SettingsSection(footer = "The list below is whatever was last seen.") {
                        Text(
                            why,
                            Modifier.padding(16.dp),
                            color = MaterialTheme.colorScheme.error,
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                }
            }
            if (state.sessions.isEmpty() && state.sessionsFailure == null) {
                item {
                    EmptyState(
                        "No sessions this week",
                        "The router lists transcripts written in the last seven days.",
                    )
                }
            }

            groupSection("recent", groups.recent, onOpen) { s ->
                scope.launch { state.archive(s, true) }
            }
            if (groups.older.isNotEmpty()) {
                item {
                    Text(
                        "EARLIER",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(start = 20.dp, top = 16.dp, bottom = 4.dp),
                    )
                }
                groupSection("older", groups.older, onOpen) { s ->
                    scope.launch { state.archive(s, true) }
                }
            }
            if (groups.archived.isNotEmpty()) {
                item {
                    TextButton(
                        onClick = { showArchived = !showArchived },
                        modifier = Modifier.padding(start = 12.dp, top = 8.dp),
                    ) {
                        Text(
                            if (showArchived) "Hide put away (${groups.archived.size})"
                            else "Put away (${groups.archived.size})"
                        )
                    }
                }
                if (showArchived) {
                    items(groups.archived, key = { it.sessionId }) { s ->
                        SessionRow(s, onOpen) { scope.launch { state.archive(s, false) } }
                        HorizontalDivider()
                    }
                }
            }
        }
        }
    }
}

/**
 * [section] is part of every key because a project can appear in more than one section at
 * once — one session touched an hour ago and another last week is the ordinary case — and
 * two list items sharing a key is a hard crash in Compose, not a visual glitch.
 */
private fun androidx.compose.foundation.lazy.LazyListScope.groupSection(
    section: String,
    projects: List<SessionGroups.Project>,
    onOpen: (LiveSession) -> Unit,
    onArchive: (LiveSession) -> Unit,
) {
    projects.forEach { project ->
        item(key = "$section-h-${project.name}") {
            Row(
                Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, top = 14.dp, bottom = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    project.name,
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.primary,
                )
                if (project.live) StateDot("working", size = 8)
            }
        }
        items(project.sessions, key = { it.sessionId }) { s ->
            ArchivableRow(s, onOpen, onArchive)
            HorizontalDivider()
        }
    }
}

@Composable
private fun SessionRow(
    session: LiveSession,
    onOpen: (LiveSession) -> Unit,
    onRestore: (() -> Unit)?,
) {
    Row(
        Modifier.fillMaxWidth().clickable { onOpen(session) }.padding(16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        StateDot(session.state)
        Column(Modifier.weight(1f)) {
            Text(session.label, style = MaterialTheme.typography.bodyLarge, maxLines = 2)
            val parts = buildList {
                add(Fmt.ago(session.lastActivityMs))
                if (session.state == "ask") add("waiting for you")
                if (!session.wrapped) add("not wrapped")
                session.gitBranch?.let { add(it) }
            }
            Text(
                parts.joinToString(" · "),
                style = MaterialTheme.typography.bodySmall,
                color = if (session.state == "ask") MaterialTheme.colorScheme.tertiary
                else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (onRestore != null) TextButton(onClick = onRestore) { Text("Restore") }
    }
}


/**
 * A session that can be swiped away. Archiving is the reader's shelf, not a delete: the
 * router un-shelves it by itself the moment the session does something new.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ArchivableRow(
    session: LiveSession,
    onOpen: (LiveSession) -> Unit,
    onArchive: (LiveSession) -> Unit,
) {
    val dismiss = rememberSwipeToDismissBoxState(
        confirmValueChange = {
            if (it == SwipeToDismissBoxValue.EndToStart) { onArchive(session); true } else false
        },
    )
    SwipeToDismissBox(
        state = dismiss,
        enableDismissFromStartToEnd = false,
        backgroundContent = {
            Box(
                Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.secondaryContainer)
                    .padding(20.dp),
                contentAlignment = Alignment.CenterEnd,
            ) {
                Text("Put away", color = MaterialTheme.colorScheme.onSecondaryContainer)
            }
        },
    ) {
        Box(Modifier.background(MaterialTheme.colorScheme.surface)) {
            SessionRow(session, onOpen, null)
        }
    }
}
