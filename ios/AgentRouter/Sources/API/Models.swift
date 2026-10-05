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

    // What a router says about how it is run. Absent from one that predates hosting, and
    // optional for that reason.

    /// `local` (a Mac) or `server` (hosted). A router that sends none is local.
    let mode: String?
    /// Whether the password is set by the router's deployment rather than its dashboard.
    let passwordPinned: Bool?
    /// The https URL a hosted router's clients use. Null from a local router.
    let publicURL: String?

    /// Hosted: run by a deployment, with no Mac of its own; its sessions come from the
    /// machines that run the Agent Router agent.
    var isHosted: Bool { mode == "server" }

    enum CodingKeys: String, CodingKey {
        case configured, updatedAt = "updated_at", remoteBind = "remote_bind", fromLoopback = "from_loopback"
        case mode, passwordPinned = "password_pinned", publicURL = "public_url"
    }
}

extension AuthStatus {
    /// The keys every router sends decode exactly as they always have; the hosting keys
    /// leniently, as a session row's newer keys are: a status that fails to decode makes
    /// the whole router read as unreachable.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        configured = try c.decode(Bool.self, forKey: .configured)
        remoteBind = try c.decode(Bool.self, forKey: .remoteBind)
        fromLoopback = try c.decode(Bool.self, forKey: .fromLoopback)
        updatedAt = try c.decodeIfPresent(Int.self, forKey: .updatedAt)
        mode = c.lenient(String.self, .mode)
        passwordPinned = c.lenient(Bool.self, .passwordPinned)
        publicURL = c.lenient(String.self, .publicURL)
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

/// One row of `GET /api/live/sessions`: a transcript written in the last week, or the
/// transcript of a session still open however old.
struct LiveSession: Decodable, Sendable, Identifiable, Hashable {
    let sessionId: String
    let cwd: String?
    let project: String
    let gitBranch: String?
    let title: String?
    let createdMs: Int?
    /// When the conversation last said something — its content, not the file's mtime,
    /// from a router that knows the difference.
    let lastActivityMs: Int
    /// Doing something or waiting on you. A current router sets it from `activity`; an
    /// older one from how recently the transcript moved.
    let active: Bool
    /// working | ask | idle | stalled | failed
    let state: String
    let archived: Bool
    /// Under the wrapper and answering on its socket: a message or a stop would arrive.
    let wrapped: Bool
    let bytes: Int

    // What the router's one derivation says the session is doing — the same answer the
    // open view's status event gives. Every key from here down is absent from an older
    // router and optional for that reason: the phone must keep listing its sessions.

    let activity: SessionActivity?
    /// Whether a process still holds the conversation; nil when the router cannot tell.
    let isOpen: Bool?
    /// The router's own line for what it is doing, worded once for every client.
    let detail: String?
    /// When the current activity began.
    let sinceMs: Int?
    /// When a closed session ended, and how the router knows: `exit` or `unknown`.
    let endedMs: Int?
    let endedHow: String?
    /// Closed with its turn still open: the process went away mid-work.
    let midTurn: Bool?
    let waitingOn: WaitingOn?
    let background: BackgroundWork?
    /// The colour class the router chose, so every client colours a state alike.
    let tone: SessionTone?
    /// `registry` (Claude Code's own record of the process) or `transcript`.
    let source: String?
    /// The router's clock when it worked this out.
    let asOfMs: Int?
    /// The machine the session runs on, from a hosted router that merges several machines'
    /// lists. A local router never sends it.
    let machine: SessionMachine?

    var id: String { sessionId }
    var shortId: String { String(sessionId.prefix(8)) }
    /// The last segment of the working directory, a Windows path's included, else the
    /// router's own name for the project (SE-C23).
    var projectName: String {
        if let cwd, let last = cwd.split(whereSeparator: { $0 == "/" || $0 == "\\" }).last { return String(last) }
        return project.isEmpty ? "—" : project
    }
    /// What to call the machine the session runs on: its label, or the start of its id when
    /// the label is blank. Nil from a local router, which names no machine.
    var machineName: String? {
        guard let machine else { return nil }
        let label = machine.label.trimmingCharacters(in: .whitespacesAndNewlines)
        if !label.isEmpty { return label }
        let id = machine.id.trimmingCharacters(in: .whitespacesAndNewlines)
        return id.isEmpty ? nil : String(id.prefix(8))
    }
    var label: String { title ?? shortId }
    var lastActivity: Date { Date(timeIntervalSince1970: TimeInterval(lastActivityMs) / 1000) }

