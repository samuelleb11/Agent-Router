// This phone's side of notifications: asking iOS, holding Apple's token, and telling
// every router about it. The router does the rest.

import Foundation
import Observation
import UIKit
import UserNotifications

@MainActor
@Observable
final class PushRegistrar {
    static let shared = PushRegistrar()

    /// Apple's device token, hex. Persisted so a router added later can be told.
    private(set) var token: String? = UserDefaults.standard.string(forKey: "apns-token")
    var authorization: UNAuthorizationStatus = .notDetermined
    /// Why iOS would not hand over a token — a build without the entitlement, mostly.
    var registrationError: String?
    /// Per router: the last registration result.
    var routers: [UUID: String] = [:]
    @ObservationIgnored weak var store: RouterStore? { didSet { flush() } }
    /// A tap that arrived before the store was attached — a cold launch from the
    /// notification itself does that.
    @ObservationIgnored private var pending: PendingOpen?

    /// Development builds register with Apple's sandbox; the router must send there.
    var environment: String {
        #if DEBUG
        "development"
        #else
        "production"
        #endif
    }

    func refreshAuthorization() async {
        authorization = await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
    }

    /// Ask iOS, then Apple. The token arrives through the app delegate.
    func enable() async {
        let granted = (try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])) ?? false
        await refreshAuthorization()
        if granted {
            registrationError = nil
            UIApplication.shared.registerForRemoteNotifications()
        }
    }

    func didRegister(deviceToken: Data) {
        let hex = deviceToken.map { String(format: "%02x", $0) }.joined()
        token = hex
        UserDefaults.standard.set(hex, forKey: "apns-token")
        registrationError = nil
        Task { await registerEverywhere() }
    }

    func didFail(_ error: Error) {
        registrationError = error.localizedDescription
    }

    func registerEverywhere() async {
        guard let store else { return }
        for r in store.routers {
            _ = await register(with: store.state(for: r))
        }
    }

    /// Tell one router about this phone. Returns what went wrong, or nil.
    func register(with state: RouterState) async -> String? {
        guard let token else { return "no device token yet" }
        guard let client = state.liveClient else { return "bad address" }
        let body = PushRegisterBody(token: token, name: UIDevice.current.name, environment: environment,
                                    routerRef: state.router.id.uuidString,
                                    teamId: Self.teamId, bundleId: Bundle.main.bundleIdentifier)
        do {
            let reply: OkReply = try await client.post("/api/push/register", body)
            let result = reply.problem ?? "registered"
            routers[state.router.id] = result
            return reply.problem
        } catch {
            let why = LiveFeed.describe(error)
            routers[state.router.id] = why
            return why
        }
    }

    /// The team this build is signed under, from `AppIdentifierPrefix` in Info.plist
    /// (`TEAMID.`). Nil on an unsigned build, where Xcode leaves it empty.
    static var teamId: String? {
        guard let raw = Bundle.main.object(forInfoDictionaryKey: "AppIdentifierPrefix") as? String else { return nil }
        let id = raw.trimmingCharacters(in: CharacterSet(charactersIn: ". "))
        return id.isEmpty || id.contains("$") ? nil : id
    }

    /// What a tapped notification asks for: the router it came from and the session.
    nonisolated static func destination(of userInfo: [AnyHashable: Any]) -> PendingOpen? {
        guard let ref = userInfo["router_ref"] as? String, let id = UUID(uuidString: ref),
              let session = userInfo["session"] as? String, !session.isEmpty else { return nil }
        return PendingOpen(routerId: id, sessionId: session)
    }

    func consume(_ open: PendingOpen) {
        pending = open
        flush()
    }

    private func flush() {
        guard let store, let open = pending else { return }
        pending = nil
        store.pendingOpen = open
    }
}
