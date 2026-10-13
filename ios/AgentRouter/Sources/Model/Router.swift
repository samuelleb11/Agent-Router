// The routers this phone knows about. Addresses in a JSON file, passwords in the
// Keychain, keyed by the router's id so a rename never loses the password.

import Foundation
import Observation

struct Router: Codable, Identifiable, Hashable, Sendable {
    /// Plain HTTP for a router on the LAN; HTTPS for a hosted one, behind an ingress.
    enum Scheme: String, Codable, Sendable {
        case http, https

        /// The port a URL that names none means.
        var defaultPort: Int { self == .https ? 443 : 80 }
    }

    var id = UUID()
    var name: String
    var host: String
    var port: Int = 8787
    /// Optional because every router saved before https has no such key, and a
    /// synthesized decoder refuses a missing non-optional one, which would lose the whole
    /// list. Nil is plain HTTP, as those routers always were; nil is also left out when
    /// the list is saved again.
    var scheme: Scheme? = nil

    /// `host:port` for a router on the LAN, as always; a hosted router shows its URL.
    var address: String { scheme == .https ? origin : "\(host):\(port)" }
    var baseURL: URL? { URL(string: origin) }

    /// The address as URL text. Plain HTTP is built exactly as it was before https,
    /// the port always written; https leaves out 443.
    var origin: String {
        let name = host.trimmingCharacters(in: .whitespaces)
        switch scheme ?? .http {
        case .http: return "http://\(name):\(port)"
        case .https: return port == 443 ? "https://\(name)" : "https://\(name):\(port)"
        }
    }
}

extension Router {
    /// What the add and edit form's Host and Port fields name together.
    enum Entry: Equatable, Sendable {
        /// Nothing typed yet.
        case empty
        /// Typed, and unusable: why, in a sentence.
        case invalid(String)
        case valid(scheme: Scheme, host: String, port: Int)
    }

    /// A whole URL with its scheme (https://agent-router.example.com,
    /// http://10.0.0.5:8787) decides everything itself, its port included: the scheme's
    /// own when it names none, the Port field ignored. A bare host is plain HTTP on the
    /// port beside it, as every router was before https.
    static func entry(host text: String, port portText: String) -> Entry {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return .empty }
        let scheme: Scheme, host: String, port: Int
        if text.contains("://") {
            guard let url = URLComponents(string: text) else { return .invalid("That is not a usable URL.") }
            guard let s = url.scheme.flatMap({ Scheme(rawValue: $0.lowercased()) }) else {
                return .invalid("Only http:// and https:// addresses work.")
            }
            guard let h = url.host, !h.isEmpty else { return .invalid("That URL names no host.") }
            guard url.user == nil, url.password == nil else {
                return .invalid("Leave the password out of the address; it goes in the Password field.")
            }
            guard url.path.isEmpty || url.path == "/", url.query == nil, url.fragment == nil else {
                return .invalid("Leave out the path: the router answers at the root of its address.")
            }
            scheme = s
            host = h
            port = url.port ?? s.defaultPort
        } else {
            guard let p = Int(portText.trimmingCharacters(in: .whitespaces)) else {
                return .invalid("The port must be a number from 1 to 65535.")
            }
            scheme = .http
            host = text
            port = p
        }
        guard (1...65535).contains(port) else { return .invalid("The port must be a number from 1 to 65535.") }
        // What is saved must read back as exactly this address. Otherwise a "/", "?", "@"
        // or ":" typed into a bare host would be saved, and quietly point elsewhere.
        let origin = Router(name: "", host: host, port: port, scheme: scheme).origin
        guard URL(string: origin) != nil, let back = URLComponents(string: origin),
              back.host == host, back.port == (scheme == .https && port == 443 ? nil : port),
              back.path.isEmpty, back.query == nil, back.fragment == nil, back.user == nil
        else {
            return .invalid(scheme == .http && text.contains(":") && !text.contains("://")
                            ? "Put the port in the Port field, or type the whole URL, starting http:// or https://."
                            : "That is not a usable host name or address.")
        }
        return .valid(scheme: scheme, host: host, port: port)
    }
}

/// Where the stack can go. Declared as values so every destination is registered once,
/// at the root — a destination declared inside a pushed, re-rendering screen is exactly
/// the arrangement SwiftUI resolves unreliably.
struct SessionsRoute: Hashable { let routerId: UUID }
/// A hosted router's machines, and how to connect one.
struct MachinesRoute: Hashable { let routerId: UUID }
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
struct LibraryRoute: Hashable { let routerId: UUID }
struct LibraryItemRoute: Hashable {
    let routerId: UUID
    let kind: String
    let name: String
}
/// One page of a router's Settings, as the console names its pages.
struct SettingsPageRoute: Hashable {
    let routerId: UUID
    let page: SettingsPage
}

enum SettingsPage: String, Hashable, Sendable {
    case models
    case accessTokens = "access-tokens"
}
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