    /// Doing something now or waiting on you — what keeps a session in view however long
    /// ago it last wrote. The router's activity when it names one this build knows;
    /// otherwise its `active`, which a current router derives from that same activity.
    var isLive: Bool {
        if let activity, activity.isKnown { return activity.isActive }
        return active
    }
    /// At work: a turn running, or background agents after one.
    var isWorking: Bool { activity?.isWorking ?? false }
    /// Blocked on a person.
    var needsYou: Bool { activity == .waiting }

    /// Seconds in the current activity, on the router's clock alone: `since_ms` against the
    /// `as_of_ms` the row was worked out at, never this phone's clock — one that disagrees
    /// with the Mac's would shift every duration, and the web view and the other phone would
    /// say something else. Nil without both; a current router always sends both with an
    /// activity. The list is re-read every few seconds, which is all the ticking a row needs.
    var elapsed: TimeInterval? {
        guard let sinceMs, let asOfMs else { return nil }
        return max(0, TimeInterval(asOfMs - sinceMs) / 1000)
    }

    enum CodingKeys: String, CodingKey {
        case cwd, project, title, active, state, archived, wrapped, bytes
        case sessionId = "session_id", gitBranch = "git_branch", createdMs = "created_ms"
        case lastActivityMs = "last_activity_ms"
        case activity, detail, background, tone, source
        case isOpen = "open", sinceMs = "since_ms", endedMs = "ended_ms", endedHow = "ended_how"
        case midTurn = "mid_turn", waitingOn = "waiting_on", asOfMs = "as_of_ms"
        case machine
    }
}

/// Where a session runs (`machine`), as the hosted router that merged its list names it:
/// the agent's id for the machine, which survives a rename, and what to call it — its host
/// name unless the agent was given a label.
struct SessionMachine: Decodable, Sendable, Hashable {
    let id: String
    let label: String
}

extension LiveSession {
    /// Written out rather than synthesized for one reason: the keys the router added for
    /// session liveness are read leniently. The legacy keys decode exactly as they always
    /// have; a new key that is absent, null or not what this build expects reads as nil,
    /// because one row that fails to decode fails the whole list.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        sessionId = try c.decode(String.self, forKey: .sessionId)
        cwd = try c.decodeIfPresent(String.self, forKey: .cwd)
        project = try c.decode(String.self, forKey: .project)
        gitBranch = try c.decodeIfPresent(String.self, forKey: .gitBranch)
        title = try c.decodeIfPresent(String.self, forKey: .title)
        createdMs = try c.decodeIfPresent(Int.self, forKey: .createdMs)
        lastActivityMs = try c.decode(Int.self, forKey: .lastActivityMs)
        active = try c.decode(Bool.self, forKey: .active)
        state = try c.decode(String.self, forKey: .state)
        archived = try c.decode(Bool.self, forKey: .archived)
        wrapped = try c.decode(Bool.self, forKey: .wrapped)
        bytes = try c.decode(Int.self, forKey: .bytes)

        activity = c.lenient(String.self, .activity).map(SessionActivity.init)
        isOpen = c.lenient(Bool.self, .isOpen)
        detail = c.lenient(String.self, .detail)
        sinceMs = c.lenient(Int.self, .sinceMs)
        endedMs = c.lenient(Int.self, .endedMs)
        endedHow = c.lenient(String.self, .endedHow)
        midTurn = c.lenient(Bool.self, .midTurn)
        waitingOn = c.lenient(WaitingOn.self, .waitingOn)
        background = c.lenient(BackgroundWork.self, .background)
        tone = c.lenient(String.self, .tone).map(SessionTone.init)
        source = c.lenient(String.self, .source)
        asOfMs = c.lenient(Int.self, .asOfMs)
        machine = c.lenient(SessionMachine.self, .machine)
    }
}

