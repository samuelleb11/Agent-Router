// What the router's API returns, as the app understands it.
//
// The router speaks snake_case, so every such key is spelled out with @SerialName rather
// than left to a naming strategy — `pace_30d` does not survive one the way `active_hours_left`
// does. Numbers SQLite sums arrive as floats and numbers it counts arrive as integers, so
// anything that is ever summed is a Double.

package ca.whnet.agentrouter.api

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

@Serializable
data class Health(val ok: Boolean, val pid: Int = 0, val version: String = "")

/** `GET /api/auth`. [remoteBind] is whether the router listens beyond loopback at all. */
@Serializable
data class AuthStatus(
    val configured: Boolean = false,
    @SerialName("access_tokens") val accessTokens: Int? = null,
    @SerialName("remote_bind") val remoteBind: Boolean = false,
    @SerialName("from_loopback") val fromLoopback: Boolean = false,
    @SerialName("updated_at") val updatedAt: Long? = null,
    /** `server` for a hosted router, `local` for one on a Mac; absent from an older router. */
    val mode: String? = null,
) {
    val isHosted: Boolean get() = mode == "server"
    val hasCredentials: Boolean get() = configured || (accessTokens ?: 0) > 0
    val credentialStatus: String get() = when {
        (accessTokens ?: 0) > 0 -> if (configured) "password and access tokens" else "access tokens"
        configured -> "password"
        else -> "not configured"
    }

    /**
     * Whether a device other than the router's own machine can reach it. A hosted router
     * always listens beyond loopback, and a router on a Mac does once Listen on your network
     * is on. A reply to a request that arrived from off the machine — as every request to an
     * https router does, through its ingress — shows it can, whatever else the reply says.
     *
     * What remains is a router that listens on loopback alone, answering a request that came
     * in through loopback: an emulator, or a tunnel on the Mac. This phone could not.
     */
    val reachableOffHost: Boolean get() = remoteBind || isHosted || !fromLoopback
}

// ---------------------------------------------------------------- project names

/**
 * How a project is named from its directory, the one rule every list in the app follows: the
 * directory's last segment, split on `/` and `\` alike, since a hosted router hears from Windows
 * machines, whose directories are `C:\Users\…\code\agent-router`. A trailing separator ends no
 * segment, and a path that mixes the two splits on both. The web console names a project the same
 * way (`projectName` in ui/src/data/projects.ts, `projectOf` in ui/src/model/session.ts; SE-C222).
 */
object ProjectPath {
    /** The last segment of [path], or null when it has none: absent, empty, or separators alone. */
    fun segment(path: String?): String? = path?.split('/', '\\')?.lastOrNull { it.isNotEmpty() }

    /**
     * A project's name in a list: its last segment; else the path as written, when it is only
     * separators; else "—" when there is no path, where the web says "(unresolved)".
     */
    fun name(path: String?): String = segment(path) ?: path?.ifEmpty { null } ?: "—"
}

// ---------------------------------------------------------------- live sessions

/**
 * One row of `GET /api/live/sessions`: a transcript written in the last week, or a session
 * whose process is running however long ago it last wrote.
 *
 * The first block is what every router has always sent, and it keeps its meaning — [state]
 * stays in the old vocabulary so builds of this app that predate the rest still read it.
 * Everything after [bytes] comes only from a router that watches the Claude Code process as
 * well as its transcript: whether the process is running at all, and what it is doing, in
 * words the router chose. All of it is optional. An older router sends none of it and the
 * app then behaves exactly as it did.
 */
