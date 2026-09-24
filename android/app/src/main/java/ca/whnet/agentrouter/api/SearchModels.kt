// What /api/search answers with.
//
// In its own file rather than appended to Models.kt: search is a self-contained slice of
// the API, and keeping it apart stops two features colliding in one busy file.
//
// Every snake_case key is spelled out, and everything carries a default, for the reasons
// Models.kt gives. Counts come from SQLite's count(*) and are always whole, so they are
// Long here — which is also what Fmt.compact takes. Only `rank` is a real.

package ca.whnet.agentrouter.api

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class SearchReply(
    val query: String = "",
    /// What the router actually ran. It differs from `query` when FTS5 refused what was
    /// typed — an apostrophe, a bare AND, an unclosed bracket — and the router retried it
    /// as a plain AND of quoted words. Worth showing when they differ, so a search that
    /// quietly changed shape does not read as a search that found nothing.
    val expression: String? = null,
    val sessions: List<SearchSessionRow> = emptyList(),
    val hits: List<SearchHit> = emptyList(),
)

@Serializable
data class SearchHit(
    @SerialName("block_id") val blockId: Long = 0,
    val kind: String = "",
    val chars: Long = 0,
    @SerialName("session_id") val sessionId: String = "",
    val role: String = "",
    val ordinal: Long = 0,
    @SerialName("request_id") val requestId: Long = 0,
    val at: Long = 0,
    val project: String? = null,
    @SerialName("session_label") val sessionLabel: String? = null,
    /// Matches wrapped in ‹…›. See data/Snippet.kt.
    val snippet: String = "",
    /// bm25: negative, lower is better. Rows arrive already ordered.
    val rank: Double = 0.0,
) {
    /// The same block appears at every place it was replayed, so the block alone is not
    /// an identity — and a LazyColumn given a duplicate key crashes rather than coping.
    val key: String get() = "$sessionId-$blockId-$ordinal"

    val shortSession: String get() = sessionId.take(8)
    val projectName: String get() = project?.substringAfterLast('/')?.ifEmpty { project } ?: "—"

    /// The wire's vocabulary is not the reader's.
    val kindName: String
        get() = when (kind) {
            "text" -> "message"
            "tool_use" -> "tool call"
            "tool_result" -> "tool result"
            "thinking" -> "thinking"
            "system" -> "system prompt"
            else -> kind
        }
}

@Serializable
data class SearchSessionRow(
    @SerialName("session_id") val sessionId: String = "",
    val project: String? = null,
    @SerialName("session_label") val sessionLabel: String? = null,
    @SerialName("git_branch") val gitBranch: String? = null,
    /// Distinct messages that matched, and places they matched.
    val blocks: Long = 0,
    val hits: Long = 0,
    @SerialName("first_at") val firstAt: Long = 0,
    @SerialName("last_at") val lastAt: Long = 0,
) {
    val shortId: String get() = sessionId.take(8)
    val projectName: String get() = project?.substringAfterLast('/')?.ifEmpty { project } ?: "—"
    val title: String get() = sessionLabel?.ifEmpty { null } ?: shortId
}
