// Every session the router can see, grouped the way the web view groups them — and the
// way the iPhone app does, so three clients looking at one router agree about what is where.

package ca.whnet.agentrouter.ui

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.VerticalDivider
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.data.AppState
import ca.whnet.agentrouter.data.Router
import ca.whnet.agentrouter.data.RouterState
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
    /** Rendered beside the list when the window is wide enough for two panes. */
    detail: (@Composable (LiveSession) -> Unit)? = null,
) {
    val state = app.state(router)
    val scope = rememberCoroutineScope()
    var refreshing by remember { mutableStateOf(false) }
    var selectedId by remember(router.id) { mutableStateOf<String?>(null) }

    LaunchedEffect(router.id) {
        while (true) {
            state.refreshSessions()
            delay(5_000)
        }
    }

    val groups = SessionGroups.build(state.sessions)
    val wide = twoPane() && detail != null

    // On a wide window the feed opens beside the list rather than replacing it, so
    // answering a prompt does not cost you your place.
    val open: (LiveSession) -> Unit =
        if (wide) { s -> selectedId = s.sessionId } else onOpen

    val list: @Composable () -> Unit = {
        Scaffold(
            topBar = {
                TopAppBar(
                    title = { Text(router.name) },
                    navigationIcon = {
                        IconButton(onClick = onBack) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back")
                        }
                    },
                    actions = { TextButton(onClick = onSettings) { Text("Settings") } },
                )
            },
        ) { pad ->
            PullToRefreshBox(
                isRefreshing = refreshing,
                onRefresh = {
                    scope.launch { refreshing = true; state.refreshSessions(); refreshing = false }
                },
                modifier = Modifier.fillMaxSize().padding(pad),
            ) {
                SessionsList(groups, state, selectedId.takeIf { wide }, open) { s, archived ->
                    scope.launch { state.archive(s, archived) }
                }
            }
        }
    }

    if (!wide) {
        list()
        return
    }
    Row(Modifier.fillMaxSize()) {
        Box(Modifier.width(listPaneWidth()).fillMaxHeight()) { list() }
        VerticalDivider()
        Box(Modifier.weight(1f).fillMaxHeight()) {
            val chosen = state.sessions.firstOrNull { it.sessionId == selectedId }
            if (chosen != null) detail!!(chosen)
            else EmptyState(
                "Pick a session",
                "Its feed opens here, beside the list.",
                Modifier.fillMaxSize().wrapContentHeight(),
            )
        }
    }
}

