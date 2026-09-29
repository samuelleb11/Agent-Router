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
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.net.URLEncoder
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
    /** The last status the stream sent. Whether it is still current is [statusFresh]. */
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

    /** A status has arrived on the connection now open, not just on one before it. */
    private var statusSeen: Boolean by mutableStateOf(false)
    /** When the stream last said anything, on a clock that never jumps. Only the watchdog reads it. */
    private var heardAt = 0L
    /** [SILENT_MS], shortened by tests that would otherwise wait ten seconds for it. */
    internal var silentMs = SILENT_MS

    /**
     * Whether [status] still describes the session. A dropped or silent stream leaves the
     * last status in place, and a header that kept showing it — "Running Bash · 12s", with
     * a Stop button — would be showing the past as the present. So the screen shows it only
     * while this holds, and says it is reconnecting otherwise.
     */
    val statusFresh: Boolean get() = connected && statusSeen

    /**
     * The last question seen waiting, so an answer from the feed can name it even once a
     * newer prompt — a permission request from a background agent, say — has become the
     * one `/api/live/prompt` reports. Not shown anywhere, so not observable state.
     */
    private var questionPrompt: LivePrompt? = null

    /**
     * Prompts answered from here. The router can list one for a moment after it was
     * answered, until the wrapper sees the answer go by; its card coming back in that
     * moment would invite a second tap on something already decided.
     */
    private val settled = LinkedHashSet<String>()

    companion object {
        /** Enough to scroll back through; the backfill is capped at the router too. */
        private const val CAP = 600

        /**
         * How long the stream may say nothing before the connection is given up on. The
         * router sends a status at least once a second (an older one every 200 ms once the
         * session has been quiet for 3 s, and on every change before that), so ten seconds
         * of nothing means the link died without either end noticing — the Mac asleep, the
         * phone off the LAN — which the socket's own read timeout takes 45 s to report.
         */
        const val SILENT_MS = 10_000L

        const val STREAM = "/api/live/stream"
        const val PROMPT = "/api/live/prompt"
        const val STATS = "/api/live/stats"

        /**
         * An endpoint about one session. Built in one place because a hand-written copy
         * once escaped its own template: `${'$'}` is a literal dollar in Kotlin, so the
         * phone asked for the prompt of a session named, literally, `${session.sessionId}`.
         * The router truthfully said nothing was waiting there, and the permission card
         * never appeared.
         */
        fun path(endpoint: String, sessionId: String): String =
            endpoint + "?session=" + URLEncoder.encode(sessionId, "UTF-8")

        /**
         * Decide how answers to a question in the feed reach the session, from what the
         * router says is waiting right now ([fresh], null when it could not be asked), the
         * last question seen waiting ([remembered]), and the questions of the form that was
         * submitted ([asked]).
         *
         * What this guards against, first: the router applies an answer to the prompt it
         * names or, named none, to the one prompt waiting — and a question's answers sent to
         * a Bash approval would approve the command. So when a permission prompt is known to
         * be waiting and the question cannot be named, or the router is too old to honour a
         * name (it sends no `count`, and answers the newest prompt whatever id it is given),
         * nothing is sent yet: the permission card is on screen, and once it is answered the
         * question is the newest prompt and can be named.
         *
         * Second: a form is only ever named onto the question it was drawn for. The feed
         * keeps every form, and one answered in the editor stays live here while the model
         * asks the next; naming the next one would write these answers into it and approve
         * it, dismissing it in the editor before anyone had read it. A different question
         * waiting now is [QuestionRoute.Different], and nothing is sent — the web view's rule.
         */
        fun routeForQuestion(fresh: LivePrompt?, remembered: LivePrompt?, asked: List<String>): QuestionRoute {
            if (fresh?.isQuestion == true) {
                if (!fresh.asks(asked)) return QuestionRoute.Different
                fresh.requestId?.let { return QuestionRoute.Named(it) }
            }
            if (fresh?.isPermission == true && fresh.count == null) return QuestionRoute.NotYet
            val seen = remembered?.takeIf { it.isQuestion }
            if (seen != null && seen.asks(asked)) seen.requestId?.let { return QuestionRoute.Named(it) }
            // Under a permission request the router reports only the newest prompt, so what
            // is waiting beneath it is not known for sure: answering that first brings the
            // question up where it can be compared.
            if (fresh?.isPermission == true) return QuestionRoute.NotYet
            // The router could not be asked, and the last question seen waiting is another.
            if (fresh == null && seen != null && !seen.asks(asked)) return QuestionRoute.Different
            return QuestionRoute.Unnamed
        }

        /** A clock that only ever moves forward, for measuring silence. */
        private fun clock(): Long = System.nanoTime() / 1_000_000
    }

    /** How answers to a question in the feed reach the session. */
    sealed interface QuestionRoute {
        /** Answer exactly this prompt: the router does, or says it is no longer waiting. */
        data class Named(val requestId: String) : QuestionRoute

        /** Nothing to name, and nothing else known to be waiting: the one prompt there is, as this app always asked. */
        data object Unnamed : QuestionRoute

        /**
         * Not now: an answer naming nothing would land on the permission prompt waiting
         * instead, and a message would only queue behind it and arrive later as a stray turn.
         */
        data object NotYet : QuestionRoute

        /**
         * Not at all: the question waiting now is another one than this form's, which was
         * answered elsewhere or went. Its answers belong to no prompt that is still asking.
         */
        data object Different : QuestionRoute
    }

    /**
     * Runs for the life of the screen: one connection after another, two seconds apart,
     * until the screen goes or the password is refused.
     */
    suspend fun run() {
        val c = client ?: run {
            notice = Notice("This router has no usable address.", false)
            return
        }
        while (coroutineContext.isActive) {
            if (!follow(c)) return
            delay(2000)
        }
    }

    /**
     * One connection's worth of the stream: until it ends, fails, or goes silent for longer
     * than the router ever is. Silence ends it as surely as an error does, so the retry
     * opens a fresh connection rather than waiting out a dead one. However it ends,
     * [connected] is false afterwards and the header stops presenting the last status as
     * the present. False when retrying cannot help.
     *
     * The feed is rebuilt from scratch on every connection — that is also what
     * de-duplicates the optimistic echoes below, since the backfill contains the real rows
     * by then — but only once the new connection delivers, so the rows already read stay
     * readable while a dead link is retried.
     */
    private suspend fun follow(c: RouterClient): Boolean = coroutineScope {
        var fatal = false
        var first = true
        statusSeen = false
        heardAt = clock()
        val reading = launch {
            c.events(path(STREAM, session.sessionId))
                .catch { e ->
                    if (e is CancellationException) throw e
                    // A wrong password will not come right by retrying.
                    if (e is RouterFailure.Unauthorized || e is RouterFailure.NoPasswordSet) {
                        notice = Notice(e.message ?: "refused", false)
                        fatal = true
                    }
                }
                .collect { event ->
                    heardAt = clock()
                    if (first) {
                        first = false
                        rows.clear()
                    }
                    connected = true
                    when (event.event) {
                        null -> runCatching { routerJson.decodeFromString<LiveItem>(event.data) }
                            .onSuccess { append(Row(item = it)) }
                        "status" -> runCatching { routerJson.decodeFromString<LiveStatus>(event.data) }
                            .onSuccess { status = it; statusSeen = true }
                        "error" -> notice = Notice(event.data, false)
                    }
                }
        }
        val watchdog = launch {
            while (true) {
                delay(1_000)
                if (clock() - heardAt > silentMs) {
                    connected = false
                    reading.cancel()
                    break
                }
            }
        }
        reading.join()
        watchdog.cancel()
        connected = false
        !fatal
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
     * Answer the waiting question, by name — the question of the form that was submitted,
     * whose texts are [asked], and no other. If the router says it is no longer waiting —
     * a 409 once it was named, a 503 from an older router — the picked answers go in as an
     * ordinary message rather than being lost. See [routeForQuestion] for when nothing is
     * sent; the form then keeps what was picked.
     */
    suspend fun answer(
        asked: List<String>,
        answers: Map<String, String>,
        fallback: List<String>,
        rowId: String,
    ): Boolean {
        val c = client ?: return false
        // Asked afresh rather than taken from the last poll, which can be two seconds old.
        val fresh = runCatching { c.get<LivePrompt>(path(PROMPT, session.sessionId)) }
            .getOrElse { if (it is CancellationException) throw it; null }
        val route = routeForQuestion(fresh, questionPrompt, asked)
        fresh?.let(::track)
        when (route) {
            QuestionRoute.NotYet -> {
                notice = Notice("A permission request is waiting too — answer it first, then submit these again.", false)
                return false
            }
            // The web view's words for the same refusal, so the two clients say it alike.
            QuestionRoute.Different -> {
                notice = Notice("Not sent: a different question is waiting now.", false)
                return false
            }
            else -> Unit
        }
        val named = (route as? QuestionRoute.Named)?.requestId
        return try {
            val reply: LiveReply = c.post(
                "/api/live/answer",
                AnswerBody(session.sessionId, answers, requestId = named),
            )
            if (!reply.ok) throw RouterFailure.Status(503, reply.reason ?: "not waiting")
            (reply.requestId ?: named)?.let(::settle)
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
        runCatching { c.get<LivePrompt>(path(PROMPT, session.sessionId)) }
            .onSuccess(::track)
            .onFailure { if (it is CancellationException) throw it }
    }

    /** File what the router says is waiting where the screen, and [answer], look for it. */
    private fun track(p: LivePrompt) {
        val live = p.takeIf { it.pending && (it.requestId == null || it.requestId !in settled) }
        pendingPrompt = live?.takeIf { it.isPermission }
        when {
            live == null -> questionPrompt = null
            live.isQuestion -> questionPrompt = live
            // A permission request is the newest; a question may still be waiting under it.
            else -> Unit
        }
    }

    private fun settle(requestId: String) {
        settled.add(requestId)
        // A feed stays open for hours; nothing about it may grow without bound.
        while (settled.size > 64) settled.remove(settled.first())
        if (questionPrompt?.requestId == requestId) questionPrompt = null
        if (pendingPrompt?.requestId == requestId) pendingPrompt = null
    }

    /**
     * Allow the waiting tool, optionally taking one of the prompt's own standing offers,
     * or refuse it. [prompt] is the card that was tapped, and it is named by id: with more
     * than one waiting, "whatever is waiting" is not something the router will guess at.
     */
    suspend fun answerPermission(prompt: LivePrompt, allow: Boolean, suggestion: Int? = null, said: String) {
        val c = client ?: return
        val body = AnswerBody(
            session = session.sessionId,
            allow = allow,
            suggestion = suggestion,
            message = if (allow) null else "Declined from the phone.",
            requestId = prompt.requestId,
        )
        runCatching { c.post<AnswerBody, LiveReply>("/api/live/answer", body) }
            .onSuccess { r ->
                if (r.ok) {
                    prompt.requestId?.let(::settle)
                    pendingPrompt = null
                    notice = Notice(said, true)
                } else {
                    notice = Notice(r.reason ?: "Could not answer that prompt.", false)
                }
            }
            .onFailure { notice = Notice(RouterState.describe(it), false) }
        // Whatever happened, what is waiting now is the next thing to show: the prompt
        // after this one, or nothing — or, if this one was answered elsewhere first, not
        // this card again.
        loadPrompt()
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
        runCatching { c.get<SessionStats>(path(STATS, session.sessionId)) }
            .onSuccess { stats = it }
        runCatching { c.get<LiveOptions>("/api/live/options") }.onSuccess { options = it }
        session.cwd?.let { cwd ->
            val projectPath = c.query("/api/live/project", mapOf("cwd" to cwd))
            runCatching { c.get<ProjectStats>(projectPath) }.onSuccess { project = it }
        }
    }
}
