// The four lists under the dashboard, and one request in full.
//
// Each narrows the same scope the Usage page is showing, which is why they all take the
// `since` the page was on rather than choosing their own range.

package ca.whnet.agentrouter.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
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
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.api.ProjectRow
import ca.whnet.agentrouter.api.ProxySessionRow
import ca.whnet.agentrouter.api.RequestRow
import ca.whnet.agentrouter.api.RouterClient
import ca.whnet.agentrouter.api.ToolsPayload
import ca.whnet.agentrouter.api.get
import ca.whnet.agentrouter.data.AppState
import ca.whnet.agentrouter.data.Router
import ca.whnet.agentrouter.data.RouterState

/** What the four lists have in common: fetch once for a scope, then render. */
@Composable
private fun <T> Loader(
    client: RouterClient?,
    path: String,
    fetch: suspend (RouterClient, String) -> T,
    content: @Composable (T) -> Unit,
) {
    var value by remember(path) { mutableStateOf<T?>(null) }
    var failure by remember(path) { mutableStateOf<String?>(null) }

    LaunchedEffect(path, client) {
        if (client == null) { failure = "bad address"; return@LaunchedEffect }
        runCatching { fetch(client, path) }
            .onSuccess { value = it; failure = null }
            .onFailure { failure = RouterState.describe(it) }
    }

    failure?.let {
        SettingsSection { Text(it, Modifier.padding(16.dp), color = MaterialTheme.colorScheme.error) }
        return
    }
    val v = value
    if (v == null) {
        Row(Modifier.fillMaxWidth().padding(32.dp), horizontalArrangement = Arrangement.Center) {
            CircularProgressIndicator()
        }
        return
    }
    content(v)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ListScaffold(title: String, onBack: () -> Unit, content: @Composable () -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title) },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
                },
            )
        },
    ) { pad ->
        Column(Modifier.fillMaxSize().padding(pad).verticalScroll(rememberScrollState())) { content() }
    }
}

@Composable
fun ProjectsScreen(app: AppState, router: Router, since: String?, onBack: () -> Unit) {
    val client = app.state(router).liveClient
    val path = client?.query("/api/projects", mapOf("since" to since)) ?: ""
    ListScaffold("Projects", onBack) {
        Loader(client, path, { c, p -> c.get<List<ProjectRow>>(p) }) { rows ->
            SettingsSection(footer = "Cost is what the proxy recorded, not what was billed.") {
                Column {
                    rows.sortedByDescending { it.cost }.forEachIndexed { i, r ->
                        if (i > 0) HorizontalDivider()
                        TwoLineRow(
                            title = r.name,
                            detail = "${Fmt.compact(r.sessions.toLong())} sessions · " +
                                "${Fmt.compact(r.requests.toLong())} requests · " +
                                "${Fmt.compact(r.promptTokens.toLong())} read" +
                                if (r.errors > 0) " · ${r.errors.toInt()} errors" else "",
                            trailing = Fmt.usd(r.cost),
                        )
                    }
                }
            }
        }
    }
}

@Composable
fun ProxySessionsScreen(app: AppState, router: Router, since: String?, onBack: () -> Unit) {
    val client = app.state(router).liveClient
    val path = client?.query("/api/sessions", mapOf("since" to since)) ?: ""
    ListScaffold("Sessions", onBack) {
        Loader(client, path, { c, p -> c.get<List<ProxySessionRow>>(p) }) { rows ->
            SettingsSection(
                footer = "These are sessions the proxy served, which is not the same list as " +
                    "the transcripts under Sessions on the router page.",
            ) {
                Column {
                    rows.forEachIndexed { i, r ->
                        if (i > 0) HorizontalDivider()
                        TwoLineRow(
                            title = "${r.projectName} · ${r.shortId}",
                            detail = buildList {
                                add(Fmt.ago(r.lastSeenAt))
                                if (r.client.isNotEmpty()) add(r.client)
                                r.gitBranch?.let { add(it) }
                                add("${Fmt.compact(r.requests.toLong())} requests")
                                if (r.subagents > 0) add("${r.subagents.toInt()} subagents")
                            }.joinToString(" · "),
                            trailing = Fmt.usd(r.cost),
                        )
                    }
                }
            }
        }
    }
}

