// What the router's read API returns, as the app understands it.
//
// Every key is spelled out: the router speaks snake_case, and a name like `pace_30d`
// does not survive automatic conversion the way `active_hours_left` does. Numbers that
// SQLite sums arrive as floats and numbers it counts arrive as integers, so anything
// that is ever summed is a Double here.

import Foundation

struct Health: Decodable, Sendable {
    let ok: Bool
    let pid: Int
    let version: String
}

/// `GET /api/auth`. `remoteBind` is whether the router listens beyond loopback at all.
struct AuthStatus: Decodable, Sendable {
    let configured: Bool
    let remoteBind: Bool
    let fromLoopback: Bool
    let updatedAt: Int?

    enum CodingKeys: String, CodingKey {
        case configured, updatedAt = "updated_at", remoteBind = "remote_bind", fromLoopback = "from_loopback"
    }
}

/// One row of `GET /api/accounts`. `ready` says whether the router could produce a
/// credential for it right now; `detail` says what kind, or why not.
struct Account: Decodable, Sendable, Identifiable, Hashable {
    let id: String
    let label: String
    let email: String?
    let orgName: String?
    let subscriptionType: String?
    let active: Int
    let source: String?
    let ready: Bool?
    let detail: String?
    let expiresAt: Int?
    let hasToken: Bool?
    let usageKey: Int?
    let createdAt: Int?

    var isActive: Bool { active == 1 }
    var title: String { email ?? label }
    var hasUsageKey: Bool { (usageKey ?? 0) == 1 }
    var expiry: Date? { expiresAt.flatMap { $0 > 0 ? Date(timeIntervalSince1970: TimeInterval($0) / 1000) : nil } }

    enum CodingKeys: String, CodingKey {
        case id, label, email, active, source, ready, detail
        case orgName = "org_name", subscriptionType = "subscription_type"
        case expiresAt = "expires_at", hasToken = "has_token", usageKey = "usage_key", createdAt = "created_at"
    }
}

/// `GET /api/quota`. Utilisation is 0–100 here (the store keeps 0–1; the endpoint
/// scales to match Anthropic's own usage API). `resetsAt` is either ISO 8601 or
/// `@<epoch seconds>` when it came from response headers.
struct Quota: Decodable, Sendable {
    struct Window: Decodable, Sendable {
        let utilization: Double?
        let resetsAt: String?

        enum CodingKeys: String, CodingKey { case utilization, resetsAt = "resets_at" }

        var fraction: Double? { utilization.map { min(1, max(0, $0 / 100)) } }
        var resetDate: Date? {
            guard let s = resetsAt else { return nil }
            if s.hasPrefix("@"), let epoch = Double(s.dropFirst()) { return Date(timeIntervalSince1970: epoch) }
            return ISO8601DateFormatter().date(from: s)
        }
    }

    let fiveHour: Window?
    let sevenDay: Window?
    let severity: String?
    let source: String?
    let accountId: String?
    let seenAt: Int?

    enum CodingKeys: String, CodingKey {
        case severity, source
        case fiveHour = "five_hour", sevenDay = "seven_day", accountId = "account_id", seenAt = "seen_at"
    }
}

/// `GET /api/forecast`: every account's windows projected to their end, plus a rollup.
struct Forecast: Decodable, Sendable {
    let generatedAt: Int
    let accounts: [ForecastAccount]
    let global: Global

    enum CodingKeys: String, CodingKey { case accounts, global, generatedAt = "generated_at" }

    struct ForecastAccount: Decodable, Sendable, Identifiable, Hashable {
        let id: String
        let label: String
        let email: String?
        let plan: String?
        let active: Int
        let usage: Usage
        let week: Usage
        let seenAt: Int?
        let status: String?
        let pool: Int?
        let fiveHour: Window?
        let sevenDay: Window?

        var isActive: Bool { active == 1 }
        var title: String { email ?? label }

        enum CodingKeys: String, CodingKey {
            case id, label, email, plan, active, usage, week, status, pool
            case seenAt = "seen_at", fiveHour = "five_hour", sevenDay = "seven_day"
        }
    }

    struct Usage: Decodable, Sendable, Hashable {
        let requests: Double?
        let promptTokens: Double?
        let outputTokens: Double?
        let cost: Double?
        let lastSeen: Int?

        enum CodingKeys: String, CodingKey {
            case requests, cost
            case promptTokens = "prompt_tokens", outputTokens = "output_tokens", lastSeen = "last_seen"
        }
    }

