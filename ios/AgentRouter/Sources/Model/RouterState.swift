// Everything the app knows about one router right now, and how it keeps that current.

import Foundation
import Observation

@MainActor
@Observable
final class RouterState {
    private(set) var router: Router
    @ObservationIgnored private var client: RouterClient?

    var health: Health?
    var auth: AuthStatus?
    var accounts: [Account] = []
    var forecast: Forecast?
    var quota: Quota?
    var sessions: [LiveSession] = []
    /// Why the last listing failed, when it did — the page must say so rather than
    /// show an empty week.
    var sessionsFailure: String?
    var shim: ShimStatus?
    var failure: RouterClient.Failure?
    var lastRefresh: Date?
    var refreshing = false
    /// The account id whose activation is in flight, for the row to show progress.
    var switching: String?

    init(router: Router, password: String?) {
        self.router = router
        apply(router: router, password: password)
    }

    func apply(router: Router, password: String?) {
        self.router = router
        client = router.baseURL.map { RouterClient(baseURL: $0, password: password) }
    }

    var isReachable: Bool { health?.ok == true && failure == nil }
    var activeAccount: Forecast.ForecastAccount? { forecast?.accounts.first(where: \.isActive) }
    /// Accounts to offer in the switcher: everything the router lists, in its order.
    var switchable: [Forecast.ForecastAccount] { forecast?.accounts ?? [] }

    func credential(for id: String) -> Account? { accounts.first { $0.id == id } }
    var activeSessions: Int { sessions.filter(\.active).count }
    /// Sessions at work and sessions waiting on you, when the router names each session's
    /// activity. Nil from an older router, whose only word for either is `active`.
    var sessionCounts: (working: Int, waiting: Int)? {
        guard sessions.contains(where: { $0.activity != nil }) else { return nil }
        return (sessions.filter(\.isWorking).count, sessions.filter(\.needsYou).count)
    }
    /// "2 working · 1 waiting", or from an older router "3 active"; nil when nothing is.
    /// An ended session is neither, so it is never counted.
    var liveSummary: String? {
        guard let c = sessionCounts else { return activeSessions > 0 ? "\(activeSessions) active" : nil }
        let parts = [c.working > 0 ? "\(c.working) working" : nil, c.waiting > 0 ? "\(c.waiting) waiting" : nil]
        let said = parts.compactMap { $0 }
        return said.isEmpty ? nil : said.joined(separator: " · ")
    }
    /// For the pages that stream: the same address and password this state uses.
    var liveClient: RouterClient? { client }

    /// `/api/accounts` makes the router resolve every credential, so it is asked for
    /// less often than the rest — on demand, and once a minute while the page is open.
    func refresh(includeAccounts: Bool) async {
        guard let client else {
            failure = .unreachable("bad address")
            return
        }
        refreshing = true
        defer { refreshing = false }
        async let h: Health = client.get("/_router/health")
        async let a: AuthStatus = client.get("/api/auth")
        async let f: Forecast = client.get("/api/forecast")
        async let q: Quota = client.get("/api/quota")
        async let s: [LiveSession] = client.get("/api/live/sessions")
        var first: (any Error)?
        do { health = try await h } catch { first = first ?? error }
        do { auth = try await a } catch { first = first ?? error }
        do { forecast = try await f } catch { first = first ?? error }
        do { quota = try await q } catch { first = first ?? error }
        // Its own failure: a slow or refused listing must not blank the page.
        do { sessions = try await s; sessionsFailure = nil } catch { sessionsFailure = LiveFeed.describe(error) }
        if first == nil, includeAccounts || accounts.isEmpty {
            do { accounts = try await client.get("/api/accounts") } catch { first = error }
            shim = try? await client.get("/api/setup/shim")
        }
        if let e = first {
            failure = (e as? RouterClient.Failure) ?? .unreachable(e.localizedDescription)
        } else {
            failure = nil
            lastRefresh = .now
        }
    }

    func activate(_ id: String) async {
        guard let client, id != activeAccount?.id else { return }
        switching = id
        defer { switching = nil }
        do {
            let _: ActivateResponse = try await client.post("/api/accounts/activate", ["id": id])
            await refresh(includeAccounts: true)
        } catch let e as RouterClient.Failure {
            failure = e
        } catch {
            failure = .unreachable(error.localizedDescription)
        }
    }

    /// Just the session list, for the sessions page's faster cadence.
    func refreshSessions() async {
        guard let client else { return }
        do {
            sessions = try await client.get("/api/live/sessions")
            sessionsFailure = nil
        } catch {
            sessionsFailure = LiveFeed.describe(error)
        }
    }

    func archive(_ session: LiveSession, _ archived: Bool) async -> String? {
        guard let client else { return "bad address" }
        do {
            let _: LiveReply = try await client.post("/api/live/archive",
                ArchiveBody(session: session.sessionId, archived: archived))
            await refreshSessions()
            return nil
        } catch {
            return (error as? RouterClient.Failure)?.errorDescription ?? error.localizedDescription
        }
    }

    /// Runs until the task that started it is cancelled — the page going away.
    func poll(every seconds: Double = 10) async {
        var n = 0
        while !Task.isCancelled {
            await refresh(includeAccounts: n % 6 == 0)
            n += 1
            do { try await Task.sleep(for: .seconds(seconds)) } catch { return }
        }
    }
}
