package ca.whnet.agentrouter.ui

import androidx.compose.foundation.horizontalScroll
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
import androidx.compose.foundation.clickable
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.api.Summary
import ca.whnet.agentrouter.api.get
import ca.whnet.agentrouter.data.AppState
import ca.whnet.agentrouter.data.Router
import ca.whnet.agentrouter.data.RouterState

/** The ranges the dashboard offers, and how far back each reaches. */
private val RANGES = listOf(
    "24h" to 24L * 3600 * 1000,
    "7d" to 7L * 24 * 3600 * 1000,
    "30d" to 30L * 24 * 3600 * 1000,
    "All" to 0L,
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun UsageScreen(
    app: AppState,
    router: Router,
    onBack: () -> Unit,
    onList: (kind: String, since: String?) -> Unit,
) {
    val state = app.state(router)
    var range by remember { mutableStateOf(1) }
    var summary by remember { mutableStateOf<Summary?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var loading by remember { mutableStateOf(true) }

    LaunchedEffect(router.id, range) {
        loading = true
        val client = state.liveClient
        if (client == null) {
            failure = "bad address"; loading = false
            return@LaunchedEffect
        }
        val since = sinceFor(range)
        val tz = (-java.util.TimeZone.getDefault().rawOffset / 60000).toString()
        val path = client.query("/api/summary", mapOf("since" to since, "tz" to tz))
        runCatching { client.get<Summary>(path) }
            .onSuccess { summary = it; failure = null }
            .onFailure { failure = RouterState.describe(it) }
        loading = false
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Usage") },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
                },
            )
        },
    ) { pad ->
        Column(Modifier.fillMaxSize().padding(pad).verticalScroll(rememberScrollState())) {

            Row(
                Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                RANGES.forEachIndexed { i, (label, _) ->
                    FilterChip(
                        selected = range == i,
                        onClick = { range = i },
                        label = { Text(label) },
                    )
                }
            }

            failure?.let {
                SettingsSection { Text(it, Modifier.padding(16.dp), color = MaterialTheme.colorScheme.error) }
            }
            if (loading && summary == null) {
                Row(Modifier.fillMaxWidth().padding(32.dp), horizontalArrangement = Arrangement.Center) {
                    CircularProgressIndicator()
                }
            }

            val s = summary ?: return@Column
            val t = s.totals
            val since = sinceFor(range)

            SettingsSection(header = "Totals") {
                Column(Modifier.padding(horizontal = 12.dp)) {
                    Row(Modifier.fillMaxWidth()) {
                        StatTile("Spend", Fmt.usd(t.cost), modifier = Modifier.weight(1f))
                        StatTile("Requests", Fmt.compact(t.requests.toLong()), modifier = Modifier.weight(1f))
                        StatTile("Sessions", Fmt.compact(t.sessions.toLong()), modifier = Modifier.weight(1f))
                    }
                    Row(Modifier.fillMaxWidth()) {
                        StatTile("Read", Fmt.compact(t.promptTokens.toLong()), modifier = Modifier.weight(1f))
                        StatTile("Written", Fmt.compact(t.outputTokens.toLong()), modifier = Modifier.weight(1f))
                        StatTile(
                            "Errors",
                            Fmt.compact(t.errors.toLong()),
                            detail = if (t.subagentRequests > 0)
                                "${Fmt.compact(t.subagentRequests.toLong())} subagent" else null,
                            modifier = Modifier.weight(1f),
                        )
                    }
                }
            }

            if (s.timeline.rows.isNotEmpty()) {
                SettingsSection(
                    header = "Activity",
                    footer = "Requests per bucket over the range.",
                ) {
                    Column(Modifier.padding(16.dp)) {
                        ActivityChart(s.timeline.rows.map { it.requests })
                    }
                }
            }

            if (s.quotaHistory.size > 1) {
                SettingsSection(
                    header = "Quota over the range",
                    footer = "Read from the rate-limit headers as requests went through. Blue is " +
                        "the five-hour window, orange the seven-day; each drop is a reset.",
                ) {
                    Column(Modifier.padding(16.dp)) {
                        QuotaHistoryChart(
                            fiveHour = s.quotaHistory.mapNotNull { p -> p.rl5h?.let { p.t to it } },
                            sevenDay = s.quotaHistory.mapNotNull { p -> p.rl7d?.let { p.t to it } },
                        )
                    }
                }
            }

            if (s.context.isNotEmpty()) {
                val total = s.context.sumOf { it.estTokens }.takeIf { it > 0 } ?: 1.0
                SettingsSection(
                    header = "What fills the context window",
                    footer = "Estimated from the characters sent, apportioned by share.",
                ) {
                    Column(Modifier.padding(16.dp)) {
                        s.context.sortedByDescending { it.estTokens }.forEach { row ->
                            ShareBar(
                                row.component,
                                row.estTokens / total,
                                Fmt.compact(row.estTokens.toLong()),
                            )
                        }
                    }
                }
            }

            if (s.models.isNotEmpty()) {
                val total = s.models.sumOf { it.cost }.takeIf { it > 0 } ?: 1.0
                SettingsSection(header = "Models") {
                    Column(Modifier.padding(16.dp)) {
                        s.models.sortedByDescending { it.cost }.forEach { m ->
                            ShareBar(
                                m.model ?: "unknown",
                                m.cost / total,
                                "${Fmt.usd(m.cost)} · ${Fmt.compact(m.requests.toLong())}",
                            )
                        }
                    }
                }
            }

            if (s.threads.isNotEmpty()) {
                val total = s.threads.sumOf { it.cost }.takeIf { it > 0 } ?: 1.0
                SettingsSection(
                    header = "Main thread vs subagents",
                    footer = "A subagent's tokens are billed to the same account as the turn that spawned it.",
                ) {
                    Column(Modifier.padding(16.dp)) {
                        s.threads.forEach { th ->
                            ShareBar(th.thread, th.cost / total, Fmt.usd(th.cost))
                        }
                    }
                }
            }

            s.latency?.let { l ->
                SettingsSection(
                    header = "Latency",
                    footer = "Time to the first byte is what the session feels as a pause; the " +
                        "total is how long the turn took to finish streaming.",
                ) {
                    Column(Modifier.padding(horizontal = 12.dp)) {
                        Row(Modifier.fillMaxWidth()) {
                            StatTile("First byte p50", l.ttfbP50?.let { Fmt.duration(it.toLong()) } ?: "—", modifier = Modifier.weight(1f))
                            StatTile("p95", l.ttfbP95?.let { Fmt.duration(it.toLong()) } ?: "—", modifier = Modifier.weight(1f))
                        }
                        Row(Modifier.fillMaxWidth()) {
                            StatTile("Turn p50", l.durationP50?.let { Fmt.duration(it.toLong()) } ?: "—", modifier = Modifier.weight(1f))
                            StatTile("p95", l.durationP95?.let { Fmt.duration(it.toLong()) } ?: "—", modifier = Modifier.weight(1f))
                            StatTile("longest", l.durationMax?.let { Fmt.duration(it.toLong()) } ?: "—", modifier = Modifier.weight(1f))
                        }
                    }
                }
            }

            if (s.stopReasons.isNotEmpty()) {
                val total = s.stopReasons.sumOf { it.requests }.takeIf { it > 0 } ?: 1.0
                SettingsSection(
                    header = "How turns ended",
                    footer = "`tool_use` means the turn stopped to call something and carried on " +
                        "afterwards, so it is the ordinary case rather than a problem.",
                ) {
                    Column(Modifier.padding(16.dp)) {
                        s.stopReasons.sortedByDescending { it.requests }.forEach { r ->
                            ShareBar(
                                r.stopReason.ifEmpty { "unknown" },
                                r.requests / total,
                                Fmt.compact(r.requests.toLong()),
                            )
                        }
                    }
                }
            }

            SettingsSection(
                header = "In detail",
                footer = "Each list narrows the range above.",
            ) {
                Column {
                    UsageNavRow("Projects", "${t.projects.toInt()} in range") { onList("projects", since) }
                    HorizontalDivider()
                    UsageNavRow("Sessions", "${t.sessions.toInt()} the proxy served") { onList("sessions", since) }
                    HorizontalDivider()
                    UsageNavRow("Tools", "What was called, and what came back") { onList("tools", since) }
                    HorizontalDivider()
                    UsageNavRow("Requests", "The most recent, one by one") { onList("requests", since) }
                }
            }

            if (s.byAccount.size > 1) {
                val total = s.byAccount.sumOf { it.cost }.takeIf { it > 0 } ?: 1.0
                SettingsSection(header = "Accounts") {
                    Column(Modifier.padding(16.dp)) {
                        s.byAccount.sortedByDescending { it.cost }.forEach { a ->
                            ShareBar(
                                a.label + if (a.active == 1) " · serving" else "",
                                a.cost / total,
                                Fmt.usd(a.cost),
                            )
                        }
                    }
                }
            }
        }
    }
}


/** Milliseconds-since for a range index, or null for "all". */
private fun sinceFor(range: Int): String? =
    RANGES[range].second.let { if (it == 0L) null else (System.currentTimeMillis() - it).toString() }

@Composable
private fun UsageNavRow(title: String, detail: String, onClick: () -> Unit) {
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
