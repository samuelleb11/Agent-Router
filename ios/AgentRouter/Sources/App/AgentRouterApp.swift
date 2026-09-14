import SwiftUI
import UserNotifications

@main
struct AgentRouterApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @State private var store: RouterStore

    init() {
        let store = RouterStore()
        _store = State(initialValue: store)
        // Attached before any view exists: a tap that launches the app is delivered
        // before the first view's task runs, and must not find nobody listening.
        PushRegistrar.shared.store = store
    }

    var body: some Scene {
        WindowGroup {
            RoutersListView()
                .environment(store)
                .task {
                    await PushRegistrar.shared.refreshAuthorization()
                    // Already allowed once: fetch a fresh token and re-register quietly.
                    if PushRegistrar.shared.authorization == .authorized {
                        UIApplication.shared.registerForRemoteNotifications()
                    }
                    #if DEBUG
                    await UITestHooks.run(store: store)
                    #endif
                }
        }
    }
}

#if DEBUG
/// `--ui-test-notify <session id>` schedules a local notification carrying the same
/// payload a router's push does, so a UI test can tap it: nothing can deliver an APNs
/// push to the simulator. `--ui-test-bogus-router` names a router the phone does not
/// know, to exercise the only-router fallback. Debug builds only; nothing else uses it.
enum UITestHooks {
    @MainActor
    static func run(store: RouterStore) async {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "--ui-test-notify"), i + 1 < args.count else { return }
        let session = args[i + 1]
        let ref = args.contains("--ui-test-bogus-router") ? UUID().uuidString : (store.routers.first?.id.uuidString ?? "")
        let center = UNUserNotificationCenter.current()
        let granted = (try? await center.requestAuthorization(options: [.alert, .sound])) ?? false
        let settings = await center.notificationSettings()
        note("auth granted=\(granted) status=\(settings.authorizationStatus.rawValue) alerts=\(settings.alertSetting.rawValue)")
        let content = UNMutableNotificationContent()
        content.title = "Agent Router"
        content.body = "UI test prompt"
        content.sound = .default
        content.userInfo = ["router_ref": ref, "session": session, "request_id": "ui-test"]
        // Late enough for the test to answer the permission alert and leave the app first.
        let trigger = UNTimeIntervalNotificationTrigger(timeInterval: 12, repeats: false)
        do {
            try await center.add(UNNotificationRequest(identifier: "ui-test", content: content, trigger: trigger))
            note("scheduled in 12 s: session \(session) router \(ref)")
        } catch {
            note("schedule failed: \(error)")
        }
    }

    /// What the hook did, for the Mac to read out of the simulator's container afterwards.
    static func note(_ line: String) {
        let url = URL.documentsDirectory.appending(path: "ui-test-notify.log")
        let data = Data("\(Date()) \(line)\n".utf8)
        if let h = try? FileHandle(forWritingTo: url) {
            h.seekToEndOfFile(); h.write(data); try? h.close()
        } else {
            try? data.write(to: url)
        }
    }
}
#endif

/// Apple's token and notification taps arrive here and nowhere else.
@MainActor
final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    func application(_ application: UIApplication,
                     didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        PushRegistrar.shared.didRegister(deviceToken: deviceToken)
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        PushRegistrar.shared.didFail(error)
    }

    // Both delegate methods are the completion-handler forms, completed on the main
    // queue, and not the `async` forms. The async forms looked cleaner and killed the app
    // on every tap: Swift's bridging invokes the completion from a background executor,
    // and UIKit does state-restoration work inside that completion which asserts it is
    // on the main thread (an NSAssertion, then abort — "the app does not open"). The
    // SDK declares neither method nor its handler as main-actor, so nothing warned.

    /// Show it even when the app is open — a prompt in another session is still news.
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter,
                                            willPresent notification: UNNotification,
                                            withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        let done = OnMain(completionHandler)
        DispatchQueue.main.async { done.call([.banner, .sound, .list]) }
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter,
                                            didReceive response: UNNotificationResponse,
                                            withCompletionHandler completionHandler: @escaping () -> Void) {
        // Read the destination here, on whatever thread this arrives on: the userInfo
        // dictionary is not Sendable, but what it names is.
        let open = PushRegistrar.destination(of: response.notification.request.content.userInfo)
        let done = OnMain(completionHandler)
        DispatchQueue.main.async {
            if let open { PushRegistrar.shared.consume(open) }
            done.call(())
        }
    }
}

/// A completion block carried to the main queue. The blocks UIKit hands over are not
/// marked Sendable; calling them once, on main, is the whole contract.
private struct OnMain<T>: @unchecked Sendable {
    let call: (T) -> Void
    init(_ call: @escaping (T) -> Void) { self.call = call }
}