    /// One rate-limit window. Three shapes share it: `null` (no reading), an expired
    /// stub (`expired`, `reset_at`, `seen_at`), and the full projection.
    struct Window: Decodable, Sendable, Hashable {
        let expired: Bool?
        let used: Double?
        let resetAt: Int?
        let windowStart: Int?
        let burnPerActiveHour: Double?
        let activeHoursLeft: Double?
        let dutyFraction: Double?
        /// The same figure with the spread of the account's days either side. Absent
        /// from a router that predates the band.
        let duty: Duty?
        let exhaustsAt: Int?
        let exhaustsAtEarly: Int?
        let exhaustsAtLate: Int?
        let resetsFirst: Bool?
        /// `crosses`, `tight` or `resets-first`. Absent from an older router, where
        /// `resetsFirst` is the whole answer.
        let verdict: String?
        let projectedEndUtil: Double?
        let projectedEndUtilLow: Double?
        let projectedEndUtilHigh: Double?
        let basis: String?
        let measuredActiveHours: Double?
        let readings: Int?
        let seenAt: Int?
        let series: [Point]?

        var isExpired: Bool { expired ?? false }
        var resetDate: Date? { resetAt.map { Date(timeIntervalSince1970: TimeInterval($0)) } }
        var startDate: Date? { windowStart.map { Date(timeIntervalSince1970: TimeInterval($0)) } }
        var exhaustDate: Date? { exhaustsAt.map { Date(timeIntervalSince1970: TimeInterval($0) / 1000) } }
        /// What the band says, falling back to the boolean an older router sends.
        var call: String { verdict ?? (exhaustsAt != nil ? "crosses" : "resets-first") }
        /// Whether there is a spread worth showing at all — a session window has none.
        var hasBand: Bool { duty.map { $0.low < $0.high } ?? false }

        enum CodingKeys: String, CodingKey {
            case expired, used, basis, readings, series
            case resetAt = "reset_at", windowStart = "window_start"
            case burnPerActiveHour = "burn_per_active_hour", activeHoursLeft = "active_hours_left"
            case dutyFraction = "duty_fraction", exhaustsAt = "exhausts_at", resetsFirst = "resets_first"
            case projectedEndUtil = "projected_end_util", measuredActiveHours = "measured_active_hours"
            case seenAt = "seen_at", duty, verdict
            case exhaustsAtEarly = "exhausts_at_early", exhaustsAtLate = "exhausts_at_late"
            case projectedEndUtilLow = "projected_end_util_low"
            case projectedEndUtilHigh = "projected_end_util_high"
        }
    }

    /// How much of the clock goes to work, as a range. `mid` is the estimate; the edges
    /// are the spread of the days on record, which is why a tight verdict is tight.
    struct Duty: Decodable, Sendable, Hashable {
        let low: Double
        let mid: Double
        let high: Double
    }

    struct Point: Decodable, Sendable, Hashable {
        let t: Double
        let u: Double
        var date: Date { Date(timeIntervalSince1970: t / 1000) }
    }

    struct Global: Decodable, Sendable {
        let requests: Double
        let promptTokens: Double
        let outputTokens: Double
        let cost: Double
        let accounts: Int
        let combinedHeadroom: Headroom
        let pace30d: Pace

        enum CodingKeys: String, CodingKey {
            case requests, cost, accounts
            case promptTokens = "prompt_tokens", outputTokens = "output_tokens"
            case combinedHeadroom = "combined_headroom", pace30d = "pace_30d"
        }
    }

    struct Headroom: Decodable, Sendable {
        let activeHoursLeft: Double?
        let pools: Int
        let firstToExhaust: First?

        enum CodingKeys: String, CodingKey {
            case pools, activeHoursLeft = "active_hours_left", firstToExhaust = "first_to_exhaust"
        }
    }

    struct First: Decodable, Sendable {
        let accountId: String
        let window: String
        let at: Int
        let inMs: Int

        enum CodingKeys: String, CodingKey { case window, at, accountId = "account_id", inMs = "in_ms" }
    }

    struct Pace: Decodable, Sendable {
        let cost: Double
        let promptTokens: Double
        let basis: String

        enum CodingKeys: String, CodingKey { case cost, basis, promptTokens = "prompt_tokens" }
    }
}

struct ActivateResponse: Decodable, Sendable {
    let ok: Bool
    let active: String
}