@Composable
fun ToolsScreen(app: AppState, router: Router, since: String?, onBack: () -> Unit) {
    val client = app.state(router).liveClient
    val path = client?.query("/api/tools", mapOf("since" to since)) ?: ""
    ListScaffold("Tools", onBack) {
        Loader(client, path, { c, p -> c.get<ToolsPayload>(p) }) { payload ->
            if (payload.called.isNotEmpty()) {
                val total = payload.called.sumOf { it.calls }.takeIf { it > 0 } ?: 1.0
                SettingsSection(header = "Called", footer = "How often each tool was asked for.") {
                    Column(Modifier.padding(16.dp)) {
                        payload.called.sortedByDescending { it.calls }.forEach { c ->
                            ShareBar(
                                c.name ?: "unknown",
                                c.calls / total,
                                "${Fmt.compact(c.calls.toLong())}" +
                                    if (c.subagentCalls > 0) " · ${Fmt.compact(c.subagentCalls.toLong())} by subagents" else "",
                            )
                        }
                    }
                }
            }
            if (payload.produced.isNotEmpty()) {
                SettingsSection(
                    header = "What came back",
                    footer = "Tool results are re-read on every later turn, so a large one is " +
                        "paid for more than once. `max` is the largest single result.",
                ) {
                    Column {
                        payload.produced.sortedByDescending { it.estTokens }.forEachIndexed { i, p ->
                            if (i > 0) HorizontalDivider()
                            TwoLineRow(
                                title = p.name ?: "unknown",
                                detail = "${Fmt.compact(p.results.toLong())} results · " +
                                    "max ${Fmt.compact(p.maxTokens.toLong())}" +
                                    if (p.errors > 0) " · ${p.errors.toInt()} errors" else "",
                                trailing = Fmt.compact(p.estTokens.toLong()),
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun RequestsScreen(
    app: AppState,
    router: Router,
    since: String?,
    onBack: () -> Unit,
    onOpen: (RequestRow) -> Unit,
) {
    val client = app.state(router).liveClient
    val path = client?.query("/api/requests", mapOf("since" to since)) ?: ""
    ListScaffold("Requests", onBack) {
        Loader(client, path, { c, p -> c.get<List<RequestRow>>(p) }) { rows ->
            SettingsSection(footer = "The most recent requests the proxy served. Tap one for its detail.") {
                Column {
                    rows.forEachIndexed { i, r ->
                        if (i > 0) HorizontalDivider()
                        Row(
                            Modifier.fillMaxWidth().clickable { onOpen(r) }.padding(16.dp),
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            StateDot(if (r.isError) "failed" else "working", size = 8)
                            Column(Modifier.weight(1f)) {
                                Text(r.model ?: "—", style = MaterialTheme.typography.bodyMedium)
                                Text(
                                    buildList {
                                        add(Fmt.ago(r.startedAt))
                                        r.status?.let { add(it.toString()) }
                                        r.durationMs?.let { add(Fmt.duration(it)) }
                                        if (r.isSubagent) add("subagent")
                                        r.stopReason?.let { add(it) }
                                    }.joinToString(" · "),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = if (r.isError) MaterialTheme.colorScheme.error
                                    else MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                            Text(Fmt.usd(r.cost), style = MaterialTheme.typography.bodySmall)
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun RequestDetailScreen(request: RequestRow, onBack: () -> Unit) {
    ListScaffold("Request", onBack) {
        SettingsSection(header = "What it was") {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Field("Model", request.model ?: "—")
                Field("Started", Fmt.ago(request.startedAt))
                Field("Status", request.status?.toString() ?: "—")
                request.stopReason?.let { Field("Stopped because", it) }
                request.ttfbMs?.let { Field("First byte", Fmt.duration(it)) }
                request.durationMs?.let { Field("Took", Fmt.duration(it)) }
                if (request.isSubagent) Field("Thread", "subagent ${request.agentId ?: ""}")
                request.project?.let { Field("Project", it.substringAfterLast('/')) }
            }
        }

        SettingsSection(
            header = "Tokens",
            footer = "Cache reads are charged at a tenth of the input rate, which is why a " +
                "large read can still be cheap.",
        ) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Field("Read", Fmt.compact((request.promptTokens ?: 0.0).toLong()))
                Field("Input", Fmt.compact((request.inputTokens ?: 0.0).toLong()))
                Field("Cache read", Fmt.compact((request.cacheReadTokens ?: 0.0).toLong()))
                Field("Cache written", Fmt.compact((request.cacheWriteTokens ?: 0.0).toLong()))
                Field("Written", Fmt.compact((request.outputTokens ?: 0.0).toLong()))
                request.thinkingTokens?.let { Field("Thinking", Fmt.compact(it.toLong())) }
                Field("Cost", Fmt.usd(request.cost))
            }
        }

        if (request.contextSplit.isNotEmpty()) {
            val total = request.contextSplit.sumOf { it.second }.takeIf { it > 0 } ?: 1.0
            SettingsSection(header = "What filled the window") {
                Column(Modifier.padding(16.dp)) {
                    request.contextSplit.sortedByDescending { it.second }.forEach { (name, v) ->
                        ShareBar(name, v / total, Fmt.compact(v.toLong()))
                    }
                }
            }
        }

        if (request.toolList.isNotEmpty()) {
            SettingsSection(header = "Tools called", footer = "In the order they were called.") {
                Text(
                    request.toolList.joinToString(", "),
                    Modifier.padding(16.dp),
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                )
            }
        }

        request.error?.let { e ->
            SettingsSection(header = "Error") {
                Text(
                    e,
                    Modifier.padding(16.dp).horizontalScroll(rememberScrollState()),
                    color = MaterialTheme.colorScheme.error,
                    fontFamily = FontFamily.Monospace,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }
    }
}

@Composable
private fun Field(label: String, value: String) {
    Row(Modifier.fillMaxWidth()) {
        Text(label, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
        Text(
            value,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun TwoLineRow(title: String, detail: String, trailing: String) {
    Row(
        Modifier.fillMaxWidth().padding(16.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyMedium)
            Text(
                detail,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Text(trailing, style = MaterialTheme.typography.bodyMedium)
    }
}
