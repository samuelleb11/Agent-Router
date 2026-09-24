// The routers this phone knows about. Addresses in a JSON file, passwords in the
// Keychain, keyed by the router's id so a rename never loses the password.

import Foundation
import Observation

struct Router: Codable, Identifiable, Hashable, Sendable {
    var id = UUID()
    var name: String
    var host: String
    var port: Int = 8787

    var address: String { "\(host):\(port)" }
    var baseURL: URL? { URL(string: "http://\(host.trimmingCharacters(in: .whitespaces)):\(port)") }
}

/// Where the stack can go. Declared as values so every destination is registered once,
/// at the root — a destination declared inside a pushed, re-rendering screen is exactly
/// the arrangement SwiftUI resolves unreliably.
struct SessionsRoute: Hashable { let routerId: UUID }
struct SessionRoute: Hashable {
    let routerId: UUID
    let session: LiveSession
}
struct UsageRoute: Hashable {
    let routerId: UUID
    let scope: UsageScope
}
struct UsageListRoute: Hashable {
    enum Kind: Hashable { case projects, sessions, tools, requests }
    let routerId: UUID
    let scope: UsageScope
    let kind: Kind
}
struct RequestRoute: Hashable {
    let routerId: UUID
    let request: RequestRow
}
struct AccountsRoute: Hashable { let routerId: UUID }
/// Search is per router: the index belongs to one store, and there is nothing to ask
/// for a view across several.
struct SearchRoute: Hashable { let routerId: UUID }
struct SettingsRoute: Hashable { let routerId: UUID }
/// A session named only by id — what a notification carries; the view looks it up.
struct SessionByIdRoute: Hashable {
    let routerId: UUID
    let sessionId: String
}

/// Where a tapped notification wants the app to go.
struct PendingOpen: Equatable, Sendable {
    let routerId: UUID
    let sessionId: String
}

@MainActor
@Observable
final class RouterStore {
    private(set) var routers: [Router] = []
    /// Set by a tapped notification; the root stack consumes it.
    var pendingOpen: PendingOpen?
    /// Live state per router. Not observed: it is created on demand from inside view
    /// bodies, and a tracked mutation there would re-render the view that caused it.
    @ObservationIgnored private var states: [UUID: RouterState] = [:]
    @ObservationIgnored private var usages: [UsageKey: UsageState] = [:]
    private struct UsageKey: Hashable { let router: UUID; let scope: UsageScope }
    @ObservationIgnored private let file: URL

    init() {
        let dir = (try? FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask,
                                                 appropriateFor: nil, create: true))
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        file = dir.appending(path: "routers.json")
        load()
    }

    func add(_ router: Router, password: String) {
        try? Keychain.set(password, for: router.id.uuidString)
        routers.append(router)
        save()
        // A new router should hear about this phone too.
        let state = state(for: router)
        Task { _ = await PushRegistrar.shared.register(with: state) }
    }

    /// A blank password keeps the one already stored.
    func update(_ router: Router, password: String) {
        if !password.isEmpty { try? Keychain.set(password, for: router.id.uuidString) }
        if let i = routers.firstIndex(where: { $0.id == router.id }) { routers[i] = router }
        states[router.id]?.apply(router: router, password: self.password(for: router))
        save()
    }

    func remove(at offsets: IndexSet) {
        for i in offsets {
            let r = routers[i]
            Keychain.delete(r.id.uuidString)
            states[r.id] = nil
            usages = usages.filter { $0.key.router != r.id }
        }
        routers.remove(atOffsets: offsets)
        save()
    }

    func password(for router: Router) -> String? {
        Keychain.get(router.id.uuidString)
    }

    func state(id: UUID) -> RouterState? {
        routers.first { $0.id == id }.map { state(for: $0) }
    }

    /// One usage page per router and scope, kept so the lists pushed from it share
    /// its data and its range rather than fetching their own.
    func usage(id: UUID, scope: UsageScope) -> UsageState? {
        guard let state = state(id: id) else { return nil }
        let key = UsageKey(router: id, scope: scope)
        if let u = usages[key] { return u }
        let u = UsageState(router: state, scope: scope)
        usages[key] = u
        return u
    }

    func state(for router: Router) -> RouterState {
        if let s = states[router.id] { return s }
        let s = RouterState(router: router, password: password(for: router))
        states[router.id] = s
        return s
    }

    private func load() {
        guard let data = try? Data(contentsOf: file),
              let list = try? JSONDecoder().decode([Router].self, from: data) else { return }
        routers = list
    }

    private func save() {
        guard let data = try? JSONEncoder().encode(routers) else { return }
        try? data.write(to: file, options: .atomic)
    }
}
