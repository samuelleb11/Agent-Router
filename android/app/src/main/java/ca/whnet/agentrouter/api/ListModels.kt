// The dashboard's drill-down lists, the wrapper's state, and the sign-in flow.

package ca.whnet.agentrouter.api

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

/**
 * One row of `GET /api/projects`. [project] is the project's key: its git repository, else
 * its directory — from an older router, always the directory. Unresolved, it decodes to "".
 */
@Serializable
data class ProjectRow(
    val project: String = "",
    val sessions: Double = 0.0,
    val requests: Double = 0.0,
    @SerialName("prompt_tokens") val promptTokens: Double = 0.0,
    val cost: Double = 0.0,
    @SerialName("output_tokens") val outputTokens: Double = 0.0,
    @SerialName("cache_read_tokens") val cacheReadTokens: Double = 0.0,
    @SerialName("subagent_tokens") val subagentTokens: Double = 0.0,
    val errors: Double = 0.0,
    val models: String? = null,
    @SerialName("last_seen") val lastSeen: Long? = null,
    /** The git repository it is, when it is one; absent from an older router. */
    val repository: String? = null,
    /** The directories the project was worked in, on whichever machines; empty from an older router. */
    val directories: List<ProjectDirectory> = emptyList(),
) {
    /** The project by name — its key's last segment, a Windows machine's directory included ([ProjectPath]). */
    val name: String get() = ProjectPath.name(project)
}

/** A directory a project was worked in, the machine it is on, and the git remotes it has. */
@Serializable
data class ProjectDirectory(
    val cwd: String = "",
    val machine: SessionMachine? = null,
    val remotes: List<GitRemote> = emptyList(),
)

/** One git remote of a directory: `origin`, its URL, and the repository the router reads it as. */
@Serializable
data class GitRemote(
    val name: String = "",
    val url: String = "",
    val repository: String? = null,
)

/**
 * A session as the proxy saw it — distinct from [LiveSession], which is a transcript. [project]
 * is its project's key, as [ProjectRow.project] is.
 */
@Serializable
data class ProxySessionRow(
    val id: String = "",
    val project: String? = null,
    /** The directory it ran in, its repository, and its machine; each absent from an older router. */
    val cwd: String? = null,
    val repository: String? = null,
    val machine: SessionMachine? = null,
    @SerialName("git_branch") val gitBranch: String? = null,
    val entrypoint: String? = null,
    @SerialName("started_at") val startedAt: Long = 0,
    @SerialName("last_seen_at") val lastSeenAt: Long = 0,
    val requests: Double = 0.0,
    @SerialName("prompt_tokens") val promptTokens: Double = 0.0,
    val cost: Double = 0.0,
    @SerialName("output_tokens") val outputTokens: Double = 0.0,
    @SerialName("subagent_tokens") val subagentTokens: Double = 0.0,
    val errors: Double = 0.0,
    val subagents: Double = 0.0,
    val models: String? = null,
) {
    val shortId: String get() = id.take(8)
    val projectName: String get() = ProjectPath.name(project)

    /** `claude-cli/2.1.269 (external, claude-vscode, …)` → `VS Code`; the CLI otherwise. */
    val client: String
        get() = when {
            entrypoint == null -> ""
            entrypoint.contains("vscode") -> "VS Code"
            entrypoint.contains("claude-cli") -> "CLI"
            else -> entrypoint.take(20)
        }
}

