import XCTest

/// A router's push, tapped: the app opens on that session, whether it was in the
/// background or not running at all. The notification is a local one carrying the same
/// payload, scheduled by the app's `--ui-test-notify` hook, since nothing can deliver an
/// APNs push to the simulator. Needs a router at 127.0.0.1:8799 with sessions to list.
@MainActor
final class NotificationTapUITests: XCTestCase {
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    func testATapWithTheAppInTheBackgroundOpensTheSession() throws {
        let session = try firstSessionId()
        ensureRouter()
        let app = launch(notifying: session, bogusRouter: false)
        allowNotificationsIfAsked()
        XCUIDevice.shared.press(.home)
        tapNotification()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10), "the tap did not bring the app forward")
        assertSessionOpen(app)
    }

    func testATapWithTheAppNotRunningOpensTheSession() throws {
        let session = try firstSessionId()
        ensureRouter()
        // The payload names a router this phone does not know; with one router that is
        // still unambiguous, and it is what a removed-and-re-added router looks like.
        let app = launch(notifying: session, bogusRouter: true)
        allowNotificationsIfAsked()
        XCUIDevice.shared.press(.home)
        app.terminate() // the scheduled trigger outlives the process
        tapNotification()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15), "the tap did not launch the app")
        assertSessionOpen(app)
    }

    func testATapWhileTheAppIsOpenGoesToTheSession() throws {
        let session = try firstSessionId()
        ensureRouter()
        let app = launch(notifying: session, bogusRouter: false)
        allowNotificationsIfAsked()
        // The app stays in front: the banner is the one the delegate chose to show.
        tapNotification()
        assertSessionOpen(app)
    }

    // MARK: - steps

    private func launch(notifying session: String, bogusRouter: Bool) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["--ui-test-notify", session] + (bogusRouter ? ["--ui-test-bogus-router"] : [])
        app.launch()
        return app
    }

    /// The same add-router flow the navigation test uses, then the app is quit so the
    /// next launch's hook sees the router.
    private func ensureRouter() {
        let app = XCUIApplication()
        app.launch()
        let any = app.descendants(matching: .any)
        if !any.matching(identifier: "router-row").firstMatch.waitForExistence(timeout: 3) {
            app.buttons["Add router"].firstMatch.tap()
            let name = any.matching(identifier: "form-name").firstMatch
            XCTAssertTrue(name.waitForExistence(timeout: 5))
            name.tap(); name.typeText("Test")
            let host = any.matching(identifier: "form-host").firstMatch
            host.tap(); host.typeText("127.0.0.1")
            let port = any.matching(identifier: "form-port").firstMatch
            port.tap(); port.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 4) + "8799")
            let pw = any.matching(identifier: "form-password").firstMatch
            pw.tap(); pw.typeText("anything1")
            any.matching(identifier: "form-save").firstMatch.tap()
            XCTAssertTrue(any.matching(identifier: "router-row").firstMatch.waitForExistence(timeout: 5), "no router row")
        }
        app.terminate()
    }

    private func allowNotificationsIfAsked() {
        let allow = springboard.alerts.firstMatch.buttons["Allow"]
        if allow.waitForExistence(timeout: 8) { allow.tap() }
    }

    /// The banner is SpringBoard's; its label carries the notification's text. A banner
    /// that was missed is still in Notification Center, which a drag from the top opens.
    private func tapNotification() {
        let cell = springboard.descendants(matching: .any)
            .matching(NSPredicate(format: "label CONTAINS[c] %@", "UI test prompt")).firstMatch
        if !cell.waitForExistence(timeout: 22) {
            shoot(springboard, "no banner on screen; opening Notification Center")
            let top = springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.02))
            top.press(forDuration: 0.1, thenDragTo: springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.75)))
            _ = cell.waitForExistence(timeout: 5)
        }
        shoot(springboard, "about to tap the notification")
        XCTAssertTrue(cell.exists, "the notification is nowhere on screen")
        cell.tap()
    }

    private func assertSessionOpen(_ app: XCUIApplication) {
        let composer = app.descendants(matching: .any).matching(identifier: "composer").firstMatch
        let opened = composer.waitForExistence(timeout: 25)
        shoot(app, "after the tap")
        XCTAssertTrue(opened, "the session did not open — the app is on: \(app.navigationBars.firstMatch.identifier)")
    }

    /// A session to name in the payload: the first one the router lists.
    private func firstSessionId() throws -> String {
        let url = URL(string: "http://127.0.0.1:8799/api/live/sessions")!
        let done = DispatchSemaphore(value: 0)
        var data: Data?
        URLSession.shared.dataTask(with: url) { d, _, _ in data = d; done.signal() }.resume()
        _ = done.wait(timeout: .now() + 10)
        let json = try JSONSerialization.jsonObject(with: try XCTUnwrap(data, "the test router at 127.0.0.1:8799 did not answer"))
        let list = (json as? [[String: Any]]) ?? ((json as? [String: Any])?["sessions"] as? [[String: Any]]) ?? []
        return try XCTUnwrap(list.first?["session_id"] as? String, "the test router lists no sessions")
    }

    private func shoot(_ app: XCUIApplication, _ name: String) {
        let a = XCTAttachment(screenshot: app.screenshot())
        a.name = name
        a.lifetime = .keepAlways
        add(a)
    }
}