extension KeyedDecodingContainer {
    /// A key added after phones were already in the field: absent, null or of a shape this
    /// build does not expect, it is nil — never the error that would fail everything
    /// around it.
    func lenient<T: Decodable>(_ type: T.Type, _ key: Key) -> T? {
        try? decodeIfPresent(type, forKey: key)
    }
}

/// What a session is doing, in the router's words (`activity`). One derivation on the
/// router produces it for the list, the open view and the notifier, so the phone draws it
/// rather than inferring it. A word this build does not know is kept, and drawn neutral.
enum SessionActivity: Hashable, Sendable {
    /// No process holds the conversation any more.
    case closed
    /// Open, and the turn is over.
    case idle
    /// Mid-turn with no tool outstanding: thinking, writing, or retrying a request.
    case generating
    /// Mid-turn with a tool call outstanding.
    case tool
    /// Blocked on a person: a question, a permission, a plan, a dialog.
    case waiting
    /// The turn is over but background agents or workflows are still running; the session
    /// will act again by itself.
    case background
    /// The last turn ended on an API error.
    case failed
    /// Mid-turn with nothing observable for ten minutes.
    case stalled
    case unknown(String)

    init(_ raw: String) {
        switch raw {
        case "closed": self = .closed
        case "idle": self = .idle
        case "generating": self = .generating
        case "tool": self = .tool
        case "waiting": self = .waiting
        case "background": self = .background
        case "failed": self = .failed
        case "stalled": self = .stalled
        default: self = .unknown(raw)
        }
    }

    var isKnown: Bool {
        if case .unknown = self { return false }
        return true
    }
    /// Work in flight that will produce output without anyone typing.
    var isWorking: Bool {
        switch self {
        case .generating, .tool, .background, .stalled: true
        default: false
        }
    }
    /// What the router's `active` means: at work, or waiting on you.
    var isActive: Bool { isWorking || self == .waiting }
    /// What Stop can act on: a turn in progress. Not background work alone — what an
    /// interrupt does to running background agents is unmeasured and could discard their
    /// results, so the router refuses it and no client offers it.
    var isStoppable: Bool {
        switch self {
        case .generating, .tool, .stalled: true
        default: false
        }
    }
    /// The colour class the router gives this activity, for a payload that carries an
    /// activity without a tone.
    var tone: SessionTone {
        switch self {
        case .generating, .tool, .background: .busy
        case .waiting: .attention
        case .stalled: .warn
        case .failed: .error
        case .idle: .idle
        case .closed: .closed
        case .unknown(let raw): .unknown(raw)
        }
    }
    /// What VoiceOver says for a dot of this activity.
    var spoken: String {
        switch self {
        case .closed: "ended"
        case .idle: "idle"
        case .generating: "working"
        case .tool: "running a tool"
        case .waiting: "waiting for you"
        case .background: "working in the background"
        case .failed: "failed"
        case .stalled: "stalled"
        case .unknown(let raw): raw
        }
    }
}

/// The router's colour class for a session (`tone`): chosen once, on the router, so the
/// web view and both phones colour the same state the same way.
enum SessionTone: Hashable, Sendable {
    case busy, attention, warn, error, idle, closed
    case unknown(String)

    init(_ raw: String) {
        switch raw {
        case "busy": self = .busy
        case "attention": self = .attention
        case "warn": self = .warn
        case "error": self = .error
        case "idle": self = .idle
        case "closed": self = .closed
        default: self = .unknown(raw)
        }
    }

    var isKnown: Bool {
        if case .unknown = self { return false }
        return true
    }

    /// The tone to draw: the router's when it sent one this build knows, else the one its
    /// activity implies. Nil only when the router sent neither — an older router.
    static func effective(_ tone: SessionTone?, _ activity: SessionActivity?) -> SessionTone? {
        if let tone, tone.isKnown { return tone }
        return activity?.tone
    }
}

/// What a waiting session is blocked on (`waiting_on`).
struct WaitingOn: Decodable, Sendable, Hashable {
    /// permission | question | plan | dialog
    let kind: String?
    let tool: String?
    /// The wrapper's id for the prompt, when the session is wrapped: what an answer names.
    let requestId: String?
    let toolUseId: String?
    let summary: String?