@Serializable
data class LiveSession(
    @SerialName("session_id") val sessionId: String,
    val cwd: String? = null,
    val project: String = "",
    @SerialName("git_branch") val gitBranch: String? = null,
    val title: String? = null,
    @SerialName("created_ms") val createdMs: Long? = null,
    @SerialName("last_activity_ms") val lastActivityMs: Long = 0,
    /** Doing something now, or waiting on a person. (An older router: wrote in the last two minutes.) */
    val active: Boolean = false,
    /** working | ask | idle | stalled | failed */
    val state: String = "idle",
    val archived: Boolean = false,
    /** Under the wrapper and answering on its socket: a message or a stop would arrive. */
    val wrapped: Boolean = false,
    val client: String? = null,
    val hosted: Boolean? = null,
    val bytes: Long = 0,
    /** One of [Activity]'s values — or a newer one, which reads as neutral rather than failing. */
    val activity: String? = null,
    /** Whether the Claude Code process is running at all. Null when the router cannot tell. */
    val open: Boolean? = null,
    /** The router's own sentence for it: "Running Bash", "Waiting for your answer". */
    val detail: String? = null,
    /** When the current activity began, on the Mac's clock. */
    @SerialName("since_ms") val sinceMs: Long? = null,
    /** A closed session's end, as near as its transcript says. */
    @SerialName("ended_ms") val endedMs: Long? = null,
    /** `exit` when the process wrote its exit marker on the way out; `unknown` when it just stopped. */
    @SerialName("ended_how") val endedHow: String? = null,
    /** Closed with a turn still open: it died, or was killed, in the middle of something. */
    @SerialName("mid_turn") val midTurn: Boolean? = null,
    @SerialName("waiting_on") val waitingOn: WaitingOn? = null,
    val background: BackgroundWork? = null,
    /** busy | attention | warn | error | idle | closed: the router picks the colour so every client agrees on it. */
    val tone: String? = null,
    /** `registry` when the process said so itself, `transcript` when it was read from what it wrote. */
    val source: String? = null,
    /** When the router took this reading, on its own clock — what a duration is measured against. */
    @SerialName("as_of_ms") val asOfMs: Long? = null,
    /**
     * The machine the session runs on. Only a hosted router sends it, on every row of the
     * list it merges from its machines' agents; a router on a Mac never does.
     */
    val machine: SessionMachine? = null,
    /**
     * The git repository the working directory belongs to, as the router spells it: its
     * `origin` remote, else its first, canonical — `gitlab.example.com/acme/ui-kit-web`.
     * Absent when the directory has no remote, or the machine's agent predates it.
     */
    val repository: String? = null,
) {
    val shortId: String get() = sessionId.removePrefix("codex:").take(8)
    val isCodex: Boolean get() = client == "codex" || sessionId.startsWith("codex:")
    val canControl: Boolean get() = !isCodex && wrapped
    val canSend: Boolean get() = !isCodex
    val canAskAside: Boolean get() = !isCodex
    val clientName: String? get() = when { isCodex -> "Codex"; client == "kimi-code" -> "Kimi Code"; else -> null }
    val watchedReason: String? get() = if (isCodex) "This Codex session is watched from its local transcript. Send messages and answer approvals in the terminal or IDE that owns it." else null

    /**
     * The repository's last segment, so a repository's clones and worktrees on one machine are
     * one project; without one, the last segment of the working directory, a Windows path's
     * included, else the router's own name for the project (SE-C23). The web's `projectOf`.
     */
    val projectName: String
        get() = ProjectPath.segment(repository) ?: ProjectPath.segment(cwd) ?: project.ifEmpty { "—" }

    /**
     * What the router knows the project by, to ask for its totals: the repository, else the
     * working directory. Null when the row has neither.
     */
    val projectKey: String?
        get() = repository?.takeIf { it.isNotEmpty() } ?: cwd?.takeIf { it.isNotEmpty() }

    val label: String get() = title ?: shortId

    /** The name of the machine it runs on, when the router gave one. */
    val machineLabel: String? get() = machine?.label?.trim()?.takeIf { it.isNotEmpty() }

    /** The process has gone: nothing more happens here unless the session is resumed. */
    val isClosed: Boolean get() = Activity.isClosed(activity, tone, open)

    /** Blocked on a person. Only a router that sends [activity] says so on the row. */
    val needsYou: Boolean get() = activity == Activity.WAITING

    /** At work — see [Activity.working]. Only a router that sends [activity] says so. */
    val isWorking: Boolean get() = activity?.let(Activity::working) ?: false
}

/**
 * The router's `activity` vocabulary, and the questions every client asks of it.
 *
 * Strings rather than an enum on purpose: a router newer than this app may add a value, and
 * one this app does not know must come out neutral — gray, no Stop — never as a failed
 * decode that blanks the whole list.
 */
object Activity {
    const val CLOSED = "closed"
    const val IDLE = "idle"
    const val GENERATING = "generating"
    const val TOOL = "tool"
    const val WAITING = "waiting"
    const val BACKGROUND = "background"
    const val FAILED = "failed"
    const val STALLED = "stalled"

