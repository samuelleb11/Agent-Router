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
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.outlined.Computer
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
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
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.Saver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
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
    val snackbar = remember { SnackbarHostState() }
    var refreshing by remember { mutableStateOf(false) }
    var selectedId by rememberSaveable(router.id) { mutableStateOf<String?>(null) }

    LaunchedEffect(router.id) {
        // An empty list is worded for a hosted router when this one is. The router's page
        // has nearly always asked already; a notification opening the app does not pass it.
        // Beside the listing, not before it, so an unreachable router is not reported late.
        if (state.auth == null) launch { state.refreshAuth() }
        while (true) {
            state.refreshSessions()
            delay(5_000)
        }
    }

    val machines = SessionGroups.byMachine(state.sessions)
    val wide = twoPane() && detail != null

    // On a wide window the feed opens beside the list rather than replacing it, so
    // answering a prompt does not cost you your place.
    val open: (LiveSession) -> Unit =
        if (wide) { s -> selectedId = s.sessionId } else onOpen

    val list: @Composable () -> Unit = {
        Scaffold(
            snackbarHost = { SnackbarHost(snackbar) },
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
                SessionsList(machines, state, selectedId.takeIf { wide }, open) { s, archived ->
                    // Refused, the router's words, at the foot of the list wherever it is
                    // scrolled to: see PutAway.
                    scope.launch { PutAway.fromList(state, s, archived) { snackbar.showSnackbar(it) } }
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
    machines: List<SessionGroups.Machine>,
    state: RouterState,
    selectedId: String?,
    onOpen: (LiveSession) -> Unit,
    onArchive: (LiveSession, Boolean) -> Unit,
) {
    // The folds that are open, by name — "older" and "archived", each behind its machine's
    // when the list is split by machine, so that each machine's folds open on their own.
    var unfolded by rememberSaveable(stateSaver = FoldsSaver) { mutableStateOf(emptySet<String>()) }
    val toggle = { fold: String -> unfolded = if (fold in unfolded) unfolded - fold else unfolded + fold }
    // A hosted router's sessions from more than one machine: a heading for each. Otherwise
    // there is one entry and no heading, and the list is exactly what it always was.
    val split = machines.size > 1

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
                // "Nothing" and "not asked yet" are different claims; only make the first
                // once the router has actually answered.
                if (state.sessionsLoaded && state.isHosted) {
                    // A hosted router sees sessions only through the agent on each machine,
                    // so an empty list there says how a machine gets one.
                    EmptyState(
                        "No sessions from a connected machine",
                        "A machine shows its sessions here once it runs the Agent Router agent:",
                    ) {
                        SelectionContainer {
                            Text(
                                "agent-router agent install --server ${state.router.baseUrl}",
                                style = MaterialTheme.typography.bodySmall,
                                fontFamily = FontFamily.Monospace,
                                textAlign = TextAlign.Center,
                            )
                        }
                    }
                } else if (state.sessionsLoaded) {
                    EmptyState("No sessions this week", "The router lists transcripts written in the last seven days.")
                } else {
                    Row(Modifier.fillMaxWidth().padding(32.dp), horizontalArrangement = Arrangement.Center) {
                        CircularProgressIndicator()
                    }
                }
            }
        }

        machines.forEach { machine ->
            // Split, every key carries the machine's name: the same project can be on two
            // machines at once, and two items sharing a key is a crash, not a glitch.
            val at = if (split) "m:${machine.label}:" else ""
            if (split) item(key = "${at}machine") { MachineHeading(machine.label) }
            sections(machine.groups, at, unfolded, toggle, selectedId, onOpen, onArchive)
        }
    }
}

/**
 * One machine's sessions, or the whole list when it is not split: recent projects, then
 * the folds. [at] goes in front of every key and fold name, and is empty when the list is
 * not split, so those are then exactly the keys the list always had.
 */
private fun LazyListScope.sections(
    groups: SessionGroups,
    at: String,
    unfolded: Set<String>,
    onToggle: (String) -> Unit,
    selectedId: String?,
    onOpen: (LiveSession) -> Unit,
    onArchive: (LiveSession, Boolean) -> Unit,
) {
    projects(groups.recent, "${at}recent", selectedId, onOpen, onArchive)

    // Older and archived are folded away, as they are on the iPhone: what you almost
    // always want is what moved today.
    val olderCount = groups.older.sumOf { it.sessions.size }
    if (olderCount > 0) {
        val fold = "${at}older"
        item {
            Disclosure(
                "$olderCount older session${if (olderCount == 1) "" else "s"}",
                fold in unfolded,
            ) { onToggle(fold) }
        }
        if (fold in unfolded) projects(groups.older, fold, selectedId, onOpen, onArchive)
    }
    if (groups.archived.isNotEmpty()) {
        val fold = "${at}archived"
        item {
            Disclosure("${groups.archived.size} put away", fold in unfolded) { onToggle(fold) }
        }
        if (fold in unfolded) {
            items(groups.archived, key = { "${at}arch-${it.sessionId}" }) { s ->
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

/** The name over one machine's share of a hosted router's list. */
@Composable
private fun MachineHeading(label: String?) {
    Row(
        Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 20.dp, bottom = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Icon(
            Icons.Outlined.Computer, null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(18.dp),
        )
        Text(
            label ?: "Unnamed machine",
            style = MaterialTheme.typography.titleSmall,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** The open folds, kept across a rotation as a list, which a saved-state bundle can hold. */
private val FoldsSaver: Saver<Set<String>, ArrayList<String>> = Saver(
    save = { ArrayList(it) },
    restore = { it.toSet() },
)

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
                project.dot?.let { StateDot(it, size = 7) }
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
    val dismiss = rememberSwipeToDismissBoxState()
    val archive by rememberUpdatedState(onArchive)
    // Asked once, when the row has come to rest swiped away, and the row then goes back in
    // place, as the iPhone's swipe action does: it leaves the list when the router has put
    // the session away and the listing says so. Asked from `confirmValueChange`, as it was, a
    // swipe put it away up to three times over (mid-drag, on release, and as the row came to
    // rest), and a refusal would be said as often; and a row left swiped away, under its "Put
    // away" and taking no more swipes, read a refusal as done.
    LaunchedEffect(dismiss.settledValue) {
        if (dismiss.settledValue == SwipeToDismissBoxValue.EndToStart) {
            archive()
            dismiss.reset()
        }
    }
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
        SessionDot(session, size = 9)
        Column(Modifier.weight(1f)) {
            Text(
                session.label,
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            // What it is doing, in the router's words, and for how long — every session
            // (see StatusText.row); the one waiting on you in the colour this app uses for
            // "waiting for you".
            StatusText.row(session)?.let { line ->
                Text(
                    line,
                    style = MaterialTheme.typography.bodySmall,
                    color = if (session.needsYou) MaterialTheme.colorScheme.tertiary
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
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