// MARK: - Live sessions

/// One row of `GET /api/live/sessions`: a transcript written in the last week.
struct LiveSession: Decodable, Sendable, Identifiable, Hashable {
    let sessionId: String
    let cwd: String?
    let project: String
    let gitBranch: String?
    let title: String?
    let createdMs: Int?
    let lastActivityMs: Int
    let active: Bool
    /// working | ask | idle | stalled | failed
    let state: String
    let archived: Bool
    /// Under the wrapper and answering on its socket: a message or a stop would arrive.
    let wrapped: Bool
    let bytes: Int

    var id: String { sessionId }
    var shortId: String { String(sessionId.prefix(8)) }
    /// The last path component of the working directory, or the project slug.
    var projectName: String {
        if let cwd, let last = cwd.split(separator: "/").last, !last.isEmpty { return String(last) }
        return project
    }
    var label: String { title ?? shortId }
    var lastActivity: Date { Date(timeIntervalSince1970: TimeInterval(lastActivityMs) / 1000) }

    enum CodingKeys: String, CodingKey {
        case cwd, project, title, active, state, archived, wrapped, bytes
        case sessionId = "session_id", gitBranch = "git_branch", createdMs = "created_ms"
        case lastActivityMs = "last_activity_ms"
    }
}

/// One rendered row of the feed, as the stream projects it.
struct LiveItem: Decodable, Sendable, Hashable {
    let t: String?
    /// user | assistant | system
    let role: String
    /// text | thinking | tool_use | tool_result | ask | answer | other
    let kind: String
    let name: String?
    let text: String?
    let ask: AskPayload?
    let durMs: Int?
    let sidechain: Bool

    enum CodingKeys: String, CodingKey {
        case t, role, kind, name, text, ask, sidechain
        case durMs = "dur_ms"
    }
}

/// The AskUserQuestion tool's own input, so the real options can be offered.
struct AskPayload: Decodable, Sendable, Hashable {
    let questions: [AskQuestion]
}

struct AskQuestion: Decodable, Sendable, Hashable, Identifiable {
    let question: String
    let header: String?
    let multiSelect: Bool?
    let options: [AskOption]?

    var id: String { question }
    var title: String { header ?? question }
    var isMulti: Bool { multiSelect ?? false }
}

struct AskOption: Decodable, Sendable, Hashable, Identifiable {
    let label: String
    let description: String?
    var id: String { label }
}

/// The `status` event: what the session is doing, from the tail of its transcript.
struct LiveStatus: Decodable, Sendable, Equatable {
    /// ask | tool | thinking | idle | working
    let state: String
    let detail: String
    let quietMs: Int
    /// Messages typed while a turn ran, still waiting their turn.
    let queued: Int?

    var isBusy: Bool { state == "tool" || state == "working" }

    enum CodingKeys: String, CodingKey { case state, detail, queued, quietMs = "quiet_ms" }
}

/// What the live endpoints answer with — success and failure share the shape.
struct LiveReply: Decodable, Sendable {
    let ok: Bool
    let via: String?
    /// True when the message went by the peer socket and will show up as coming from
    /// another Claude rather than from the person who typed it.
    let framed: Bool?
    let reason: String?
    let requestId: String?

    enum CodingKeys: String, CodingKey { case ok, via, framed, reason, requestId = "request_id" }
}

struct SessionStats: Decodable, Sendable {
    let model: String?
    let effort: String?
    let input: Int
    let output: Int
    let cacheRead: Int
    let cacheWrite: Int
    let cost: Double?
    let total: Int
    let turns: Int

    enum CodingKeys: String, CodingKey {
        case model, effort, input, output, cost, total, turns
        case cacheRead = "cache_read", cacheWrite = "cache_write"
    }
}

struct ProjectStats: Decodable, Sendable {
    let sessions: Int
    let total: Int
    let output: Int
    let cacheRead: Int
    let turns: Int
    let cost: Double?

    enum CodingKeys: String, CodingKey { case sessions, total, output, turns, cost, cacheRead = "cache_read" }
}

/// `GET /api/setup/shim`: whether each Claude Code extension on the Mac is wrapped.
struct ShimStatus: Decodable, Sendable {
    struct Target: Decodable, Sendable {
        let ext: String
        let path: String
        /// not_installed | installed | inconsistent
        let state: String
        enum CodingKeys: String, CodingKey { case ext = "extension", path, state }
    }
    struct Event: Decodable, Sendable, Identifiable {
        let at: Int
        let what: String
        var id: Int { at }
    }
    let available: Bool
    let installed: Int
    let targets: [Target]
    /// Whether the router wraps new extension versions on its own.
    let auto: Bool?
    let autoLog: [Event]?