    /**
     * What a Stop can interrupt: the model mid-reply, a tool running, a turn gone quiet.
     * Not [BACKGROUND]. The turn is already over there, and what an interrupt does to agents
     * and workflows still running on their own has never been measured — it may throw away
     * the results they are about to hand back. The router refuses that Stop for older
     * builds, which still show the button for the legacy `working`.
     */
    private val STOPPABLE = setOf(GENERATING, TOOL, STALLED)

    /**
     * At work, as the router's session counts say it: a turn running, background work a
     * finished turn left behind, or a turn gone quiet — the process still says it is busy.
     * Not [WAITING]: a session blocked on you is counted apart, since it is the one that
     * needs a thumb.
     */
    private val WORKING = setOf(GENERATING, TOOL, BACKGROUND, STALLED)

    /** Busy, or waiting on a person: what the router's `active` means once it sends this. */
    private val ACTIVE = WORKING + WAITING

    fun stoppable(activity: String): Boolean = activity in STOPPABLE

    fun working(activity: String): Boolean = activity in WORKING

    fun active(activity: String): Boolean = activity in ACTIVE

    /** Any one of the three is enough: the router sends them together, but only one is needed to know. */
    fun isClosed(activity: String?, tone: String?, open: Boolean?): Boolean =
        activity == CLOSED || tone == "closed" || open == false
}

/** What a waiting session is blocked on. */
@Serializable
data class WaitingOn(
    /** permission | question | plan | dialog */
    val kind: String? = null,
    val tool: String? = null,
    /** The wrapper's id for the prompt, when the wrapper holds it: what an answer names. */
    @SerialName("request_id") val requestId: String? = null,
    @SerialName("tool_use_id") val toolUseId: String? = null,
    val summary: String? = null,
)

/** Work still running after the turn itself ended. */
@Serializable
data class BackgroundWork(val agents: Int = 0, val workflows: Int = 0, val shells: Int = 0)

/**
 * Which machine a session runs on, as a hosted router names it. [id] is the machine's own,
 * and survives a rename; [label] is what to call it — its host name, unless its agent was
 * given another. Both default, so a row that leaves one out still decodes.
 */
@Serializable
data class SessionMachine(val id: String = "", val label: String = "")

/** One rendered row of the feed, as the stream projects it. */
@Serializable
data class LiveItem(
    val t: String? = null,
    /** user | assistant | system */
    val role: String = "system",
    /** text | thinking | tool_use | tool_result | ask | answer | interrupt | command | other */
    val kind: String = "other",
    val name: String? = null,
    val text: String? = null,
    val ask: AskPayload? = null,
    @SerialName("dur_ms") val durMs: Long? = null,
    val sidechain: Boolean = false,
)

/** The AskUserQuestion tool's own input, so the real options can be offered. */
@Serializable
data class AskPayload(val questions: List<AskQuestion> = emptyList())

@Serializable
data class AskQuestion(
    val question: String,
    val header: String? = null,
    @SerialName("multiSelect") val multiSelect: Boolean? = null,
    val options: List<AskOption>? = null,
) {
    val title: String get() = header ?: question
    val isMulti: Boolean get() = multiSelect ?: false
}

@Serializable
data class AskOption(val label: String, val description: String? = null)

/**
 * The `status` event: what the session is doing.
 *
 * [state], [detail] and [quietMs] are what every router sends. The rest comes only from a
 * router that watches the process as well as the transcript, and is optional for the same
 * reason as on [LiveSession].
 */
@Serializable
data class LiveStatus(
    /** ask | tool | idle | working | failed — the old vocabulary, kept for older builds */
    val state: String = "idle",
    val detail: String = "",
    /** How long it has been doing what [detail] says. */
    @SerialName("quiet_ms") val quietMs: Long = 0,
    /** Messages typed while a turn ran, still waiting their turn. */
    val queued: Int? = null,
    val activity: String? = null,
    val open: Boolean? = null,
    @SerialName("since_ms") val sinceMs: Long? = null,
    @SerialName("as_of_ms") val asOfMs: Long? = null,
    @SerialName("waiting_on") val waitingOn: WaitingOn? = null,
    val background: BackgroundWork? = null,
    val tone: String? = null,
    @SerialName("ended_ms") val endedMs: Long? = null,
) {
    /**
     * Whether a Stop would interrupt anything. From [activity] when the router sends it —
     * so background work, which the legacy [state] also calls `working`, offers no Stop —
     * and from [state], as it always was, when it does not.
     */
    val isBusy: Boolean
        get() = activity?.let(Activity::stoppable) ?: (state == "tool" || state == "working")

    val isClosed: Boolean get() = Activity.isClosed(activity, tone, open)

    /**
     * Busy or waiting on you, so the header's dot wears its halo. Never from an older
     * router: its header never had one, and a guess from [state] would be the old guesses.
     */
    val isActive: Boolean get() = activity?.let(Activity::active) ?: false
}

