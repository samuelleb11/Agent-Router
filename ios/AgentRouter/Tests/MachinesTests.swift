import XCTest
@testable import AgentRouter

/// A hosted router's machines, as the Machines screen lists them, and how one connects. The
/// web console's `ui/src/model/machine.ts` is the reference, and its words are the screen's
/// (docs/web-console-contracts.md, MA-C2 to MA-C6).
final class MachinesTests: XCTestCase {
    private func machines(_ json: String) throws -> [MachineRow] {
        try JSONDecoder().decode([MachineRow].self, from: Data(json.utf8))
    }

    /// A pinned clock, so every "for how long" is the same on every run.
    private let now = Date(timeIntervalSince1970: 1_791_400_000)
    private func ms(hoursAgo h: Double) -> Int { Int((now.timeIntervalSince1970 - h * 3600) * 1000) }

    /// `GET /api/machines` as src/machines.rs writes it, every key present.
    func testAFullReplyDecodes() throws {
        let rows = try machines(#"[{"id":"5f0c2a8e-1b2c-4d5e-8f90-a1b2c3d4e5f6","label":"studio","platform":"macos","version":"0.1.0","build":"tree:6f1d0c2b9a8e7f6d5c4b3a291807f6e5d4c3b2a1","online":true,"since_ms":1791389200000},{"id":"9b3e1d47-0a1b-4c2d-9e3f-405162738495","label":"build-box","platform":"linux","version":"0.1.0","build":"tree:0000000000000000000000000000000000000000","online":false,"since_ms":1791227200000}]"#)
        XCTAssertEqual(rows.count, 2)
        XCTAssertEqual(rows[0].id, "5f0c2a8e-1b2c-4d5e-8f90-a1b2c3d4e5f6")
        XCTAssertEqual(rows[0].label, "studio")
        XCTAssertEqual(rows[0].platform, "macos")
        XCTAssertEqual(rows[0].version, "0.1.0")
        XCTAssertEqual(rows[0].build, "tree:6f1d0c2b9a8e7f6d5c4b3a291807f6e5d4c3b2a1")
        XCTAssertTrue(rows[0].online)
        XCTAssertEqual(rows[0].sinceMs, 1_791_389_200_000)
        XCTAssertFalse(rows[1].online)
        XCTAssertEqual(rows[1].platform, "linux")
    }

    /// An agent that says less than this build expects — no label, platform, build or time,
    /// or a key of another shape — is listed all the same, and the rest of the list with it.
    func testASparseReplyDecodes() throws {
        let rows = try machines(#"[{"id":"77aa00bb11cc"},{"id":"88dd","label":"Laptop","online":true,"since_ms":"soon","build":null}]"#)
        XCTAssertEqual(rows.count, 2)
        XCTAssertEqual(rows[0].label, "")
        XCTAssertEqual(rows[0].platform, "")
        XCTAssertNil(rows[0].version)
        XCTAssertNil(rows[0].build)
        XCTAssertFalse(rows[0].online, "a machine that does not say it is online is not")
        XCTAssertNil(rows[0].sinceMs)
        XCTAssertTrue(rows[1].online)
        XCTAssertNil(rows[1].sinceMs, "a time of the wrong shape is no time")
        XCTAssertEqual(try machines("[]").count, 0, "an empty list is a list")
    }

    /// A reply that is not a list is a failed read, never an empty one (MA-C6).
    func testAReplyThatIsNotAListDoesNotDecode() {
        XCTAssertThrowsError(try machines(#"{"error":"not found"}"#))
        XCTAssertThrowsError(try machines(#"{"machines":[]}"#))
        XCTAssertThrowsError(try machines("null"))
    }

    /// Through the screen's own state and the app's own client: a reply that is not a list is
    /// a failure the screen says, with no list to show; and once a list was read, a failed
    /// read keeps it on screen and says why (MA-C6).
    @MainActor
    func testAFailedReadKeepsWhatWasShownAndSaysWhy() async {
        StandInRouter.reset()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [StandInRouter.self]
        let router = RouterState(router: Router(name: "Hosted", host: "router.invalid", port: 443, scheme: .https),
                                 password: nil, calls: URLSession(configuration: config))
        let model = MachinesState(router: router)

        StandInRouter.answer("/api/machines", 200, #"{"error":"not a list"}"#)
        StandInRouter.answer("/_router/health", 200, #"{"ok":true,"pid":1,"version":"0.1.0","build":"tree:aaa"}"#)
        await model.load()
        XCTAssertNil(model.rows, "nothing read is not an empty list")
        XCTAssertNotNil(model.failure)
        XCTAssertEqual(model.routerBuild, "tree:aaa")

        StandInRouter.answer("/api/machines", 200, #"[{"id":"m1","label":"studio","platform":"macos","version":"0.1.0","build":"tree:aaa","online":true,"since_ms":1}]"#)
        await model.load()
        XCTAssertEqual(model.rows?.map(\.label), ["studio"])
        XCTAssertNil(model.failure)
        XCTAssertEqual(model.items(now: now).first?.build?.same, true)

        StandInRouter.answer("/api/machines", 503, #"{"error":"the router is restarting"}"#)
        await model.load()
        XCTAssertEqual(model.rows?.map(\.label), ["studio"], "a failed read blanks nothing")
        XCTAssertEqual(model.failure, "the router is restarting", "in the router's words")
    }

    /// `/_router/health` names the source tree the router was built from; a router from
    /// before that does not, and still decodes, as does one that sends something else there.
    func testHealthNamesItsBuildWhenTheRouterSendsOne() throws {
        func health(_ json: String) throws -> Health { try JSONDecoder().decode(Health.self, from: Data(json.utf8)) }
        let current = try health(#"{"ok":true,"pid":48291,"version":"0.1.0","build":"tree:6f1d0c2b9a8e7f6d5c4b3a291807f6e5d4c3b2a1"}"#)
        XCTAssertEqual(current.build, "tree:6f1d0c2b9a8e7f6d5c4b3a291807f6e5d4c3b2a1")
        XCTAssertEqual(current.pid, 48291)
        let older = try health(#"{"ok":true,"pid":48291,"version":"0.1.0"}"#)
        XCTAssertNil(older.build)
        XCTAssertTrue(older.ok)
        XCTAssertNil(try health(#"{"ok":true,"pid":1,"version":"0.1.0","build":7}"#).build)
    }

    /// Online first, then by label as the session list heads its machines: case aside, and
    /// two labels that differ only in case in one fixed order, the capitalised one first. A
    /// blank label is named by the start of its id (MA-C3).
    func testOnlineMachinesComeFirstThenByLabel() throws {
        let rows = try machines(#"""
        [{"id":"1","label":"zed","online":false},
         {"id":"2","label":"laptop","online":true},
         {"id":"3","label":"Beta","online":true},
         {"id":"4","label":"alpha","online":false},
         {"id":"5","label":"Laptop","online":true},
         {"id":"c0ffee00deadbeef","label":"  ","online":true},
         {"id":"6","label":"  studio ","online":false}]
        """#)
        let items = Machines.items(rows, routerBuild: nil, now: now)
        XCTAssertEqual(items.map(\.label), ["Beta", "c0ffee00", "Laptop", "laptop", "alpha", "studio", "zed"])
        XCTAssertEqual(items.map(\.online), [true, true, true, true, false, false, false])
        XCTAssertEqual(Machines.tally(items), "4 online · 3 offline")
        XCTAssertEqual(Machines.tally(Array(items.prefix(2))), "2 online")
        XCTAssertEqual(Machines.tally(Array(items.suffix(1))), "1 offline")
        XCTAssertEqual(Machines.tally([]), "")
    }

    /// How long a machine has been online or offline, from `since_ms`, in the app's own
    /// durations; with no time, only which (MA-C3). Online is drawn as work is, offline as an
    /// ended session's ring.
    func testPresenceSaysForHowLong() throws {
        XCTAssertEqual(Machines.presence(online: true, sinceMs: ms(hoursAgo: 3), now: now), "online for 3h 00m")
        XCTAssertEqual(Machines.presence(online: false, sinceMs: ms(hoursAgo: 48), now: now), "offline for 2d 0h")
        XCTAssertEqual(Machines.presence(online: true, sinceMs: ms(hoursAgo: 45.0 / 3600), now: now), "online for 45s")
        XCTAssertEqual(Machines.presence(online: true, sinceMs: nil, now: now), "online")
        XCTAssertEqual(Machines.presence(online: false, sinceMs: 0, now: now), "offline")
        XCTAssertEqual(Machines.presence(online: true, sinceMs: ms(hoursAgo: -1), now: now), "online for 0s",
                       "a router's clock ahead of the phone's never says a negative time")

        let rows = try machines(#"[{"id":"a","label":"studio","online":true,"since_ms":\#(ms(hoursAgo: 3))},{"id":"b","label":"box","online":false,"since_ms":\#(ms(hoursAgo: 48))}]"#)
        let items = Machines.items(rows, routerBuild: nil, now: now)
        XCTAssertEqual(items.map(\.presence), ["online for 3h 00m", "offline for 2d 0h"])
        XCTAssertEqual(items[0].look, StateLook(color: .green, hollow: false, halo: true, label: "online"))
        XCTAssertEqual(items[1].look, StateLook(color: .gray, hollow: true, halo: false, label: "offline"))
    }

    /// macOS, Linux and Windows in the reader's words, whatever the case they arrive in; a
    /// platform this build does not know is shown as the agent sent it (MA-C3).
    func testPlatformsAreNamedInWords() {
        XCTAssertEqual(Machines.platform("macos"), "macOS")
        XCTAssertEqual(Machines.platform("linux"), "Linux")
        XCTAssertEqual(Machines.platform("windows"), "Windows")
        XCTAssertEqual(Machines.platform("Windows"), "Windows")
        XCTAssertEqual(Machines.platform(" MACOS "), "macOS")
        XCTAssertEqual(Machines.platform("freebsd"), "freebsd")
        XCTAssertEqual(Machines.platform(" Haiku "), "Haiku")
        XCTAssertEqual(Machines.platform(""), "")
    }

    /// A machine's build against the router's: the same build quietly, another with what
    /// updates it, and nothing at all until the router's health has named its own — nor for a
    /// machine that named none (MA-C4).
    func testABuildIsComparedOnlyOnceTheRoutersIsKnown() throws {
        let rows = try machines(#"[{"id":"a","label":"a","online":true,"build":"tree:aaa"},{"id":"b","label":"b","online":true,"build":"tree:bbb"},{"id":"c","label":"c","online":true},{"id":"d","label":"d","online":true,"build":""}]"#)
        XCTAssertEqual(Machines.items(rows, routerBuild: nil, now: now).map(\.build), [nil, nil, nil, nil],
                       "no build line before health answers")
        XCTAssertEqual(Machines.items(rows, routerBuild: "", now: now).map(\.build), [nil, nil, nil, nil],
                       "nor from a router that names no build")
        let items = Machines.items(rows, routerBuild: "tree:aaa", now: now)
        XCTAssertEqual(items.map(\.build), [
            Machines.Build(same: true, text: "Same build as this router"),
            Machines.Build(same: false, text: "Runs a different build from this router — run the installer on it again to update it."),
            nil, nil,
        ])
    }

    /// The installer's command for each platform, at the address the router gives for itself
    /// when it gives one, else at the one this phone reaches it by (MA-C2).
    func testInstallCommandsUseTheRoutersPublicAddress() {
        let hosted = Router(name: "Hosted", host: "agent-router.example.net", port: 443, scheme: .https)
        let given = Machines.origin(publicURL: "https://router.example.com", router: hosted)
        XCTAssertEqual(given, "https://router.example.com")
        XCTAssertEqual(Machines.installCommands(origin: given), [
            .init(label: "macOS and Linux", command: "curl -fsSL https://router.example.com/agent/install.sh | sh"),
            .init(label: "Windows (PowerShell)", command: "irm https://router.example.com/agent/install.ps1 | iex"),
        ])
        XCTAssertEqual(Machines.origin(publicURL: " https://router.example.com/ ", router: hosted), "https://router.example.com",
                       "a trailing slash would double the path's")

        for none in [nil, "", "  "] {
            XCTAssertEqual(Machines.origin(publicURL: none, router: hosted), "https://agent-router.example.net")
        }
        let lan = Router(name: "Mac", host: "10.0.0.5", port: 8787)
        let own = Machines.origin(publicURL: nil, router: lan)
        XCTAssertEqual(own, "http://10.0.0.5:8787")
        XCTAssertEqual(Machines.installCommands(origin: own).map(\.command), [
            "curl -fsSL http://10.0.0.5:8787/agent/install.sh | sh",
            "irm http://10.0.0.5:8787/agent/install.ps1 | iex",
        ])
    }
}