@Serializable
data class RequestRow(
    val id: Long = 0,
    @SerialName("request_id") val requestId: String? = null,
    @SerialName("session_id") val sessionId: String? = null,
    val project: String? = null,
    val model: String? = null,
    val status: Int? = null,
    @SerialName("ttfb_ms") val ttfbMs: Long? = null,
    @SerialName("duration_ms") val durationMs: Long? = null,
    @SerialName("stop_reason") val stopReason: String? = null,
    @SerialName("is_sidechain") val isSidechain: Int? = null,
    @SerialName("agent_id") val agentId: String? = null,
    @SerialName("started_at") val startedAt: Long = 0,
    val error: String? = null,
    @SerialName("prompt_tokens") val promptTokens: Double? = null,
    @SerialName("input_tokens") val inputTokens: Double? = null,
    @SerialName("output_tokens") val outputTokens: Double? = null,
    @SerialName("thinking_tokens") val thinkingTokens: Double? = null,
    val cost: Double? = null,
    @SerialName("cache_read_tokens") val cacheReadTokens: Double? = null,
    @SerialName("cache_write_tokens") val cacheWriteTokens: Double? = null,
    /** Comma-separated tool names, in call order. */
    val tools: String? = null,
    /** `component:est_tokens` pairs, comma-separated. */
    val context: String? = null,
) {
    val isError: Boolean get() = (status ?: 0) >= 400 || error != null
    val isSubagent: Boolean get() = isSidechain == 1

    /** The project it was sent from, by name, a Windows machine's included ([ProjectPath]). */
    val projectName: String get() = ProjectPath.name(project)

    val toolList: List<String>
        get() = (tools ?: "").split(",").map { it.trim() }.filter { it.isNotEmpty() }
    val contextSplit: List<Pair<String, Double>>
        get() = (context ?: "").split(",").mapNotNull { pair ->
            val p = pair.split(":", limit = 2)
            if (p.size != 2) return@mapNotNull null
            val v = p[1].toDoubleOrNull() ?: return@mapNotNull null
            p[0] to v
        }
}

/** `GET /api/tools`: what the assistant called, and what came back. */
@Serializable
data class ToolsPayload(
    val produced: List<Produced> = emptyList(),
    val called: List<Called> = emptyList(),
) {
    @Serializable
    data class Produced(
        val name: String? = null,
        val results: Double = 0.0,
        val chars: Double = 0.0,
        @SerialName("est_tokens") val estTokens: Double = 0.0,
        @SerialName("max_tokens") val maxTokens: Double = 0.0,
        @SerialName("est_cost_once") val estCostOnce: Double = 0.0,
        val errors: Double = 0.0,
    )

    @Serializable
    data class Called(
        val name: String? = null,
        val calls: Double = 0.0,
        val requests: Double = 0.0,
        @SerialName("subagent_calls") val subagentCalls: Double = 0.0,
    )
}

/** `GET /api/setup/shim`: whether each Claude Code extension on the Mac is wrapped. */
@Serializable
data class ShimStatus(
    val available: Boolean = false,
    val installed: Int = 0,
    val targets: List<Target> = emptyList(),
    /** Whether the router wraps new extension versions on its own. */
    val auto: Boolean? = null,
    @SerialName("auto_log") val autoLog: List<Event> = emptyList(),
) {
    @Serializable
    data class Target(
        @SerialName("extension") val ext: String = "",
        val path: String = "",
        /** not_installed | installed | inconsistent */
        val state: String = "",
    )

    @Serializable
    data class Event(val at: Long = 0, val what: String = "")

    /** The newest extension, when sessions it starts would be view-only. */
    val newestUnwrapped: Target?
        get() = targets.lastOrNull()?.takeIf { it.state != "installed" }
}

@Serializable data class ShimBody(val path: String, val action: String)
@Serializable data class ShimAutoBody(val enabled: Boolean)

/** `GET /api/login/status`: the `claude setup-token` flow the router drives. */
@Serializable
data class LoginStatus(
    /** starting | awaiting_sign_in | done | failed — absent when the start itself failed. */
    val phase: String? = null,
    val url: String? = null,
    val label: String? = null,
    @SerialName("account_id") val accountId: String? = null,
    val email: String? = null,
    val error: String? = null,
    /** The last 400 characters of the CLI log; the token is never returned. */
    val tail: String? = null,
)

@Serializable data class LoginStartBody(val label: String)
@Serializable data class AddAccountBody(val label: String, val source: String, val token: String? = null)
@Serializable data class UsageTokenBody(
    @SerialName("account_id") val accountId: String,
    val token: String? = null,
    val clear: Boolean? = null,
)