    enum CodingKeys: String, CodingKey {
        case kind, tool, summary, requestId = "request_id", toolUseId = "tool_use_id"
    }
}

/// Work a session left running after its turn (`background`): agents and workflows that
/// will report back, and shells that will start a turn of their own when they finish.
struct BackgroundWork: Decodable, Sendable, Hashable {
    let agents: Int?
    let workflows: Int?
    let shells: Int?
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

/// The `status` event: what the session is doing — from a current router the same
/// derivation the session list uses, from an older one the tail of the transcript.
struct LiveStatus: Decodable, Sendable, Equatable {
    /// ask | tool | idle | working | failed — the vocabulary every build of this app
    /// understands; `activity` says the same thing in finer words.
    let state: String
    let detail: String
    /// How long the session has been doing what it is doing now, by the router's clock.
    let quietMs: Int
    /// Messages typed while a turn ran, still waiting their turn.
    let queued: Int?

    // Absent from an older router, and optional for that reason.
    let activity: SessionActivity?
    let isOpen: Bool?
    let sinceMs: Int?
    let asOfMs: Int?
    let waitingOn: WaitingOn?
    let background: BackgroundWork?
    let tone: SessionTone?
    let endedMs: Int?

    /// Whether a turn is running that Stop could end: thinking, running a tool, or gone
    /// quiet mid-turn. Not background work alone, and not an activity this build does not
    /// know. From an older router, its `tool` and `working`, which is all Stop ever keyed on.
    var isBusy: Bool {
        if let activity { return activity.isStoppable }
        return state == "tool" || state == "working"
    }
    /// Blocked on a person.
    var needsYou: Bool {
        if let activity, activity.isKnown { return activity == .waiting }
        return state == "ask"
    }
    /// Doing something or waiting on you — the halo around the header's dot.
    var isActive: Bool {
        guard let activity else { return isBusy }
        return activity.isActive
    }

    enum CodingKeys: String, CodingKey {
        case state, detail, queued, quietMs = "quiet_ms"
        case activity, background, tone
        case isOpen = "open", sinceMs = "since_ms", asOfMs = "as_of_ms"
        case waitingOn = "waiting_on", endedMs = "ended_ms"
    }
}

extension LiveStatus {
    /// The legacy keys as they always were; the newer ones leniently, as on a session row:
    /// a status that fails to decode leaves the header with nothing to say.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        state = try c.decode(String.self, forKey: .state)
        detail = try c.decode(String.self, forKey: .detail)
        quietMs = try c.decode(Int.self, forKey: .quietMs)
        queued = try c.decodeIfPresent(Int.self, forKey: .queued)
        activity = c.lenient(String.self, .activity).map(SessionActivity.init)
        isOpen = c.lenient(Bool.self, .isOpen)
        sinceMs = c.lenient(Int.self, .sinceMs)
        asOfMs = c.lenient(Int.self, .asOfMs)
        waitingOn = c.lenient(WaitingOn.self, .waitingOn)
        background = c.lenient(BackgroundWork.self, .background)
        tone = c.lenient(String.self, .tone).map(SessionTone.init)
        endedMs = c.lenient(Int.self, .endedMs)
    }
}

/// What the live endpoints answer with — success and failure share the shape, so a reply
/// of `ok: false` is a refusal whatever status came with it (`Verdict`).
struct LiveReply: Decodable, Sendable, Verdict {
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
    /// The permission mode in force, as the session last stamped it on a turn.
    let mode: String?
    let input: Int
    let output: Int
    let cacheRead: Int
    let cacheWrite: Int
    let cost: Double?
    let total: Int
    let turns: Int

