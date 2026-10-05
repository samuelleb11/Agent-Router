// Everything the dashboard shows above and below the fold, for one router and one scope.

import Foundation
import Observation

/// The dashboard's range buttons.
enum UsageRange: String, CaseIterable, Identifiable, Sendable {
    case all, h1 = "1h", h24 = "24h", d7 = "7d", d30 = "30d"
    var id: String { rawValue }
    var label: String { self == .all ? "All" : rawValue }
    var sinceMs: Int? {
        switch self {
        case .all: nil
        case .h1: 3_600_000
        case .h24: 86_400_000
        case .d7: 7 * 86_400_000
        case .d30: 30 * 86_400_000
        }
    }
}

/// What the page is narrowed to. Empty means the whole router.
struct UsageScope: Hashable, Sendable {
    var account: String? = nil
    var project: String? = nil
    var session: String? = nil
    var isEmpty: Bool { account == nil && project == nil && session == nil }
    var title: String {
        if let session { return "Session \(session.prefix(8))" }
        // The sessions with no directory are narrowed to by the router's key for them, and
        // named as their row is.
        if let project { return ProjectPath.name(project == ProjectPath.unresolved ? nil : project) }
        if let account { return account }
        return "Usage"
    }
}

/// One bar of the activity chart, one point of the sparkline.
struct Bucket: Identifiable, Sendable {
    let t: Double
    var requests = 0.0, prompt = 0.0, output = 0.0, cache = 0.0, sub = 0.0, errors = 0.0, cost = 0.0
    var families: [String: Double] = [:]
    var id: Double { t }
    var date: Date { Date(timeIntervalSince1970: t / 1000) }

    /// opus | sonnet | haiku | other, from the wire name.
    static func family(_ model: String?) -> String {
        let m = (model ?? "").lowercased()
        for f in ["opus", "sonnet", "haiku"] where m.contains(f) { return f }
        return "other"
    }

    /// The dashboard's `buckets()`: every slot in the range, empty ones included, so a
    /// quiet hour is drawn as an hour of quiet.
    static func build(_ tl: Summary.Timeline, tz: TimeZone = .current) -> [Bucket] {
        guard tl.bucketMs > 0, let start = tl.start, let end = tl.end else { return [] }
        let b = Double(tl.bucketMs)
        let tzMs = Double(tz.secondsFromGMT()) * 1000
        let t0 = floor((Double(start) + tzMs) / b) * b - tzMs
        var byT: [Double: Bucket] = [:]
        for r in tl.rows {
            var o = byT[r.t] ?? Bucket(t: r.t)
            o.requests += r.requests; o.prompt += r.promptTokens; o.output += r.outputTokens
            o.cache += r.cacheReadTokens; o.sub += r.subagentTokens; o.errors += r.errors; o.cost += r.cost
            o.families[family(r.model), default: 0] += r.promptTokens
            byT[r.t] = o
        }
        var out: [Bucket] = []
        var t = t0
        while t <= Double(end), out.count < 500 {
            out.append(byT[t] ?? Bucket(t: t))
            t += b
        }
        return out
    }
}

@MainActor
@Observable
final class UsageState {
    let router: RouterState
    let scope: UsageScope
    var range: UsageRange = .all {
        didSet { if range != oldValue { Task { await refresh() } } }
    }
    var summary: Summary?
    var projects: [ProjectRow] = []
    var sessions: [ProxySessionRow] = []
    var tools: ToolsPayload?
    var requests: [RequestRow] = []
    var failure: String?
    var refreshing = false
    var lastRefresh: Date?

    init(router: RouterState, scope: UsageScope) {
        self.router = router
        self.scope = scope
    }

    var buckets: [Bucket] { summary.map { Bucket.build($0.timeline) } ?? [] }

    private var since: String? { range.sinceMs.map { String(Int(Date().timeIntervalSince1970 * 1000) - $0) } }

    func refresh() async {
        guard let client = router.liveClient else { return }
        refreshing = true
        defer { refreshing = false }
        let tz = String(TimeZone.current.secondsFromGMT() / 60)
        let base: [String: String?] = ["account": scope.account, "project": scope.project, "session": scope.session, "since": since]
        async let s: Summary = client.get("/api/summary", query: base.merging(["tz": tz]) { $1 })
        async let p: [ProjectRow] = client.get("/api/projects", query: ["account": scope.account, "since": since])
        async let ss: [ProxySessionRow] = client.get("/api/sessions", query: ["account": scope.account, "project": scope.project, "since": since])
        async let t: ToolsPayload = client.get("/api/tools", query: base)
        async let r: [RequestRow] = client.get("/api/requests", query: base)
        // Each child awaited on its own: a tuple `try await` that throws leaves the rest
        // to be torn down out of order, which the runtime aborts on (seen on this page
        // with real data). Nothing here may leave a child pending.
        var problems: [String] = []
        do { summary = try await s } catch { problems.append(LiveFeed.describe(error)) }
        do { projects = try await p } catch { problems.append(LiveFeed.describe(error)) }
        do { sessions = try await ss } catch { problems.append(LiveFeed.describe(error)) }
        do { tools = try await t } catch { problems.append(LiveFeed.describe(error)) }
        do { requests = try await r } catch { problems.append(LiveFeed.describe(error)) }
        failure = problems.first
        if problems.isEmpty { lastRefresh = .now }
    }

    func poll(every seconds: Double = 10) async {
        while !Task.isCancelled {
            await refresh()
            do { try await Task.sleep(for: .seconds(seconds)) } catch { return }
        }
    }
}
