// What the router's API returns, as the app understands it.
//
// The router speaks snake_case, so every such key is spelled out with @SerialName rather
// than left to a naming strategy — `pace_30d` does not survive one the way `active_hours_left`
// does. Numbers SQLite sums arrive as floats and numbers it counts arrive as integers, so
// anything that is ever summed is a Double.

package ca.whnet.agentrouter.api

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class Health(val ok: Boolean, val pid: Int = 0, val version: String = "")

/** `GET /api/auth`. [remoteBind] is whether the router listens beyond loopback at all. */
@Serializable
data class AuthStatus(
    val configured: Boolean = false,
    @SerialName("remote_bind") val remoteBind: Boolean = false,
    @SerialName("from_loopback") val fromLoopback: Boolean = false,
    @SerialName("updated_at") val updatedAt: Long? = null,
)

// ---------------------------------------------------------------- live sessions

/** One row of `GET /api/live/sessions`: a transcript written in the last week. */
@Serializable
data class LiveSession(
    @SerialName("session_id") val sessionId: String,
    val cwd: String? = null,
    val project: String = "",
    @SerialName("git_branch") val gitBranch: String? = null,
    val title: String? = null,
    @SerialName("created_ms") val createdMs: Long? = null,
    @SerialName("last_activity_ms") val lastActivityMs: Long = 0,
    val active: Boolean = false,
    /** working | ask | idle | stalled | failed */
    val state: String = "idle",
    val archived: Boolean = false,
    /** Under the wrapper and answering on its socket: a message or a stop would arrive. */
    val wrapped: Boolean = false,
    val bytes: Long = 0,
) {
    val shortId: String get() = sessionId.take(8)

    /** The last path component of the working directory, or the project slug. */
    val projectName: String
        get() = cwd?.trimEnd('/')?.substringAfterLast('/')?.takeIf { it.isNotEmpty() } ?: project

    val label: String get() = title ?: shortId
}

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

/** The `status` event: what the session is doing, from the tail of its transcript. */
@Serializable
data class LiveStatus(
    /** ask | tool | idle | working */
    val state: String = "idle",
    val detail: String = "",
    @SerialName("quiet_ms") val quietMs: Long = 0,
    /** Messages typed while a turn ran, still waiting their turn. */
    val queued: Int? = null,
) {
    val isBusy: Boolean get() = state == "tool" || state == "working"
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
)

@Serializable
data class SwitchReply(
    val ok: Boolean = false,
    val model: String? = null,
    val effort: String? = null,
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
@Serializable data class AnswerBody(val session: String, val answers: Map<String, String>)
@Serializable data class SessionBody(val session: String)
@Serializable data class ArchiveBody(val session: String, val archived: Boolean)
@Serializable data class ModelBody(val session: String, val model: String)
@Serializable data class EffortBody(val session: String, val effort: String)
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
