// Managing a router's accounts from the phone: the dashboard's settings table.

import Foundation
import Observation

@MainActor
@Observable
final class AccountsState {
    let router: RouterState
    var accounts: [Account] = []
    var failure: String?
    var busy = false

    init(router: RouterState) { self.router = router }

    private var client: RouterClient? { router.liveClient }

    /// `probe` makes the router check every credential, which is not free; without it a row
    /// carries no verdict. Asked for on open, on a pull and after a change.
    func load(probe: Bool) async {
        guard let client else { return }
        do {
            accounts = try await client.get("/api/accounts", query: ["probe": probe ? "1" : nil])
            failure = nil
        } catch {
            failure = LiveFeed.describe(error)
        }
    }

    /// Run one change, then re-read everything that depends on it. Returns what went
    /// wrong, or nil.
    private func change(_ op: (RouterClient) async throws -> OkReply) async -> String? {
        guard let client else { return "That router has no usable address." }
        busy = true
        defer { busy = false }
        do {
            let reply = try await op(client)
            if let p = reply.problem { return p }
        } catch {
            return LiveFeed.describe(error)
        }
        await load(probe: true)
        await router.refresh(includeAccounts: true)
        return nil
    }

    func activate(_ id: String) async -> String? {
        await change { try await $0.post("/api/accounts/activate", AccountRefBody(id: id)) }
    }

    func remove(_ id: String) async -> String? {
        await change { try await $0.post("/api/accounts/remove", AccountRefBody(id: id)) }
    }

    func edit(_ body: AccountEditBody) async -> String? {
        await change { try await $0.post("/api/accounts/edit", body) }
    }

    func usageKey(_ id: String, token: String?, clear: Bool) async -> String? {
        await change { try await $0.post("/api/usage-token", UsageTokenBody(accountId: id, token: token, clear: clear ? true : nil)) }
    }

    /// A token or profile account. Sign-in goes through `loginStart` instead.
    func add(label: String, source: String, token: String?, provider: String? = nil) async -> String? {
        await change { try await $0.post("/api/accounts", AddAccountBody(label: label, source: source, token: token, provider: provider)) }
    }

    func probe(_ id: String) async -> ProbeReply? {
        guard let client else { return nil }
        return try? await client.post("/api/accounts/probe", AccountRefBody(id: id))
    }

    /// The router runs `claude setup-token` on the Mac and reports a sign-in link.
    func loginStart(label: String) async -> LoginStatus? {
        guard let client else { return nil }
        do { return try await client.post("/api/login/start", LoginStartBody(label: label)) }
        catch { return LoginStatus(phase: nil, url: nil, label: label, accountId: nil, email: nil, error: LiveFeed.describe(error), tail: nil) }
    }

    func loginStatus() async -> LoginStatus? {
        guard let client else { return nil }
        return try? await client.get("/api/login/status")
    }
}

/// The router's setup, from the phone: what the dashboard's settings modal controls.
@MainActor
@Observable
final class SettingsState {
    let router: RouterState
    var setup: SetupStatus?
    var shim: ShimStatus?
    var auth: AuthStatus?
    var push: PushStatus?
    var failure: String?
    var busy: String?
    var message: String?

    init(router: RouterState) { self.router = router }

    private var client: RouterClient? { router.liveClient }

    func load() async {
        guard let client else { return }
        async let a: SetupStatus = client.get("/api/setup")
        async let b: ShimStatus = client.get("/api/setup/shim")
        async let c: AuthStatus = client.get("/api/auth")
        async let d: PushStatus = client.get("/api/push")
        var problems: [String] = []
        do { setup = try await a } catch { problems.append(LiveFeed.describe(error)) }
        do { shim = try await b } catch { problems.append(LiveFeed.describe(error)) }
        do { auth = try await c } catch { problems.append(LiveFeed.describe(error)) }
        do { push = try await d } catch { problems.append(LiveFeed.describe(error)) }
        failure = problems.first
    }

    private func change(_ what: String, _ op: (RouterClient) async throws -> OkReply) async {
        guard let client else { return }
        busy = what
        defer { busy = nil }
        do {
            let reply = try await op(client)
            message = reply.problem ?? "Done."
        } catch {
            message = LiveFeed.describe(error)
        }
        await load()
        await router.refresh(includeAccounts: true)
    }

    func service(enable: Bool) async {
        await change("service") { try await $0.post("/api/setup/service", ServiceBody(enable: enable)) }
    }

    func config(enable: Bool, project: String?) async {
        await change(project ?? "global") { try await $0.post("/api/setup/config", ConfigBody(enable: enable, project: project)) }
    }

    func shimAuto(_ enabled: Bool) async {
        await change("shim-auto") { try await $0.post("/api/setup/shim/auto", ShimAutoBody(enabled: enabled)) }
    }

    func shim(path: String, action: String) async {
        await change(path) { try await $0.post("/api/setup/shim", ShimBody(path: path, action: action)) }
    }

    func password(_ new: String?, clear: Bool) async {
        await change("password") { try await $0.post("/api/auth", PasswordBody(password: new, clear: clear ? true : nil)) }
    }

    func pushConfig(teamId: String, keyId: String, p8: String, bundleId: String?) async {
        await change("push") { try await $0.post("/api/push/config", PushConfigBody(teamId: teamId, keyId: keyId, p8: p8, bundleId: bundleId, clear: nil)) }
    }

    func pushClear() async {
        await change("push") { try await $0.post("/api/push/config", PushConfigBody(teamId: "", keyId: "", p8: "", bundleId: nil, clear: true)) }
    }

    func pushTest() async {
        guard let client else { return }
        busy = "push-test"
        defer { busy = nil }
        do {
            let r: PushTestReply = try await client.post("/api/push/test", [String: String]())
            if let e = r.error { message = e }
            else if let lines = r.results, !lines.isEmpty {
                message = lines.map { "…\($0.device): \($0.ok ? "delivered" : ($0.error ?? "failed"))" }.joined(separator: " · ")
            } else { message = "No phone is registered with this router yet." }
        } catch { message = LiveFeed.describe(error) }
        await load()
    }
}
