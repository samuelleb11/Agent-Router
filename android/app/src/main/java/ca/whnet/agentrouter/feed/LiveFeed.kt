// One open session: its feed as the router streams it, and the ways back into it.

package ca.whnet.agentrouter.feed

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import ca.whnet.agentrouter.api.AnswerBody
import ca.whnet.agentrouter.api.AsideBody
import ca.whnet.agentrouter.api.AsideReply
import ca.whnet.agentrouter.api.EffortBody
import ca.whnet.agentrouter.api.LiveItem
import ca.whnet.agentrouter.api.LiveOptions
import ca.whnet.agentrouter.api.LivePrompt
import ca.whnet.agentrouter.api.LiveReply
import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.LiveStatus
import ca.whnet.agentrouter.api.ModeBody
import ca.whnet.agentrouter.api.ModelBody
import ca.whnet.agentrouter.api.ProjectStats
import ca.whnet.agentrouter.api.RouterClient
import ca.whnet.agentrouter.api.RouterFailure
import ca.whnet.agentrouter.api.SendBody
import ca.whnet.agentrouter.api.SessionBody
import ca.whnet.agentrouter.api.SessionStats
import ca.whnet.agentrouter.api.SwitchReply
import ca.whnet.agentrouter.api.get
import ca.whnet.agentrouter.api.post
import ca.whnet.agentrouter.api.routerJson
import ca.whnet.agentrouter.data.RouterState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.isActive
import java.util.UUID
import kotlin.coroutines.coroutineContext

class LiveFeed(val session: LiveSession, private val client: RouterClient?) {

    data class Row(
        val id: String = UUID.randomUUID().toString(),
        val item: LiveItem,
        /**
         * Echoed the moment a send succeeds; the real row arrives when the session writes
         * it to its transcript, which on a busy session can be minutes.
         */
        val pending: Boolean = false,
        val answered: Boolean = false,
    )

    /** `at` makes two identical notices in a row distinct, so the second one still shows. */
    data class Notice(val text: String, val ok: Boolean, val at: Long = System.nanoTime())

    val rows = mutableStateListOf<Row>()
    var status: LiveStatus? by mutableStateOf(null)
    var connected: Boolean by mutableStateOf(false)
    var notice: Notice? by mutableStateOf(null)
    var stats: SessionStats? by mutableStateOf(null)
    var project: ProjectStats? by mutableStateOf(null)
    var options: LiveOptions? by mutableStateOf(null)
    /**
     * Set only while the session is blocked on a permission request; a question is left
     * to the feed, which has it from the transcript.
     */
    var pendingPrompt: LivePrompt? by mutableStateOf(null)
    var sending: Boolean by mutableStateOf(false)
    var stopping: Boolean by mutableStateOf(false)

    private companion object {
        /** Enough to scroll back through; the backfill is capped at the router too. */
        const val CAP = 600
    }

    /**
     * Runs for the life of the screen. The feed is rebuilt from scratch on every attempt —
     * that is also what de-duplicates the optimistic echoes below, since the backfill
     * contains the real rows by then.
     */
    suspend fun run() {
        val c = client ?: run {
            notice = Notice("This router has no usable address.", false)
            return
        }
        while (coroutineContext.isActive) {
            rows.clear()
            var fatal = false
            c.events("/api/live/stream?session=${session.sessionId}")
                .catch { e ->
                    if (e is CancellationException) throw e
                    connected = false
                    // A wrong password will not come right by retrying.
                    if (e is RouterFailure.Unauthorized || e is RouterFailure.NoPasswordSet) {
                        notice = Notice(e.message ?: "refused", false)
                        fatal = true
                    }
                }
                .collect { event ->
                    connected = true
                    when (event.event) {
                        null -> runCatching { routerJson.decodeFromString<LiveItem>(event.data) }
                            .onSuccess { append(Row(item = it)) }
                        "status" -> runCatching { routerJson.decodeFromString<LiveStatus>(event.data) }
                            .onSuccess { status = it }
                        "error" -> notice = Notice(event.data, false)
                    }
                }
            connected = false
            if (fatal) return
            delay(2000)
        }
    }

    private fun append(row: Row) {
        rows.add(row)
        while (rows.size > CAP) rows.removeAt(0)
    }

    suspend fun send(text: String): Boolean {
        val c = client ?: return false
        val body = text.trim()
        if (body.isEmpty()) return false
        sending = true
        try {
            val reply: LiveReply = c.post("/api/live/send", SendBody(session.sessionId, body))
            if (!reply.ok) {
                notice = Notice(reply.reason ?: "The router would not take it.", false)
                return false
            }
            append(Row(item = LiveItem(role = "user", kind = "text", text = body), pending = true))
            notice = when {
                reply.framed == true -> Notice(
                    "Sent — it arrives as a message from another Claude, not as you.", true
                )
                status?.state == "ask" -> Notice(
                    "Sent as a message — it may not dismiss the prompt.", true
                )
                else -> Notice("Delivered as your message.", true)
            }
            return true
        } catch (e: Throwable) {
            if (e is CancellationException) throw e
            notice = Notice(RouterState.describe(e), false)
            return false
        } finally {
            sending = false
        }
    }

