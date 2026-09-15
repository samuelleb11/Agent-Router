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
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LargeTopAppBar
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.rememberTopAppBarState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.data.AppState
import ca.whnet.agentrouter.data.Projection
import ca.whnet.agentrouter.data.Router
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RouterDetailScreen(
    app: AppState,
    router: Router,
    onBack: (() -> Unit)?,
    onSessions: () -> Unit,
    onUsage: () -> Unit,
    onAccounts: () -> Unit,
    onSettings: () -> Unit,
) {
    val state = app.state(router)
    val scope = rememberCoroutineScope()
    val bar = TopAppBarDefaults.exitUntilCollapsedScrollBehavior(rememberTopAppBarState())

    LaunchedEffect(router.id) { state.poll(10) }

    Scaffold(
        modifier = Modifier.fillMaxSize().nestedScroll(bar.nestedScrollConnection),
        topBar = {
            LargeTopAppBar(
                title = { Text(router.name) },
                navigationIcon = {
                    if (onBack != null) {
                        IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
                    }
                },
                actions = { TextButton(onClick = onSettings) { Text("Settings") } },
                scrollBehavior = bar,
            )
        },
    ) { pad ->
        Column(
            Modifier.fillMaxSize().padding(pad).readableWidth(820.dp)
                .bottomInset().verticalScroll(rememberScrollState())
        ) {

            state.failure?.let { f ->
                SettingsSection(footer = "Everything below is whatever was last seen.") {
                    Text(
                        f.message ?: "unreachable",
                        Modifier.padding(16.dp),
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }

            SettingsSection(header = "Now") {
                Column {
                    NavRow("Sessions", "${state.activeSessions} active · ${state.sessions.size} this week", onSessions)
                    HorizontalDivider()
                    NavRow("Usage", "Spend, tokens and what filled the context", onUsage)
                    HorizontalDivider()
                    NavRow(
                        "Accounts",
                        state.activeAccount?.title ?: "${state.accounts.size} configured",
                        onAccounts,
                    )
                }
            }

            val fc = state.activeAccount
            SettingsSection(
                header = "Quota",
                footer = state.quota?.source?.let { "Source: $it" },
            ) {
                Column {
                    QuotaRow(
                        "Five hours",
                        state.quota?.fiveHour,
                        Projection.describe(fc?.fiveHour),
                    )
                    HorizontalDivider()
                    QuotaRow(
                        "Seven days",
                        state.quota?.sevenDay,
                        Projection.describe(fc?.sevenDay),
                    )
                }
            }

            BurnDownSection("Five-hour burn-down", fc?.fiveHour)
            BurnDownSection("Seven-day burn-down", fc?.sevenDay)

            if (state.switchable.size > 1) {
                SettingsSection(
                    header = "Serving",
                    footer = "A switch takes effect on the next request the router proxies.",
                ) {
                    Column {
                        state.switchable.forEachIndexed { i, acc ->
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
                                StateDot(if (acc.isActive) "working" else null)
                                Column(Modifier.weight(1f)) {
                                    Text(acc.title, style = MaterialTheme.typography.bodyLarge)
                                    val bits = buildList {
                                        acc.plan?.let { add(it) }
                                        acc.fiveHour?.used?.let { add("5h ${(it * 100).roundToInt()}%") }
                                        acc.sevenDay?.used?.let { add("7d ${(it * 100).roundToInt()}%") }
                                    }
                                    if (bits.isNotEmpty()) {
                                        Text(
                                            bits.joinToString(" · "),
                                            style = MaterialTheme.typography.bodySmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                }
                                if (state.switching == acc.id) {
                                    CircularProgressIndicator(Modifier.padding(4.dp), strokeWidth = 2.dp)
                                } else if (acc.isActive) {
                                    Text("serving", style = MaterialTheme.typography.labelMedium)
                                }
                            }
                        }
                    }
                }
            }

            state.forecast?.global?.let { g ->
                SettingsSection(
                    header = "Across all accounts",
                    footer = g.pace30d?.let { "Pace over 30 days, ${it.basis}." },
                ) {
                    StatTiles(
                        listOf(
                            Triple("Spend", Fmt.usd(g.cost), null),
                            Triple("Requests", Fmt.compact(g.requests.toLong()), null),
                            Triple("Read", Fmt.compact(g.promptTokens.toLong()), null),
                            Triple("Written", Fmt.compact(g.outputTokens.toLong()), null),
                        ),
                        Modifier.padding(horizontal = 12.dp),
                    )
                }
            }

            SettingsSection(header = "Router") {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Row {
                        Text("Address", Modifier.weight(1f))
                        Text(router.address, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    Row {
                        Text("Version", Modifier.weight(1f))
                        Text(
                            state.health?.let { "${it.version} · pid ${it.pid}" } ?: "—",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun NavRow(title: String, detail: String, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            Text(
                detail,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Icon(
            Icons.AutoMirrored.Filled.KeyboardArrowRight,
            null,
            tint = MaterialTheme.colorScheme.outline,
        )
    }
}
