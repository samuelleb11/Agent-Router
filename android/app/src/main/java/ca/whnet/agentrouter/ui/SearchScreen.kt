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
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.api.SearchHit
import ca.whnet.agentrouter.api.SearchReply
import ca.whnet.agentrouter.api.SearchSessionRow
import ca.whnet.agentrouter.api.get
import ca.whnet.agentrouter.data.AppState
import ca.whnet.agentrouter.data.Router
import ca.whnet.agentrouter.data.RouterState
import ca.whnet.agentrouter.data.Snippet
import kotlinx.coroutines.delay

/** The facets the router indexes. The first sends no `kind`, which means "don't narrow". */
private val KINDS = listOf(
    "All" to null,
    "Messages" to "text",
    "Tool calls" to "tool_use",
    "Results" to "tool_result",
    "Thinking" to "thinking",
)

/**
 * Search everything this router has recorded.
 *
 * Per router, because the index belongs to one: each router has its own store, and there
 * is nothing to ask for a view across several.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SearchScreen(
    app: AppState,
    router: Router,
    onBack: () -> Unit,
    onOpenSession: (String) -> Unit,
) {
    val state = app.state(router)
    var query by rememberSaveable { mutableStateOf("") }
    var kind by rememberSaveable { mutableStateOf(0) }
    var reply by remember { mutableStateOf<SearchReply?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var searching by remember { mutableStateOf(false) }

    // Answers the keyboard, not the clock — there is no polling here. Re-keying the
    // effect on every keystroke cancels the previous one, so the delay at the top of it
    // is the whole debounce and no sequence counter is needed to keep answers in order.
    LaunchedEffect(query, kind, router.id) {
        val q = query.trim()
        if (q.isEmpty()) {
            reply = null
            failure = null
            return@LaunchedEffect
        }
        delay(220)
        val client = state.liveClient
        if (client == null) {
            failure = "This router has no usable address."
            return@LaunchedEffect
        }
        searching = true
        // Smaller than the dashboard's sixty: this is a phone, often on cellular, and
        // the rows past the first screenful are rarely the ones wanted.
        val path = client.query(
            "/api/search",
            mapOf("q" to q, "kind" to KINDS[kind].second, "limit" to "25"),
        )
        runCatching { client.get<SearchReply>(path) }
            .onSuccess { reply = it; failure = null }
            .onFailure { failure = RouterState.describe(it) }
        searching = false
    }

    Scaffold(
        modifier = Modifier.fillMaxSize(),
        topBar = {
            TopAppBar(
                title = { Text("Search") },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
                },
            )
        },
    ) { pad ->
        Column(Modifier.fillMaxSize().padding(pad)) {
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                singleLine = true,
                label = { Text("A word, a phrase, an identifier") },
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
            )
            Row(
                Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp, vertical = 4.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                KINDS.forEachIndexed { i, (label, _) ->
                    FilterChip(selected = kind == i, onClick = { kind = i }, label = { Text(label) })
                }
            }

            val r = reply
            val sessions = r?.sessions.orEmpty()
            val hits = r?.hits.orEmpty()

            LazyColumn(Modifier.fillMaxSize()) {
                failure?.let {
                    item {
                        SettingsSection {
                            Text(it, Modifier.padding(16.dp), color = MaterialTheme.colorScheme.error)
                        }
                    }
                }

                // Only worth saying when the router ran something other than what was typed.
                val expression = r?.expression
                if (!expression.isNullOrEmpty() && expression != query.trim()) {
                    item {
                        Text(
                            "Searched for $expression",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp),
                        )
                    }
                }

                if (sessions.isNotEmpty()) {
                    item {
                        SettingsSection(header = "Sessions that mention it") {
                            Column {
                                sessions.forEachIndexed { i, s ->
                                    if (i > 0) HorizontalDivider()
                                    SessionHitRow(s) { onOpenSession(s.sessionId) }
                                }
                            }
                        }
                    }
                }

                if (hits.isNotEmpty()) {
                    item {
                        Text(
                            if (hits.size == 1) "1 match" else "${hits.size} matches",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.primary,
                            modifier = Modifier.padding(start = 20.dp, top = 12.dp, bottom = 2.dp),
                        )
                    }
                    items(hits, key = { it.key }) { hit ->
                        SettingsSection { HitRow(hit) { onOpenSession(hit.sessionId) } }
                    }
                }

                if (query.isBlank()) {
                    item {
                        EmptyState(
                            "Search the transcripts",
                            "Every message, tool call and result this router has recorded.",
                        )
                    }
                } else if (r == null && failure == null) {
                    item {
                        Row(
                            Modifier.fillMaxWidth().padding(32.dp),
                            horizontalArrangement = Arrangement.Center,
                        ) { CircularProgressIndicator() }
                    }
                } else if (hits.isEmpty() && failure == null && !searching) {
                    // "Nothing matched" and "not asked yet" are different claims; this is
                    // only reached once the router has actually answered.
                    item { EmptyState("Nothing matched", "No message in this router's history contains that.") }
                }
            }
        }
    }
}

@Composable
private fun SessionHitRow(s: SearchSessionRow, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(s.title, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                "${s.projectName} · ${Fmt.compact(s.blocks)} messages · ${Fmt.ago(s.lastAt)}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Text(
            Fmt.compact(s.hits),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
        )
    }
}

@Composable
private fun HitRow(hit: SearchHit, onClick: () -> Unit) {
    Column(Modifier.fillMaxWidth().clickable(onClick = onClick).padding(16.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(
                "${hit.kindName} · ${hit.role}",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.weight(1f),
            )
            Text(
                Fmt.ago(hit.at),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.outline,
            )
        }
        Text(
            highlighted(hit.snippet),
            style = MaterialTheme.typography.bodyMedium,
            maxLines = 4,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.padding(top = 4.dp),
        )
        Text(
            "${hit.projectName} · ${hit.shortSession}",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.outline,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.padding(top = 2.dp),
        )
    }
}

/** The matched runs in bold and in the primary colour; memoized, as inline text is. */
@Composable
private fun highlighted(snippet: String): AnnotatedString {
    val hit = MaterialTheme.colorScheme.primary
    return remember(snippet, hit) {
        buildAnnotatedString {
            for (run in Snippet.parse(snippet)) {
                if (run.match) {
                    withStyle(SpanStyle(color = hit, fontWeight = FontWeight.Bold)) { append(run.text) }
                } else {
                    append(run.text)
                }
            }
        }
    }
}