/** What the live endpoints answer with — success and failure share the shape. */
@Serializable
data class LiveReply(
    val ok: Boolean = false,
    val via: String? = null,
    /**
     * True when the message went by the peer socket, so it shows up in the session as
     * coming from another Claude rather than from the person who typed it.
     */
    val framed: Boolean? = null,
    val reason: String? = null,
    @SerialName("request_id") val requestId: String? = null,
)

@Serializable
data class SessionStats(
    val model: String? = null,
    val effort: String? = null,
    /** The permission mode in force, as the session last stamped it on a turn. */
    val mode: String? = null,
    val input: Long = 0,
    val output: Long = 0,
    @SerialName("cache_read") val cacheRead: Long = 0,
    @SerialName("cache_write") val cacheWrite: Long = 0,
    val cost: Double? = null,
    val total: Long = 0,
    val turns: Long = 0,
)

@Serializable
data class ProjectStats(
    val sessions: Int = 0,
    val models: List<String> = emptyList(),
    val total: Long = 0,
    val output: Long = 0,
    @SerialName("cache_read") val cacheRead: Long = 0,
    val turns: Long = 0,
    val cost: Double? = null,
)

/** `GET /api/live/options`: what a session can be switched to. */
@Serializable
data class LiveOptions(
    val models: List<String> = emptyList(),
    val efforts: List<String> = emptyList(),
    /** Empty against an older router, which sends no modes. */
    val modes: List<LiveMode> = emptyList(),
)

/**
 * A permission mode and the wording for it. The label comes from the router so the three
 * clients cannot drift apart on what to call these.
 */
@Serializable
data class LiveMode(val value: String, val label: String) {
    /** This one stops the session asking about anything, so every client confirms first. */
    val isBypass: Boolean get() = value == "bypassPermissions"
}

/**
 * `GET /api/live/prompt`: what the session is blocked on. A question carries its own
 * options and is answered from the feed; anything else is a request for permission.
 */
@Serializable
data class LivePrompt(
    val pending: Boolean = false,
    @SerialName("request_id") val requestId: String? = null,
    val tool: String? = null,
    val question: Boolean = false,
    @SerialName("display_name") val displayName: String? = null,
    val description: String? = null,
    val reason: String? = null,
    val suggestions: List<PromptSuggestion> = emptyList(),
    /**
     * How many prompts are waiting in all; this one is the newest. An older router sends
     * no count — it only ever knew one — and it also answers "whatever is newest" no matter
     * which request id it is given, which is why an answer checks for this.
     */
    val count: Int? = null,
    /**
     * The tool's own input, as the editor was asked about it. Kept as raw JSON: a
     * permission request carries whatever its tool takes, and no one shape decodes them
     * all — a strict one here would fail the whole prompt, and with it the card.
     */
    val input: JsonElement? = null,
) {
    val isPermission: Boolean get() = pending && !question
    val isQuestion: Boolean get() = pending && question

    /**
     * The questions a question prompt asks, by their text and in their order. Null when its
     * input does not list them, as from a router that sends no input.
     */
    val questionTexts: List<String?>?
        get() = ((input as? JsonObject)?.get("questions") as? JsonArray)
            ?.map { ((it as? JsonObject)?.get("question") as? JsonPrimitive)?.contentOrNull }

    /**
     * Whether this is the question a form with [questions] was drawn for: the same
     * questions, in the same order — the web view's rule, word for word. The feed keeps
     * every form it has drawn, so one answered in the editor stays on screen here while the
     * model asks the next; its answers must not be handed to that next one. A prompt that
     * does not list its questions is taken at its word.
     */
    fun asks(questions: List<String>): Boolean = questionTexts?.let { it == questions } ?: true
}