    enum CodingKeys: String, CodingKey { case available, installed, targets, auto, autoLog = "auto_log" }

    /// The newest extension, when sessions it starts would be view-only.
    var newestUnwrapped: Target? {
        guard let newest = targets.last else { return nil }
        return newest.state == "installed" ? nil : newest
    }
}

// MARK: - Dashboard: usage in scope

/// `GET /api/summary`, scoped the way the dashboard scopes it. Everything SQLite sums
/// is a Double; timestamps are milliseconds.
struct Summary: Decodable, Sendable {
    let totals: Totals
    let context: [ContextRow]
    let models: [ModelRow]
    let threads: [ThreadRow]
    let latency: Latency?
    let stopReasons: [StopReason]
    let byAccount: [AccountSplitRow]
    let timeline: Timeline
    let quotaHistory: [QuotaPoint]

    enum CodingKeys: String, CodingKey {
        case totals, context, models, threads, latency, timeline
        case stopReasons = "stop_reasons", byAccount = "by_account", quotaHistory = "quota_history"
    }

    struct Totals: Decodable, Sendable {
        let requests: Double
        let promptTokens: Double
        let cost: Double
        let inputTokens: Double
        let outputTokens: Double
        let thinkingTokens: Double
        let cacheReadTokens: Double
        let cacheWrite5mTokens: Double
        let cacheWrite1hTokens: Double
        let cacheWriteTokens: Double
        let errors: Double
        let subagentRequests: Double
        let sessions: Double
        let projects: Double
        let firstSeen: Int?
        let lastSeen: Int?

        enum CodingKeys: String, CodingKey {
            case requests, cost, errors, sessions, projects
            case promptTokens = "prompt_tokens", inputTokens = "input_tokens", outputTokens = "output_tokens"
            case thinkingTokens = "thinking_tokens", cacheReadTokens = "cache_read_tokens"
            case cacheWrite5mTokens = "cache_write_5m_tokens", cacheWrite1hTokens = "cache_write_1h_tokens"
            case cacheWriteTokens = "cache_write_tokens", subagentRequests = "subagent_requests"
            case firstSeen = "first_seen", lastSeen = "last_seen"
        }
    }

    struct ContextRow: Decodable, Sendable, Identifiable {
        let component: String
        let estTokens: Double
        let chars: Double
        let estCostOnce: Double
        let requests: Double
        var id: String { component }
        enum CodingKeys: String, CodingKey {
            case component, chars, requests, estTokens = "est_tokens", estCostOnce = "est_cost_once"
        }
    }

    struct ModelRow: Decodable, Sendable, Identifiable {
        let model: String?
        let requests: Double
        let promptTokens: Double
        let cost: Double
        let outputTokens: Double
        let thinkingTokens: Double
        let cacheReadTokens: Double
        let errors: Double
        let subagentRequests: Double
        var id: String { model ?? "?" }
        enum CodingKeys: String, CodingKey {
            case model, requests, cost, errors
            case promptTokens = "prompt_tokens", outputTokens = "output_tokens", thinkingTokens = "thinking_tokens"
            case cacheReadTokens = "cache_read_tokens", subagentRequests = "subagent_requests"
        }
    }

    struct ThreadRow: Decodable, Sendable, Identifiable {
        let thread: String
        let requests: Double
        let promptTokens: Double
        let cost: Double
        let outputTokens: Double
        let cacheReadTokens: Double
        let agents: Double
        var id: String { thread }
        enum CodingKeys: String, CodingKey {
            case thread, requests, cost, agents
            case promptTokens = "prompt_tokens", outputTokens = "output_tokens", cacheReadTokens = "cache_read_tokens"
        }
    }

    struct Latency: Decodable, Sendable {
        let ttfbP50: Double?
        let ttfbP95: Double?
        let durationP50: Double?
        let durationP95: Double?
        let durationMax: Double?
        enum CodingKeys: String, CodingKey {
            case ttfbP50 = "ttfb_p50", ttfbP95 = "ttfb_p95", durationP50 = "duration_p50"
            case durationP95 = "duration_p95", durationMax = "duration_max"
        }
    }

