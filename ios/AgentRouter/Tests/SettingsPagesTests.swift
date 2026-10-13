import XCTest
@testable import AgentRouter

/// A router's Settings on the phone as the console's pages (AGEN-0182): named and ordered as the
/// console's for a hosted router, a local router and one that has not said which it is; the
/// Mac's pages on a local router alone; the Mac's setup never read from a hosted router; the
/// connect snippet; and the request that switches the network.
final class SettingsPagesTests: XCTestCase {
    func testTheConsolesPagesForEachMode() {
        XCTAssertEqual(SettingsPages.pages(hosted: true).map(\.title),
                       ["Accounts", "Models", "Clients", "Access tokens", "Password", "Notifications", "Router", "Forges", "Library"])
        XCTAssertEqual(SettingsPages.pages(hosted: false).map(\.title),
                       ["Accounts", "Models", "Clients", "Claude Code", "Network", "Access tokens", "Password", "Notifications", "Router"])
        XCTAssertEqual(SettingsPages.pages(hosted: nil).map(\.title),
                       ["Accounts", "Models", "Clients", "Access tokens", "Password", "Notifications", "Router"])
        XCTAssertEqual(SettingsPages.pages(hosted: true).map(\.rawValue),
                       ["accounts", "models", "clients", "access-tokens", "password", "notifications", "router", "forges", "library"], "the console's slugs")
    }

    func testTheMacsPagesAreALocalRoutersAlone() {
        for hosted in [true, nil] as [Bool?] {
            let pages = SettingsPages.pages(hosted: hosted)
            XCTAssertFalse(pages.contains(.claudeCode), "no Claude Code on the Mac for \(String(describing: hosted))")
            XCTAssertFalse(pages.contains(.network))
        }
        XCTAssertTrue(SettingsPages.readsSetup(hosted: false))
        XCTAssertFalse(SettingsPages.readsSetup(hosted: true))
        XCTAssertFalse(SettingsPages.readsSetup(hosted: nil))
    }

    @MainActor
    func testAHostedRoutersSettingsNeverAskForTheMacsSetup() async {
        StandInRouter.reset()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [StandInRouter.self]
        let router = RouterState(router: Router(name: "Stand-in", host: "router.invalid", port: 443, scheme: .https),
                                 password: nil, calls: URLSession(configuration: config))
        StandInRouter.answer("/api/auth", 200, #"{"configured":true,"access_tokens":2,"remote_bind":true,"from_loopback":false,"mode":"server"}"#)
        StandInRouter.answer("/api/push", 200, #"{"configured":false,"devices":[]}"#)
        let settings = SettingsState(router: router)
        await settings.load()
        XCTAssertEqual(settings.auth?.isHosted, true)
        XCTAssertNil(settings.failure, "the setup it would refuse is never asked for, so nothing fails")
        XCTAssertNil(settings.setup)
        XCTAssertNil(settings.shim)
    }

    func testTheSnippetNamesACredentialWhereOneIsAsked() {
        let hosted = SettingsPages.connectSnippet(origin: "https://router.example.com", hosted: true, fromLoopback: false)
        XCTAssertEqual(hosted, "{ \"env\": {\n  \"ANTHROPIC_BASE_URL\": \"https://router.example.com\",\n  \"ANTHROPIC_AUTH_TOKEN\": \"<Client access token or router password>\"\n} }")
        XCTAssertEqual(SettingsPages.connectSnippet(origin: "http://127.0.0.1:8787", hosted: false, fromLoopback: true),
                       "{ \"env\": { \"ANTHROPIC_BASE_URL\": \"http://127.0.0.1:8787\" } }")
        XCTAssertTrue(SettingsPages.connectSnippet(origin: "http://10.0.0.5:8787", hosted: false, fromLoopback: false).contains("ANTHROPIC_AUTH_TOKEN"),
                      "a local router read from another machine asks for a credential too")
        XCTAssertTrue(SettingsPages.storeNote(hosted: true).contains("set by the deployment"))
    }

    func testTheNetworkSwitchSendsItsState() throws {
        let on = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(NetworkBody(enable: true))) as? [String: Bool])
        XCTAssertEqual(on, ["enable": true])
        let auth = try JSONDecoder().decode(AuthStatus.self, from: Data(#"{"configured":true,"remote_bind":true,"from_loopback":true,"lan_url":"http://10.0.0.5:8787","env_override":false,"mode":"local"}"#.utf8))
        XCTAssertEqual(auth.lanURL, "http://10.0.0.5:8787")
        XCTAssertEqual(auth.envOverride, false)
    }
}