    /**
     * Answer the waiting question. If the router says the session is no longer waiting —
     * which it does with a 503 — the picked answers go in as an ordinary message rather
     * than being lost.
     */
    suspend fun answer(answers: Map<String, String>, fallback: List<String>, rowId: String): Boolean {
        val c = client ?: return false
        return try {
            val reply: LiveReply = c.post("/api/live/answer", AnswerBody(session.sessionId, answers))
            if (!reply.ok) throw RouterFailure.Status(503, reply.reason ?: "not waiting")
            val i = rows.indexOfFirst { it.id == rowId }
            if (i >= 0) rows[i] = rows[i].copy(answered = true)
            notice = Notice("Answered.", true)
            true
        } catch (e: Throwable) {
            if (e is CancellationException) throw e
            send(fallback.joinToString("\n"))
        }
    }

    suspend fun interrupt() {
        val c = client ?: return
        stopping = true
        try {
            c.post<SessionBody, LiveReply>("/api/live/interrupt", SessionBody(session.sessionId))
            notice = Notice("Asked it to stop.", true)
        } catch (e: Throwable) {
            if (e is CancellationException) throw e
            notice = Notice(RouterState.describe(e), false)
        } finally {
            stopping = false
        }
    }

    suspend fun setModel(model: String) {
        val c = client ?: return
        runCatching { c.post<ModelBody, SwitchReply>("/api/live/model", ModelBody(session.sessionId, model)) }
            .onSuccess { r ->
                notice = when {
                    !r.ok -> Notice(r.reason ?: "The session refused that model.", false)
                    r.pending == true -> Notice("Asked for $model; it has not confirmed yet.", true)
                    else -> Notice("Now on ${r.model ?: model}.", true)
                }
                loadFacts()
            }
            .onFailure { notice = Notice(RouterState.describe(it), false) }
    }

    /**
     * Change what the session stops to ask about. The CLI can refuse — `bypassPermissions`
     * needs the session to have been launched with `--dangerously-skip-permissions`, and
     * `auto` can be gated — so its own sentence is what gets shown.
     */
    suspend fun setMode(mode: String) {
        val c = client ?: return
        runCatching { c.post<ModeBody, SwitchReply>("/api/live/mode", ModeBody(session.sessionId, mode)) }
            .onSuccess { r ->
                notice = when {
                    !r.ok -> Notice(r.reason ?: "The session refused that mode.", false)
                    r.pending == true -> Notice("Asked for $mode; it has not confirmed yet.", true)
                    else -> Notice("Now ${r.mode ?: mode}.", true)
                }
                loadFacts()
            }
            .onFailure { notice = Notice(RouterState.describe(it), false) }
    }

    /**
     * What the session is blocked on, when it is a permission request rather than a
     * question. It arrives as a control frame between the CLI and its editor, so it is
     * never in the transcript the feed is built from and has to be asked for.
     */
    suspend fun loadPrompt() {
        val c = client ?: return
        runCatching { c.get<LivePrompt>("/api/live/prompt?session=${'$'}{session.sessionId}") }
            .onSuccess { pendingPrompt = if (it.isPermission) it else null }
    }

    /**
     * Allow the waiting tool, optionally taking one of the prompt's own standing offers,
     * or refuse it.
     */
    suspend fun answerPermission(allow: Boolean, suggestion: Int? = null, said: String) {
        val c = client ?: return
        val body = AnswerBody(
            session = session.sessionId,
            allow = allow,
            suggestion = suggestion,
            message = if (allow) null else "Declined from the phone.",
        )
        runCatching { c.post<AnswerBody, LiveReply>("/api/live/answer", body) }
            .onSuccess { r ->
                if (r.ok) { pendingPrompt = null; notice = Notice(said, true) }
                else notice = Notice(r.reason ?: "Could not answer that prompt.", false)
            }
            .onFailure { notice = Notice(RouterState.describe(it), false) }
    }

    suspend fun setEffort(effort: String) {
        val c = client ?: return
        runCatching { c.post<EffortBody, SwitchReply>("/api/live/effort", EffortBody(session.sessionId, effort)) }
            .onSuccess { r ->
                notice = if (r.ok) Notice("Effort is now ${r.effort ?: effort}.", true)
                else Notice(r.reason ?: "The session refused that.", false)
                loadFacts()
            }
            .onFailure { notice = Notice(RouterState.describe(it), false) }
    }

    /** Ask a forked copy something without disturbing the session. Can take a while. */
    suspend fun aside(text: String): AsideReply? {
        val c = client ?: return null
        return runCatching { c.post<AsideBody, AsideReply>("/api/live/aside", AsideBody(session.sessionId, text)) }
            .onFailure { notice = Notice(RouterState.describe(it), false) }
            .getOrNull()
    }

    suspend fun loadFacts() {
        val c = client ?: return
        runCatching { c.get<SessionStats>("/api/live/stats?session=${session.sessionId}") }
            .onSuccess { stats = it }
        runCatching { c.get<LiveOptions>("/api/live/options") }.onSuccess { options = it }
        session.cwd?.let { cwd ->
            val path = c.query("/api/live/project", mapOf("cwd" to cwd))
            runCatching { c.get<ProjectStats>(path) }.onSuccess { project = it }
        }
    }
}