    enum CodingKeys: String, CodingKey {
        case model, effort, mode, input, output, cost, total, turns
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

/// `POST /api/accounts/probe`: whether a profile account can sign requests right now —
/// what `claude auth status --json` says of its profile, as the router reads it and sends it
/// on (src/accounts.rs, `AuthStatus`): `loggedIn`, in the CLI's own camel case. This read
/// `logged_in` once, which the router never sends, so a profile not yet signed in was never
/// said to be.
struct ProbeReply: Decodable, Sendable {
    let loggedIn: Bool?
    let detail: String?
    let error: String?
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
    /// Optional so an older router, which sends no modes, still decodes.
    let modes: [LiveMode]?
}

/// A permission mode and the wording for it. The label comes from the router so the
/// three clients cannot drift apart on what to call these.
struct LiveMode: Decodable, Sendable, Hashable {
    let value: String
    let label: String
    /// Turning this one on stops the session asking about anything, so every client
    /// confirms first.
    var isBypass: Bool { value == "bypassPermissions" }
}

/// `GET /api/live/prompt`: what the session is blocked on, if anything. A question
/// carries its own options and is answered from the feed; anything else is a request
/// for permission, answered here.
struct LivePrompt: Decodable, Sendable {
    let pending: Bool
    let requestId: String?
    let tool: String?
    let question: Bool?
    let displayName: String?
    let description: String?
    let reason: String?
    let suggestions: [PromptSuggestion]?
    /// How many prompts are waiting in the session, this one the newest. Absent from an
    /// older router, which only ever reported the one.
    let count: Int?
    /// The questions a waiting AskUserQuestion puts, in order, from the tool's own input:
    /// what says whether a form drawn from the transcript — which carries no prompt id — is
    /// the question waiting now. Nil when the prompt does not list them. A listed question
    /// with no text is kept, as nil, so that it matches no form.
    let questions: [String?]?

    var isPermission: Bool { pending && question != true }
    var isQuestion: Bool { pending && question == true }

    /// Whether this is the question a form was drawn for: the same questions in the same
    /// order — the web view's `sameQuestions`, so a form one client refuses to send the
    /// others refuse too. A prompt that does not list its questions is taken at its word.
    func asks(_ texts: [String]) -> Bool {
        guard let questions else { return true }
        return questions == texts.map { Optional($0) }
    }

    enum CodingKeys: String, CodingKey {
        case pending, tool, question, description, reason, suggestions, count, input
        case requestId = "request_id"
        case displayName = "display_name"
    }
}

extension LivePrompt {
    /// Written out for `input` alone, which is whatever the tool was called with — for a
    /// permission request a command or a path, of any shape — so it is read leniently: a
    /// shape this build does not expect costs the question list, never the prompt, whose
    /// poll would otherwise stop showing the permission card. The rest decodes as it did.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        pending = try c.decode(Bool.self, forKey: .pending)
        requestId = try c.decodeIfPresent(String.self, forKey: .requestId)
        tool = try c.decodeIfPresent(String.self, forKey: .tool)
        question = try c.decodeIfPresent(Bool.self, forKey: .question)
        displayName = try c.decodeIfPresent(String.self, forKey: .displayName)
        description = try c.decodeIfPresent(String.self, forKey: .description)
        reason = try c.decodeIfPresent(String.self, forKey: .reason)
        suggestions = try c.decodeIfPresent([PromptSuggestion].self, forKey: .suggestions)
        count = try c.decodeIfPresent(Int.self, forKey: .count)
        questions = c.lenient(AskedInput.self, .input)?.questions?.map(\.question)
    }
}

/// Just enough of AskUserQuestion's input to say which questions it puts.
private struct AskedInput: Decodable {
    struct Asked: Decodable {
        let question: String?

        /// An entry that is not an object, or whose text is not a string, is a question with
        /// no text: it stays in the list, so the count and the order still have to match.
        init(from decoder: Decoder) throws {
            question = (try? decoder.container(keyedBy: CodingKeys.self))?.lenient(String.self, .question)
        }

        enum CodingKeys: String, CodingKey { case question }
    }

    let questions: [Asked]?
}

/// One of the prompt's own offers — "always allow this, for the rest of the session".
/// Only ever these: the router refuses an index it did not offer, and the editor on the
/// far end drops anything it did not suggest.
struct PromptSuggestion: Decodable, Sendable, Hashable, Identifiable {
    let index: Int
    let label: String
    var id: Int { index }
}
struct ModelBody: Encodable, Sendable { let session: String; let model: String }
struct EffortBody: Encodable, Sendable { let session: String; let effort: String }
struct ModeBody: Encodable, Sendable { let session: String; let mode: String }
/// The answer to a model or effort change.
struct SwitchReply: Decodable, Sendable {
    let ok: Bool
    let model: String?
    let effort: String?
    let mode: String?
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