    struct StopReason: Decodable, Sendable, Identifiable {
        let stopReason: String
        let requests: Double
        var id: String { stopReason }
        enum CodingKeys: String, CodingKey { case requests, stopReason = "stop_reason" }
    }

    struct AccountSplitRow: Decodable, Sendable, Identifiable {
        let id: String
        let label: String
        let active: Int
        let requests: Double
        let promptTokens: Double
        let cost: Double
        let outputTokens: Double
        let sessions: Double
        let lastSeen: Int?
        enum CodingKeys: String, CodingKey {
            case id, label, active, requests, cost, sessions
            case promptTokens = "prompt_tokens", outputTokens = "output_tokens", lastSeen = "last_seen"
        }
    }

    struct Timeline: Decodable, Sendable {
        let start: Int?
        let end: Int?
        let bucketMs: Int
        let rows: [TimelineRow]
        enum CodingKeys: String, CodingKey { case start, end, rows, bucketMs = "bucket_ms" }
    }

    struct TimelineRow: Decodable, Sendable {
        let t: Double
        let model: String?
        let requests: Double
        let promptTokens: Double
        let cost: Double
        let outputTokens: Double
        let cacheReadTokens: Double
        let subagentTokens: Double
        let errors: Double
        enum CodingKeys: String, CodingKey {
            case t, model, requests, cost, errors
            case promptTokens = "prompt_tokens", outputTokens = "output_tokens"
            case cacheReadTokens = "cache_read_tokens", subagentTokens = "subagent_tokens"
        }
    }

    struct QuotaPoint: Decodable, Sendable {
        let t: Double
        let rl5h: Double?
        let rl7d: Double?
        enum CodingKeys: String, CodingKey { case t, rl5h = "rl_5h", rl7d = "rl_7d" }
        var date: Date { Date(timeIntervalSince1970: t / 1000) }
    }
}

struct ProjectRow: Decodable, Sendable, Identifiable, Hashable {
    let project: String
    let sessions: Double
    let requests: Double
    let promptTokens: Double
    let cost: Double
    let outputTokens: Double
    let cacheReadTokens: Double
    let subagentTokens: Double
    let errors: Double
    let models: String?
    let firstSeen: Int?
    let lastSeen: Int?
    var id: String { project }
    var name: String { project.split(separator: "/").last.map(String.init) ?? project }
    enum CodingKeys: String, CodingKey {
        case project, sessions, requests, cost, errors, models
        case promptTokens = "prompt_tokens", outputTokens = "output_tokens", cacheReadTokens = "cache_read_tokens"
        case subagentTokens = "subagent_tokens", firstSeen = "first_seen", lastSeen = "last_seen"
    }
}

/// A session as the proxy saw it — distinct from `LiveSession`, which is a transcript.
struct ProxySessionRow: Decodable, Sendable, Identifiable, Hashable {
    let id: String
    let project: String?
    let gitBranch: String?
    let entrypoint: String?
    let startedAt: Int
    let lastSeenAt: Int
    let requests: Double
    let promptTokens: Double
    let cost: Double
    let outputTokens: Double
    let cacheReadTokens: Double
    let subagentTokens: Double
    let errors: Double
    let subagents: Double
    let models: String?
    var shortId: String { String(id.prefix(8)) }
    var projectName: String { project?.split(separator: "/").last.map(String.init) ?? "—" }
    /// `claude-cli/2.1.269 (external, claude-vscode, …)` → `vscode`; the CLI otherwise.
    var client: String {
        guard let e = entrypoint else { return "" }
        return e.contains("vscode") ? "VS Code" : e.contains("claude-cli") ? "CLI" : String(e.prefix(20))
    }
    enum CodingKeys: String, CodingKey {
        case id, project, entrypoint, requests, cost, errors, subagents, models
        case gitBranch = "git_branch", startedAt = "started_at", lastSeenAt = "last_seen_at"
        case promptTokens = "prompt_tokens", outputTokens = "output_tokens"
        case cacheReadTokens = "cache_read_tokens", subagentTokens = "subagent_tokens"
    }
}

