import XCTest

/// A look at every screen against a real router, for a person to read: the app adds the router
/// named by `TEST_RUNNER_ROUTER_BASE` (a full https:// address) with the credential in
/// `TEST_RUNNER_ROUTER_PASSWORD` (`--ui-test-router`), then the test opens each screen and keeps
/// a screenshot of it, which `xcrun xcresulttool export attachments` lays out. Skipped without
/// them, so a plain run of the suite never reaches for a network.
@MainActor
final class TourUITests: XCTestCase {
    func testTourOfTheRouter() throws {
        let env = ProcessInfo.processInfo.environment
        guard let base = env["ROUTER_BASE"], let password = env["ROUTER_PASSWORD"] else {
            throw XCTSkip("set TEST_RUNNER_ROUTER_BASE and TEST_RUNNER_ROUTER_PASSWORD to tour a router")
        }
        // Handed to the app rather than typed: a credential typed into the form arrived wrong.
        let app = XCUIApplication()
        app.launchArguments = ["--ui-test-router"]
        app.launchEnvironment = ["UI_TEST_ROUTER_BASE": base, "UI_TEST_ROUTER_PASSWORD": password]
        app.launch()
        let any = app.descendants(matching: .any)

        let row = any.matching(identifier: "router-row").firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10), "no router row")
        sleep(6)
        shoot(app, "01 routers")
        row.tap()

        XCTAssertTrue(any.matching(identifier: "live-sessions").firstMatch.waitForExistence(timeout: 20), "the router page did not load")
        sleep(5)
        shoot(app, "02 router")
        for i in 3...6 {
            app.swipeUp()
            shoot(app, String(format: "%02d router, further down", i))
        }

        // The live sessions, a Kimi Code one when the list has one, and the start form.
        let live = any.matching(identifier: "live-sessions").firstMatch
        for _ in 0..<6 where !live.isHittable { app.swipeDown() }
        live.tap()
        XCTAssertTrue(any.matching(identifier: "session-row").firstMatch.waitForExistence(timeout: 20), "no session listed")
        shoot(app, "15 sessions")
        let kimi = app.buttons.matching(NSPredicate(format: "label CONTAINS 'Kimi Code'")).firstMatch
        if kimi.waitForExistence(timeout: 3) {
            kimi.tap()
            sleep(4)
            shoot(app, "16 a Kimi Code session")
            app.navigationBars.buttons.firstMatch.tap()
        }
        let start = any.matching(identifier: "start-session").firstMatch
        if start.waitForExistence(timeout: 5) {
            start.tap()
            sleep(4)
            shoot(app, "17 start a session")
            app.buttons["Cancel"].firstMatch.tap()
        }
        app.navigationBars.buttons.firstMatch.tap()
        let machines = any.matching(identifier: "machines").firstMatch
        if machines.waitForExistence(timeout: 5) {
            machines.tap()
            sleep(4)
            shoot(app, "18 machines")
            app.swipeUp()
            shoot(app, "19 machines, further down")
            app.navigationBars.buttons.firstMatch.tap()
        }

        // The Library and an item of it.
        let library = any.matching(identifier: "library").firstMatch
        if library.waitForExistence(timeout: 5) {
            library.tap()
            sleep(5)
            shoot(app, "20 library")
            // The item the router builds in, there on every hosted router.
            let item = app.staticTexts["agent-router"].firstMatch
            if item.waitForExistence(timeout: 5) {
                item.tap()
                if app.staticTexts["Versions"].waitForExistence(timeout: 10) {
                    sleep(3)
                    shoot(app, "21 a library item")
                    app.swipeUp()
                    shoot(app, "22 a library item, further down")
                    app.navigationBars.buttons.firstMatch.tap()
                }
            }
            app.navigationBars.buttons.firstMatch.tap()
        }

        // Settings, and each of its pages.
        let settings = any.matching(identifier: "settings").firstMatch
        XCTAssertTrue(any.matching(identifier: "live-sessions").firstMatch.waitForExistence(timeout: 10), "back on the router page")
        for _ in 0..<10 where !settings.isHittable { app.swipeUp() }
        if settings.isHittable {
            settings.tap()
            sleep(2)
            shoot(app, "23 settings")
            for (i, page) in ["models", "access-tokens", "router", "password", "notifications", "clients", "forges", "library"].enumerated() {
                let link = any.matching(identifier: "settings-\(page)").firstMatch
                guard link.waitForExistence(timeout: 5) else { continue }
                link.tap()
                sleep(4)
                shoot(app, String(format: "%02d settings %@", 24 + i, page))
                app.navigationBars.buttons.firstMatch.tap()
            }
            app.navigationBars.buttons.firstMatch.tap()
        }
        for _ in 0..<8 { app.swipeDown() }

        let usage = any.matching(identifier: "usage").firstMatch
        for _ in 0..<6 where !usage.isHittable { app.swipeDown() }
        usage.tap()
        XCTAssertTrue(any.matching(identifier: "usage-range").firstMatch.waitForExistence(timeout: 10))
        // All of history is slow to sum: wait for the totals rather than a fixed time.
        XCTAssertTrue(app.staticTexts["Totals in scope"].waitForExistence(timeout: 60), "the usage page did not load")
        shoot(app, "07 usage")
        for i in 8...11 {
            app.swipeUp()
            shoot(app, String(format: "%02d usage, further down", i))
        }
        app.navigationBars.buttons.firstMatch.tap()

        let manage = any.matching(identifier: "manage-accounts").firstMatch
        for _ in 0..<8 where !manage.isHittable { app.swipeUp() }
        if manage.isHittable {
            manage.tap()
            sleep(4)
            shoot(app, "12 accounts")
            app.swipeUp()
            shoot(app, "13 accounts, further down")
            app.navigationBars.buttons.matching(identifier: "Add account").firstMatch.tap()
            sleep(1)
            shoot(app, "14 add account")
        }
    }

    private func shoot(_ app: XCUIApplication, _ name: String) {
        let a = XCTAttachment(screenshot: app.screenshot())
        a.name = name
        a.lifetime = .keepAlways
        add(a)
    }
}
