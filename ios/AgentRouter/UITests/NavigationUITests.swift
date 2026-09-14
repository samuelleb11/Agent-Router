import XCTest

/// Drives the app the way a thumb does: add a router, open it, open its sessions, open
/// one — and stay there. Needs a router at 127.0.0.1:8799 with sessions to list.
final class NavigationUITests: XCTestCase {
    func testASessionOpensAndStaysOpen() throws {
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
        }
        let row = any.matching(identifier: "router-row").firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 5), "no router row")
        row.tap()

        let link = any.matching(identifier: "live-sessions").firstMatch
        XCTAssertTrue(link.waitForExistence(timeout: 15), "the router page did not load")
        sleep(4) // let the first refresh land
        shoot(app, "router page")
        let err = any.matching(identifier: "router-error").firstMatch
        XCTAssertFalse(err.exists, "the router page shows an error: \(err.label)")
        link.tap()

        let session = any.matching(identifier: "session-row").firstMatch
        let listed = session.waitForExistence(timeout: 15)
        shoot(app, "sessions page")
        let why = any.matching(identifier: "sessions-error").firstMatch
        XCTAssertTrue(listed, "no session rows listed" + (why.exists ? " — the app says: \(why.label)" : ""))
        session.tap()

        let composer = any.matching(identifier: "composer").firstMatch
        let opened = composer.waitForExistence(timeout: 10)
        shoot(app, "after tapping a session")
        XCTAssertTrue(opened, "the session view did not open — a lone warning triangle here is SwiftUI's unresolved destination")
        sleep(7) // across at least one list refresh
        shoot(app, "seven seconds later")
        XCTAssertTrue(composer.exists, "the session view was popped by a refresh")
    }

    /// The dashboard's screens open from the router page.
    func testDashboardScreensOpen() throws {
        let app = XCUIApplication()
        app.launch()
        let any = app.descendants(matching: .any)
        let row = any.matching(identifier: "router-row").firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 8), "no router row — run the session test first")
        row.tap()
        let usage = any.matching(identifier: "usage").firstMatch
        XCTAssertTrue(usage.waitForExistence(timeout: 15))
        usage.tap()
        XCTAssertTrue(any.matching(identifier: "usage-range").firstMatch.waitForExistence(timeout: 10), "the usage page did not open")
        sleep(4) // charts need the first refresh
        shoot(app, "usage top")
        let details = any.matching(identifier: "usage-details").firstMatch
        scroll(app, to: details)
        shoot(app, "usage bottom")
        let projects = app.staticTexts["Projects"].firstMatch
        XCTAssertTrue(projects.waitForExistence(timeout: 5), "no details links")
        projects.tap()
        XCTAssertTrue(app.navigationBars["Projects"].waitForExistence(timeout: 10), "projects did not open")
        shoot(app, "projects")
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(app.navigationBars["Usage"].waitForExistence(timeout: 5), "did not come back to Usage")
        app.navigationBars.buttons.element(boundBy: 0).tap()

        let accounts = any.matching(identifier: "manage-accounts").firstMatch
        scroll(app, to: accounts)
        XCTAssertTrue(accounts.exists, "no manage-accounts row")
        accounts.tap()
        XCTAssertTrue(app.navigationBars["Accounts"].waitForExistence(timeout: 10), "accounts did not open")
        sleep(3)
        shoot(app, "accounts")
        app.navigationBars.buttons.element(boundBy: 0).tap()

        let settings = any.matching(identifier: "settings").firstMatch
        scroll(app, to: settings)
        XCTAssertTrue(settings.exists, "no settings row")
        settings.tap()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10), "settings did not open")
        sleep(3)
        shoot(app, "settings")
    }

    /// Rows below the fold are not in the accessibility tree until scrolled to.
    private func scroll(_ app: XCUIApplication, to element: XCUIElement) {
        for _ in 0..<8 where !element.exists {
            app.swipeUp()
        }
    }

    private func shoot(_ app: XCUIApplication, _ name: String) {
        let a = XCTAttachment(screenshot: app.screenshot())
        a.name = name
        a.lifetime = .keepAlways
        add(a)
    }
}