struct RequestRow: Decodable, Sendable, Identifiable, Hashable {
    let id: Int
    let requestId: String?
    let sessionId: String?
    let project: String?
    let model: String?
    let status: Int?
    let ttfbMs: Int?
    let durationMs: Int?
    let stopReason: String?
    let isSidechain: Int?
    let agentId: String?
    let startedAt: Int
    let error: String?
    let promptTokens: Double?
    let inputTokens: Double?
    let outputTokens: Double?
    let thinkingTokens: Double?
    let cost: Double?
    let cacheReadTokens: Double?
    let cacheWrite5mTokens: Double?
    let cacheWrite1hTokens: Double?
    let cacheWriteTokens: Double?
    /// Comma-separated tool names, in call order.
    let tools: String?
    /// `component:est_tokens` pairs, comma-separated.
    let context: String?

    var isError: Bool { (status ?? 0) >= 400 || error != nil }
    var isSubagent: Bool { isSidechain == 1 }
    var toolList: [String] { (tools ?? "").split(separator: ",").map(String.init) }
    var contextSplit: [(String, Double)] {
        (context ?? "").split(separator: ",").compactMap { pair in
            let p = pair.split(separator: ":", maxSplits: 1)
            guard p.count == 2, let v = Double(p[1]) else { return nil }
            return (String(p[0]), v)
        }
    }
    enum CodingKeys: String, CodingKey {
        case id, project, model, status, error, tools, context, cost
        case requestId = "request_id", sessionId = "session_id", ttfbMs = "ttfb_ms", durationMs = "duration_ms"
        case stopReason = "stop_reason", isSidechain = "is_sidechain", agentId = "agent_id", startedAt = "started_at"
        case promptTokens = "prompt_tokens", inputTokens = "input_tokens", outputTokens = "output_tokens"
        case thinkingTokens = "thinking_tokens", cacheReadTokens = "cache_read_tokens"
        case cacheWrite5mTokens = "cache_creation_5m_tokens", cacheWrite1hTokens = "cache_creation_1h_tokens"
        case cacheWriteTokens = "cache_write_tokens"
    }
}

/// `GET /api/tools`: what the assistant called, and what came back.
struct ToolsPayload: Decodable, Sendable {
    struct Produced: Decodable, Sendable, Identifiable {
        let name: String?
        let results: Double
        let chars: Double
        let estTokens: Double
        let maxTokens: Double
        let estCostOnce: Double
        let errors: Double
        var id: String { name ?? "?" }
        enum CodingKeys: String, CodingKey {
            case name, results, chars, errors
            case estTokens = "est_tokens", maxTokens = "max_tokens", estCostOnce = "est_cost_once"
        }
    }
    struct Called: Decodable, Sendable, Identifiable {
        let name: String?
        let calls: Double
        let requests: Double
        let subagentCalls: Double
        var id: String { name ?? "?" }
        enum CodingKeys: String, CodingKey { case name, calls, requests, subagentCalls = "subagent_calls" }
    }
    let produced: [Produced]
    let called: [Called]
}

// MARK: - Settings: setup, wrapper, sign-in

struct SetupStatus: Decodable, Sendable {
    struct Service: Decodable, Sendable {
        let registered: Bool
        let supported: Bool
        let platform: String
    }
    struct Managed: Decodable, Sendable, Identifiable {
        let path: String
        let scope: String
        let value: String?
        let previous: String?
        let backupPath: String?
        let appliedAt: Int?
        var id: String { path }
        enum CodingKeys: String, CodingKey {
            case path, scope, value, previous, backupPath = "backup_path", appliedAt = "applied_at"
        }
    }
    let service: Service
    let url: String
    let globalSettingsPath: String
    let managed: [Managed]
    var globalRouted: Bool { managed.contains { $0.scope == "global" } }
    enum CodingKeys: String, CodingKey { case service, url, managed, globalSettingsPath = "global_settings_path" }
}

/// `GET /api/login/status`: the `claude setup-token` flow the router drives.
struct LoginStatus: Decodable, Sendable {
    /// starting | awaiting_sign_in | done | failed — absent when the start itself failed.
    let phase: String?
    let url: String?
    let label: String?
    let accountId: String?
    let email: String?
    let error: String?
    let tail: String?
    enum CodingKeys: String, CodingKey { case phase, url, label, email, error, tail, accountId = "account_id" }
}

/// What the write endpoints answer: `{ok}` or `{error}` or `{ok:false, reason}`.
struct OkReply: Decodable, Sendable {
    let ok: Bool?
    let error: String?
    let reason: String?
    let id: String?
    var problem: String? { error ?? reason ?? (ok == false ? "refused" : nil) }
}

