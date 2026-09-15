package ca.whnet.agentrouter.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.LargeTopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.VerticalDivider
import androidx.compose.material3.rememberTopAppBarState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.data.AppState
import ca.whnet.agentrouter.data.Router
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RoutersListScreen(
    app: AppState,
    onOpen: (Router) -> Unit,
    onAdd: () -> Unit,
    /** Rendered beside the list when the window is wide enough for two panes. */
    detail: (@Composable (Router) -> Unit)? = null,
) {
    val routers = app.routers
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val bar = TopAppBarDefaults.exitUntilCollapsedScrollBehavior(rememberTopAppBarState())

    // Every router at once, on the same cadence the iPhone list uses.
    LaunchedEffect(routers.map { it.id }) {
        while (true) {
            routers.forEach { app.state(it).refresh() }
            kotlinx.coroutines.delay(15_000)
        }
    }

    // On a wide window the list keeps its own selection and shows it alongside, rather
    // than navigating away and leaving half the screen empty.
    val wide = twoPane() && detail != null
    var selected by rememberSaveable { mutableStateOf<String?>(null) }
    val chosen = routers.firstOrNull { it.id == selected } ?: routers.firstOrNull()
    if (wide && chosen != null) {
        Row(Modifier.fillMaxSize()) {
            Box(Modifier.width(listPaneWidth()).fillMaxHeight()) {
                RouterListPane(app, routers, chosen.id, refreshing, { r -> selected = r.id }, onAdd) {
                    scope.launch {
                        refreshing = true
                        routers.forEach { app.state(it).refresh() }
                        refreshing = false
                    }
                }
            }
            VerticalDivider()
            Box(Modifier.weight(1f).fillMaxHeight()) { detail(chosen) }
        }
        return
    }

    Scaffold(
        modifier = Modifier.fillMaxSize().nestedScroll(bar.nestedScrollConnection),
        topBar = { LargeTopAppBar(title = { Text("Routers") }, scrollBehavior = bar) },
        floatingActionButton = {
            if (routers.isNotEmpty()) {
                FloatingActionButton(onClick = onAdd) { Icon(Icons.Default.Add, "Add a router") }
            }
        },
    ) { pad ->
        if (routers.isEmpty()) {
            Column(Modifier.fillMaxSize().padding(pad), verticalArrangement = Arrangement.Center) {
                EmptyState(
                    title = "No routers yet",
                    detail = "Add the Mac running Agent Router by its LAN address and password.",
                ) {
                    androidx.compose.material3.Button(onClick = onAdd) { Text("Add a router") }
                }
            }
            return@Scaffold
        }
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = {
                scope.launch {
                    refreshing = true
                    routers.forEach { app.state(it).refresh() }
                    refreshing = false
                }
            },
            modifier = Modifier.fillMaxSize().padding(pad),
        ) {
            LazyColumn(Modifier.fillMaxSize(), contentPadding = screenPadding()) {
                items(routers, key = { it.id }) { router ->
                    RouterRow(app, router, onOpen)
                    HorizontalDivider()
                }
            }
        }
    }
}

@Composable
private fun RouterRow(
    app: AppState,
    router: Router,
    onOpen: (Router) -> Unit,
    selected: Boolean = false,
) {
    val state = app.state(router)
    val status = when {
        state.failure != null -> "failed"
        state.isReachable -> "working"
        else -> null
    }
    Column(
        Modifier.fillMaxWidth()
            .background(if (selected) MaterialTheme.colorScheme.secondaryContainer else androidx.compose.ui.graphics.Color.Transparent)
            .clickable { onOpen(router) }
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            StateDot(status, size = 8)
            Text(router.name, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            Text(
                router.address,
                style = MaterialTheme.typography.labelSmall,
                fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val failure = state.failure
        val account = state.activeAccount
        when {
            failure != null -> Text(
                failure.message ?: "unreachable",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
                maxLines = 2,
            )
            account != null -> {
                Text(account.title, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    MiniGauge("5h", account.fiveHour, Modifier.weight(1f, fill = false))
                    MiniGauge("7d", account.sevenDay, Modifier.weight(1f, fill = false))
                }
                val n = state.activeSessions
                if (n > 0) {
                    Text(
                        "$n active session${if (n == 1) "" else "s"}",
                        style = MaterialTheme.typography.labelMedium,
                        color = Tint.green,
                    )
                }
            }
            else -> Text(
                if (state.isReachable) "connected" else "checking…",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/** A window's fill at a glance, small enough for a list row. */
@Composable
private fun MiniGauge(label: String, window: ca.whnet.agentrouter.api.Forecast.Window?, modifier: Modifier = Modifier) {
    val live = window != null && !window.isExpired
    val value = if (live) (window!!.used ?: 0.0).coerceIn(0.0, 1.0) else 0.0
    Column(modifier.widthIn(max = 140.dp), verticalArrangement = Arrangement.spacedBy(3.dp)) {
        Row(Modifier.fillMaxWidth()) {
            Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
            Text(if (live) Fmt.percent(value) else "—", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        LinearProgressIndicator(
            progress = { value.toFloat() },
            color = quotaTint(value, live),
            trackColor = MaterialTheme.colorScheme.surfaceVariant,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}


/** The list on its own, for the wide layout's left-hand pane. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RouterListPane(
    app: AppState,
    routers: List<Router>,
    selectedId: String?,
    refreshing: Boolean,
    onSelect: (Router) -> Unit,
    onAdd: () -> Unit,
    onRefresh: () -> Unit,
) {
    Scaffold(
        topBar = { TopAppBar(title = { Text("Routers") }) },
        floatingActionButton = {
            FloatingActionButton(onClick = onAdd) { Icon(Icons.Default.Add, "Add a router") }
        },
    ) { pad ->
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = onRefresh,
            modifier = Modifier.fillMaxSize().padding(pad),
        ) {
            LazyColumn(Modifier.fillMaxSize(), contentPadding = screenPadding()) {
                items(routers, key = { it.id }) { router ->
                    RouterRow(app, router, onSelect, selected = router.id == selectedId)
                    HorizontalDivider()
                }
            }
        }
    }
}