/**
 * One of the prompt's own offers. Only ever these: the router refuses an index it did
 * not offer, and the editor on the far end drops anything it did not suggest.
 */
@Serializable
data class PromptSuggestion(val index: Int, val label: String)

@Serializable
data class SwitchReply(
    val ok: Boolean = false,
    val model: String? = null,
    val effort: String? = null,
    val mode: String? = null,
    val pending: Boolean? = null,
    val notices: List<String> = emptyList(),
    val reason: String? = null,
)

@Serializable
data class AsideReply(
    val ok: Boolean = false,
    val answer: String? = null,
    val fork: String? = null,
    val cost: Double? = null,
    @SerialName("took_ms") val tookMs: Long? = null,
    val reason: String? = null,
)

/** What the write endpoints answer: `{ok}` or `{error}` or `{ok:false, reason}`. */
@Serializable
data class OkReply(
    val ok: Boolean? = null,
    val error: String? = null,
    val reason: String? = null,
    val id: String? = null,
) {
    val problem: String? get() = error ?: reason ?: if (ok == false) "refused" else null
}

// ---------------------------------------------------------------- request bodies

@Serializable data class SendBody(val session: String, val text: String)
/**
 * `answers` alone is how a question is answered, and how this app always spoke. The rest
 * is a permission request: let it through, optionally taking one of the offers the
 * prompt itself made, or refuse it with a reason.
 */
@Serializable data class AnswerBody(
    val session: String,
    val answers: Map<String, String> = emptyMap(),
    val allow: Boolean? = null,
    val suggestion: Int? = null,
    val message: String? = null,
    /**
     * Which prompt this answers, by the id `/api/live/prompt` gave it. Left out, the router
     * answers only when exactly one prompt is waiting — with several, it will not guess
     * which one was meant. Null is not sent at all, so the body is the old one.
     */
    @SerialName("request_id") val requestId: String? = null,
)
@Serializable data class SessionBody(val session: String)
@Serializable data class ArchiveBody(val session: String, val archived: Boolean)
@Serializable data class ModelBody(val session: String, val model: String)
@Serializable data class EffortBody(val session: String, val effort: String)
@Serializable data class ModeBody(val session: String, val mode: String)
@Serializable data class AsideBody(val session: String, val text: String)

// ---------------------------------------------------------------- notifications

/** `/api/push/pending`: the prompts the router says are still waiting for a person. */
@Serializable
data class PendingReply(val waiting: List<String> = emptyList())

@Serializable
data class PushRegisterBody(
    val token: String,
    val name: String,
    /** `fcm` here, always. The router keeps Apple's phones under `apns`. */
    val platform: String,
    val environment: String,
    @SerialName("router_ref") val routerRef: String,
)

@Serializable
data class PushStatus(
    /** Apple's key. Kept at the top level so the dashboard's own form is unchanged. */
    val configured: Boolean = false,
    @SerialName("key_file") val keyFile: String? = null,
    @SerialName("last_error") val lastError: String? = null,
    @SerialName("last_sent") val lastSent: Sent? = null,
    val fcm: Fcm = Fcm(),
    @SerialName("any_configured") val anyConfigured: Boolean = false,
    val devices: List<Device> = emptyList(),
) {
    @Serializable
    data class Fcm(
        val configured: Boolean = false,
        @SerialName("project_id") val projectId: String? = null,
        @SerialName("client_email") val clientEmail: String? = null,
        @SerialName("private_key_id") val privateKeyId: String? = null,
        @SerialName("key_file") val keyFile: String? = null,
        @SerialName("last_error") val lastError: String? = null,
    )

    @Serializable
    data class Device(
        @SerialName("token_tail") val tokenTail: String = "",
        val platform: String = "apns",
        val name: String? = null,
        val environment: String = "",
        @SerialName("router_ref") val routerRef: String? = null,
        @SerialName("last_seen_at") val lastSeenAt: Long = 0,
        @SerialName("last_result") val lastResult: String? = null,
    )

    @Serializable data class Sent(val at: Long = 0, val what: String = "")
}

@Serializable
data class PushTestReply(
    val ok: Boolean? = null,
    val results: List<Line> = emptyList(),
    val error: String? = null,
) {
    @Serializable
    data class Line(val device: String = "", val ok: Boolean = false, val error: String? = null)
}