@Composable
private fun SessionsList(
    groups: SessionGroups,
    state: RouterState,
    selectedId: String?,
    onOpen: (LiveSession) -> Unit,
    onArchive: (LiveSession, Boolean) -> Unit,
) {
    var showOlder by remember { mutableStateOf(false) }
    var showArchived by remember { mutableStateOf(false) }
    val olderCount = groups.older.sumOf { it.sessions.size }

    LazyColumn(Modifier.fillMaxSize(), contentPadding = screenPadding()) {
        // A session started by an unwrapped extension can be read but not answered, which
        // is worth saying before you tap into one and find the composer useless.
        state.shim?.newestUnwrapped?.let { t ->
            item {
                Row(
                    Modifier.fillMaxWidth().padding(16.dp),
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    Icon(
                        Icons.Default.Warning, null,
                        tint = MaterialTheme.colorScheme.tertiary,
                        modifier = Modifier.size(18.dp),
                    )
                    Text(
                        "${t.ext} is not wrapped — sessions it starts are view-only until the " +
                            "wrapper is installed, from Settings or the dashboard on the Mac.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.tertiary,
                    )
                }
                HorizontalDivider()
            }
        }
        state.sessionsFailure?.let { why ->
            item {
                Text(
                    why,
                    Modifier.fillMaxWidth().padding(16.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
                HorizontalDivider()
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

        projects(groups.recent, "recent", selectedId, onOpen, onArchive)

        // Older and archived are folded away, as they are on the iPhone: what you almost
        // always want is what moved today.
        if (olderCount > 0) {
            item {
                Disclosure(
                    "$olderCount older session${if (olderCount == 1) "" else "s"}",
                    showOlder,
                ) { showOlder = !showOlder }
            }
            if (showOlder) projects(groups.older, "older", selectedId, onOpen, onArchive)
        }
        if (groups.archived.isNotEmpty()) {
            item {
                Disclosure("${groups.archived.size} put away", showArchived) {
                    showArchived = !showArchived
                }
            }
            if (showArchived) {
                items(groups.archived, key = { "arch-${it.sessionId}" }) { s ->
                    SessionRow(
                        s, selected = s.sessionId == selectedId,
                        onClick = { onOpen(s) },
                        trailing = { TextButton(onClick = { onArchive(s, false) }) { Text("Restore") } },
                    )
                    HorizontalDivider()
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
private fun LazyListScope.projects(
    projects: List<SessionGroups.Project>,
    section: String,
    selectedId: String?,
    onOpen: (LiveSession) -> Unit,
    onArchive: (LiveSession, Boolean) -> Unit,
) {
    projects.forEach { project ->
        item(key = "$section-h-${project.name}") {
            Row(
                Modifier.fillMaxWidth()
                    .padding(start = 16.dp, end = 16.dp, top = 14.dp, bottom = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(7.dp),
            ) {
                Text(
                    project.name,
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.primary,
                )
                if (project.live) StateDot("working", size = 7, active = true)
            }
        }
        items(project.sessions, key = { "$section-${it.sessionId}" }) { s ->
            SwipeableSessionRow(s, s.sessionId == selectedId, { onOpen(s) }) { onArchive(s, true) }
            HorizontalDivider()
        }
    }
}

@Composable
private fun Disclosure(label: String, open: Boolean, onToggle: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onToggle)
            .padding(horizontal = 16.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            label,
            Modifier.weight(1f),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Icon(
            if (open) Icons.Default.KeyboardArrowUp else Icons.Default.KeyboardArrowDown,
            null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/**
 * Archiving is the reader's shelf, not a delete: the router un-shelves a session by itself
 * the moment it does something new.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SwipeableSessionRow(
    session: LiveSession,
    selected: Boolean,
    onClick: () -> Unit,
    onArchive: () -> Unit,
) {
    val dismiss = rememberSwipeToDismissBoxState(
        confirmValueChange = {
            if (it == SwipeToDismissBoxValue.EndToStart) { onArchive(); true } else false
        },
    )
    SwipeToDismissBox(
        state = dismiss,
        enableDismissFromStartToEnd = false,
        backgroundContent = {
            Box(
                Modifier.fillMaxSize().background(MaterialTheme.colorScheme.secondaryContainer)
                    .padding(horizontal = 20.dp),
                contentAlignment = Alignment.CenterEnd,
            ) {
                Text("Put away", color = MaterialTheme.colorScheme.onSecondaryContainer)
            }
        },
    ) {
        SessionRow(session, selected, onClick)
    }
}

@Composable
private fun SessionRow(
    session: LiveSession,
    selected: Boolean,
    onClick: () -> Unit,
    trailing: (@Composable () -> Unit)? = null,
) {
    Row(
        Modifier.fillMaxWidth()
            .background(
                if (selected) MaterialTheme.colorScheme.secondaryContainer
                else MaterialTheme.colorScheme.surface
            )
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 12.dp)
            .animateContentSize(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        StateDot(session.state, size = 9, active = session.active)
        Column(Modifier.weight(1f)) {
            Text(
                session.label,
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                Text(
                    session.shortId,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                session.gitBranch?.let {
                    Text("·", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.outline)
                    Text(
                        it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Text("·", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.outline)
                Text(
                    Fmt.ago(session.lastActivityMs),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                )
            }
        }
        if (trailing != null) trailing() else LiveBadge(session.wrapped)
    }
}

/** Whether a typed message would actually arrive, said in one word. */
@Composable
private fun LiveBadge(wrapped: Boolean) {
    val tint = if (wrapped) Color(0xFF34C759) else MaterialTheme.colorScheme.onSurfaceVariant
    Text(
        if (wrapped) "live" else "view",
        Modifier
            .clip(CircleShape)
            .background(tint.copy(alpha = 0.15f))
            .padding(horizontal = 8.dp, vertical = 2.dp),
        style = MaterialTheme.typography.labelSmall,
        color = tint,
    )
}