struct AddAccountBody: Encodable, Sendable { let label: String; let source: String; let token: String? }
struct AccountEditBody: Encodable, Sendable { let id: String; let label: String?; let email: String?; let token: String? }
struct AccountRefBody: Encodable, Sendable { let id: String }
struct UsageTokenBody: Encodable, Sendable {
    let accountId: String
    let token: String?
    let clear: Bool?
    enum CodingKeys: String, CodingKey { case token, clear, accountId = "account_id" }
}
struct LoginStartBody: Encodable, Sendable { let label: String }
struct ServiceBody: Encodable, Sendable { let enable: Bool }
struct ConfigBody: Encodable, Sendable { let enable: Bool; let project: String? }
struct ShimBody: Encodable, Sendable { let path: String; let action: String }
struct PasswordBody: Encodable, Sendable { let password: String?; let clear: Bool? }

/// `POST /api/accounts/probe`: whether a profile account can sign requests right now.
struct ProbeReply: Decodable, Sendable {
    let loggedIn: Bool?
    let detail: String?
    let error: String?
    enum CodingKeys: String, CodingKey { case detail, error, loggedIn = "logged_in" }
}

// MARK: - Push notifications

/// `GET /api/push`: the router's APNs key, and the phones it knows.
struct PushStatus: Decodable, Sendable {
    struct Device: Decodable, Sendable, Identifiable {
        let tokenTail: String
        let name: String?
        let environment: String
        let routerRef: String?
        let registeredAt: Int
        let lastSeenAt: Int
        let lastResult: String?
        var id: String { tokenTail }
        enum CodingKeys: String, CodingKey {
            case name, environment
            case tokenTail = "token_tail", routerRef = "router_ref", registeredAt = "registered_at"
            case lastSeenAt = "last_seen_at", lastResult = "last_result"
        }
    }
    struct Sent: Decodable, Sendable {
        let at: Int
        let what: String
    }
    let configured: Bool
    let teamId: String?
    let keyId: String?
    let bundleId: String?
    let devices: [Device]
    let lastError: String?
    let lastSent: Sent?
    /// The key file dropped beside the router: taken in, or what it still waits on.
    let keyFile: String?
    enum CodingKeys: String, CodingKey {
        case configured, devices
        case teamId = "team_id", keyId = "key_id", bundleId = "bundle_id", lastError = "last_error", lastSent = "last_sent"
        case keyFile = "key_file"
    }
}

struct PushConfigBody: Encodable, Sendable {
    let teamId: String
    let keyId: String
    let p8: String
    let bundleId: String?
    let clear: Bool?
    enum CodingKeys: String, CodingKey { case p8, clear, teamId = "team_id", keyId = "key_id", bundleId = "bundle_id" }
}
/// `/api/push/pending`: the prompts the router says are still waiting for a person.
struct PendingReply: Decodable, Sendable {
    let waiting: [String]
}

struct PushRegisterBody: Encodable, Sendable {
    let token: String
    let name: String
    let environment: String
    let routerRef: String
    /// The team the app is signed under, and its bundle id: what the router needs to
    /// complete a dropped-in key and to address this app.
    let teamId: String?
    let bundleId: String?
    enum CodingKeys: String, CodingKey { case token, name, environment, routerRef = "router_ref", teamId = "team_id", bundleId = "bundle_id" }
}
struct PushTestReply: Decodable, Sendable {
    struct Line: Decodable, Sendable { let device: String; let ok: Bool; let error: String? }
    let ok: Bool?
    let results: [Line]?
    let error: String?
}

/// `GET /api/live/options`: what a session can be switched to.
struct LiveOptions: Decodable, Sendable {
    let models: [String]
    let efforts: [String]
}
struct ModelBody: Encodable, Sendable { let session: String; let model: String }
struct EffortBody: Encodable, Sendable { let session: String; let effort: String }
/// The answer to a model or effort change.
struct SwitchReply: Decodable, Sendable {
    let ok: Bool
    let model: String?
    let effort: String?
    let notices: [String]?
    let pending: Bool?
    let reason: String?
}

struct ShimAutoBody: Encodable, Sendable { let enabled: Bool }

struct AsideBody: Encodable, Sendable { let session: String; let text: String }
struct AsideReply: Decodable, Sendable {
    let ok: Bool
    let answer: String?
    let fork: String?
    let cost: Double?
    let tookMs: Int?
    let reason: String?
    enum CodingKeys: String, CodingKey { case ok, answer, fork, cost, reason, tookMs = "took_ms" }
}
