import XCTest
import os
@testable import AgentRouter

/// A refused change reaches the screen in the router's own words.
///
/// The router refuses a change with an error status and the reason under `error` — the
/// wrapper switches under `reason` as well — and a router from before that answered the
/// same refusal as a 200 carrying `error`. Either way the screen shows the sentence the
/// router wrote: never "Done.", and never "Router returned 400".
///
/// Each call goes to a stand-in router through the screens' own state and the app's own
/// client, so the real request, status handling and decoding all run; the stand-in is a
/// URL protocol on the client's session, so nothing leaves the process. The replies are the
/// router's, word for word (src/refusal.rs and the handlers it names).
final class RefusalTests: XCTestCase {
    /// The router's settings and accounts screens, calling the stand-in.
    @MainActor
    private func screens() -> (RouterState, SettingsState, AccountsState) {
        StandInRouter.reset()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [StandInRouter.self]
        let router = RouterState(
            router: Router(name: "Stand-in", host: "router.invalid", port: 443, scheme: .https),
            password: nil,
            calls: URLSession(configuration: config)
        )
        return (router, SettingsState(router: router), AccountsState(router: router))
    }

    private func refusal(_ words: String) -> String { #"{"ok":false,"error":"\#(words)"}"# }

    @MainActor
    func testARefusedPasswordIsShownInTheRoutersWords() async {
        let (_, settings, _) = screens()
        StandInRouter.answer("/api/auth", 400, refusal("use at least 8 characters"))
        await settings.password("short", clear: false)
        XCTAssertEqual(settings.message, "use at least 8 characters")
        XCTAssertEqual(StandInRouter.posts, ["/api/auth"])

        let pinned = "the password is set by the deployment (AGENT_ROUTER_SERVER_PASSWORD); change the Secret and restart the router"
        StandInRouter.answer("/api/auth", 409, refusal(pinned))
        await settings.password(nil, clear: true)
        XCTAssertEqual(settings.message, pinned)
    }

    /// No screen on the phone throws this switch; the client reads its refusal all the same.
    @MainActor
    func testARefusedNetworkSwitchIsShownInTheRoutersWords() async throws {
        let (router, _, _) = screens()
        let words = "set a password first — until one exists, callers from other machines are refused"
        StandInRouter.answer("/api/network", 409, refusal(words))
        let client = try XCTUnwrap(router.liveClient)
        do {
            let _: OkReply = try await client.post("/api/network", ["enable": true])
            XCTFail("a refusal is not a reply")
        } catch {
            XCTAssertEqual(error as? RouterClient.Failure, .refused(409, words))
            XCTAssertEqual(LiveFeed.describe(error), words)
        }
    }

    @MainActor
    func testARefusedLoginItemSwitchIsShownInTheRoutersWords() async {
        let (_, settings, _) = screens()
        let hosted = "this router is hosted: it does not run on a developer's machine, so there is no Claude Code, login item, editor extension or network switch here to manage — set those up on each machine"
        StandInRouter.answer("/api/setup/service", 409, refusal(hosted))
        await settings.service(enable: true)
        XCTAssertEqual(settings.message, hosted)

        // A failure, not a refusal: the router's words all the same.
        let launchd = "launchctl bootstrap gui/501 /Users/u/Library/LaunchAgents/ca.whnet.agent-router.plist: Bootstrap failed: 5: Input/output error"
        StandInRouter.answer("/api/setup/service", 500, refusal(launchd))
        await settings.service(enable: true)
        XCTAssertEqual(settings.message, launchd)
    }

    @MainActor
    func testARefusedRoutingSwitchIsShownInTheRoutersWords() async {
        let (_, settings, _) = screens()
        StandInRouter.answer("/api/setup/config", 400, refusal("/Users/u/code/nowhere is not a directory"))
        await settings.config(enable: true, project: "/Users/u/code/nowhere")
        XCTAssertEqual(settings.message, "/Users/u/code/nowhere is not a directory")

        let broken = "/Users/u/.claude/settings.json is not valid JSON (expected value at line 1 column 1) — fix or move it first"
        StandInRouter.answer("/api/setup/config", 409, refusal(broken))
        await settings.config(enable: true, project: nil)
        XCTAssertEqual(settings.message, broken)
    }

    @MainActor
    func testARefusedWrapperSwitchIsShownInTheRoutersWords() async {
        let (_, settings, _) = screens()
        let stale = "Not a known extension binary."
        StandInRouter.answer("/api/setup/shim", 409, #"{"ok":false,"reason":"\#(stale)","error":"\#(stale)"}"#)
        await settings.shim(path: "/Users/u/.vscode/extensions/anthropic.claude-code-2.1.0/resources/native-binary/claude", action: "install")
        XCTAssertEqual(settings.message, stale)

        StandInRouter.answer("/api/setup/shim", 400, #"{"ok":false,"reason":"unknown action \"replace\"","error":"unknown action \"replace\""}"#)
        await settings.shim(path: "/x/claude", action: "replace")
        XCTAssertEqual(settings.message, #"unknown action "replace""#)

        let hosted = "this router is hosted: it does not run on a developer's machine, so there is no Claude Code, login item, editor extension or network switch here to manage — set those up on each machine"
        StandInRouter.answer("/api/setup/shim/auto", 409, refusal(hosted))
        await settings.shimAuto(true)
        XCTAssertEqual(settings.message, hosted)
    }

    @MainActor
    func testARefusedPushKeyIsShownInTheRoutersWords() async {
        let (_, settings, _) = screens()
        StandInRouter.answer("/api/push/config", 422, refusal("that key does not sign: that is not a PEM private key"))
        await settings.pushConfig(teamId: "ABCDE12345", keyId: "ABC123DEF4", p8: "not a key", bundleId: nil)
        XCTAssertEqual(settings.message, "that key does not sign: that is not a PEM private key")

        StandInRouter.answer("/api/push/config", 400, refusal("team id, key id and the .p8 key are all needed"))
        await settings.pushConfig(teamId: "ABCDE12345", keyId: "", p8: "", bundleId: nil)
        XCTAssertEqual(settings.message, "team id, key id and the .p8 key are all needed")
    }

    @MainActor
    func testARefusedPushTestIsShownInTheRoutersWords() async {
        let (_, settings, _) = screens()
        StandInRouter.answer("/api/push/test", 409, refusal("no push credentials are set"))
        await settings.pushTest()
        XCTAssertEqual(settings.message, "no push credentials are set")
    }

    @MainActor
    func testARefusedUsageKeyIsShownInTheRoutersWords() async {
        let (_, _, accounts) = screens()
        let words = "that does not look like a claude.ai session key (expected sk-ant-sid…)"
        StandInRouter.answer("/api/usage-token", 400, refusal(words))
        let said = await accounts.usageKey("default", token: "sk-ant-api03-abc", clear: false)
        XCTAssertEqual(said, words)
    }

    @MainActor
    func testARefusedSignInIsShownInTheRoutersWords() async {
        let (_, _, accounts) = screens()
        StandInRouter.answer("/api/login/start", 409, refusal("a sign-in is already in progress"))
        let busy = await accounts.loginStart(label: "work")
        XCTAssertEqual(busy?.error, "a sign-in is already in progress")
        XCTAssertNil(busy?.phase, "no sign-in started")

        StandInRouter.answer("/api/login/start", 400, refusal("give the account a name first"))
        let unnamed = await accounts.loginStart(label: " ")
        XCTAssertEqual(unnamed?.error, "give the account a name first")
    }

    /// A router from before refusals had a status: the same words, in a 200.
    @MainActor
    func testAnOlderRoutersRefusalStillReadsInItsWords() async {
        let (_, settings, accounts) = screens()
        StandInRouter.answer("/api/auth", 200, #"{"error":"use at least 8 characters"}"#)
        await settings.password("short", clear: false)
        XCTAssertEqual(settings.message, "use at least 8 characters")

        StandInRouter.answer("/api/setup/shim/auto", 200, #"{"ok":false,"reason":"database is locked"}"#)
        await settings.shimAuto(true)
        XCTAssertEqual(settings.message, "database is locked")

        StandInRouter.answer("/api/usage-token", 200, #"{"error":"no token supplied"}"#)
        let said = await accounts.usageKey("default", token: " ", clear: false)
        XCTAssertEqual(said, "no token supplied")
    }

    /// The live endpoints a hosted router relays refuse in the same shape — 404 for a
    /// session nobody holds, 503 for a machine that is offline, 502 for a relay that failed
    /// — and read the same way, with nothing more to it.
    @MainActor
    func testARelayedLiveRefusalReadsInItsWordsToo() async throws {
        let (router, _, _) = screens()
        let client = try XCTUnwrap(router.liveClient)
        for (status, body, words) in [
            (404, #"{"ok":false,"error":"No such session."}"#, "No such session."),
            (503, #"{"ok":false,"error":"the machine holding that session is offline"}"#, "the machine holding that session is offline"),
            (502, #"{"ok":false,"reason":"the relay failed"}"#, "the relay failed"),
        ] {
            StandInRouter.answer("/api/live/send", status, body)
            do {
                let _: OkReply = try await client.post("/api/live/send", ["session": "s", "text": "hi"])
                XCTFail("\(status) is not a reply")
            } catch {
                XCTAssertEqual(LiveFeed.describe(error), words)
            }
        }
    }

    /// An error status with no words of the router's in it still says what it was.
    @MainActor
    func testAnErrorStatusWithNothingToSayStillNamesItsStatus() async throws {
        let (router, _, _) = screens()
        let client = try XCTUnwrap(router.liveClient)
        StandInRouter.answer("/api/auth", 403, "agent-router: caller is not on this machine")
        do {
            let _: OkReply = try await client.post("/api/auth", ["password": "long enough"])
            XCTFail("a refusal is not a reply")
        } catch {
            XCTAssertEqual(LiveFeed.describe(error), "Router returned 403: agent-router: caller is not on this machine")
        }
        StandInRouter.answer("/api/auth", 401, "agent-router: password required")
        do {
            let _: OkReply = try await client.post("/api/auth", ["password": "long enough"])
            XCTFail("a refusal is not a reply")
        } catch {
            XCTAssertEqual(error as? RouterClient.Failure, .unauthorized)
        }
    }
}

/// A router with no network under it: answers each path from a table, and writes down what
/// was posted. Anything not in the table is a 404 with nothing to say — the reloads that
/// follow a change ask for plenty, and none of it is what these tests read.
final class StandInRouter: URLProtocol {
    struct Reply: Sendable {
        var status: Int
        var body: String
    }

    private static let table = OSAllocatedUnfairLock<[String: Reply]>(initialState: [:])
    private static let asked = OSAllocatedUnfairLock<[String]>(initialState: [])

    static func reset() {
        table.withLock { $0 = [:] }
        asked.withLock { $0 = [] }
    }

    static func answer(_ path: String, _ status: Int, _ body: String) {
        table.withLock { $0[path] = Reply(status: status, body: body) }
    }

    /// The paths posted to, in order.
    static var posts: [String] { asked.withLock { $0 } }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let url = request.url else { return }
        if request.httpMethod == "POST" { Self.asked.withLock { $0.append(url.path) } }
        let reply = Self.table.withLock { $0[url.path] } ?? Reply(status: 404, body: "")
        let type = reply.body.hasPrefix("{") ? "application/json" : "text/plain"
        let response = HTTPURLResponse(url: url, statusCode: reply.status, httpVersion: "HTTP/1.1",
                                       headerFields: ["content-type": type])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(reply.body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}
