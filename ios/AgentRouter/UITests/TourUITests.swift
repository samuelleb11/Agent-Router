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
