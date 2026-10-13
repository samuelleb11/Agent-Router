import XCTest
@testable import AgentRouter

/// Kimi Code as a client on the phone, as the console has it (AGEN-0117): each session's client,
/// a watched one's controls held in the router's words and a hosted one driven, each machine's
/// clients, and the request that starts a hosted Kimi Code session. The fixtures are the router's
/// replies with the identities replaced.
final class KimiClientTests: XCTestCase {
    private func fixture(_ name: String) throws -> Data {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: name, withExtension: "json"))
        return try Data(contentsOf: url)
    }

    private func sessions() throws -> [LiveSession] { try JSONDecoder().decode([LiveSession].self, from: fixture("kimi-live-sessions")) }

    func testEachSessionNamesItsClient() throws {
        let s = try sessions()
        XCTAssertEqual(s.map(\.clientName), [nil, "Kimi Code", "Kimi Code web"], "Claude Code's is not named; Kimi Code's hosted one is its web server's")
        XCTAssertEqual(s.map(\.markProvider), [nil, "kimi", "kimi"], "Kimi Code's mark on its rows alone")
        XCTAssertEqual(s[2].hosted, true)
        XCTAssertTrue(SessionClient.isKimi("session_abc", client: nil), "known by its id before the list says")
        XCTAssertFalse(SessionClient.isKimi("11111111-2222-3333-4444-555555555555", client: nil))
    }

    func testAWatchedKimiSessionHoldsEveryControlInTheRoutersWords() throws {
        let watched = try sessions()[1]
        XCTAssertFalse(watched.canControl)
        XCTAssertFalse(watched.canSend)
        XCTAssertEqual(watched.watchedReason, "This Kimi Code session runs in its own terminal or in VS Code, which takes no outside control. Start a hosted session to drive one.")
        XCTAssertEqual(watched.controls.question, watched.controls.held, "its question form is held for the same reason")
    }

    func testAHostedKimiSessionIsDrivenWithoutEffortSideQuestionOrQuestionForm() throws {
        let hosted = try sessions()[2]
        XCTAssertTrue(hosted.canControl)
        XCTAssertTrue(hosted.canSend)
        XCTAssertNil(hosted.watchedReason)
        XCTAssertEqual(hosted.controls.effort, "A Kimi Code session has no effort to set from here.")
        XCTAssertEqual(hosted.controls.aside, "A Kimi Code session has no side question to ask from here.")
        XCTAssertFalse(hosted.canAskAside)
        XCTAssertEqual(hosted.controls.question, "A Kimi Code question is answered in its own window for now.")
        XCTAssertEqual(hosted.controls.modes?.map(\.value), ["manual", "auto", "yolo"], "Kimi Code's own modes")
        XCTAssertEqual(hosted.controls.modes?.map(\.label), ["Manual", "Auto", "YOLO"])
        XCTAssertEqual(hosted.controls.bypass, "yolo", "YOLO is the mode asked about first")
        let claude = try sessions()[0]
        XCTAssertEqual(claude.controls, .every, "a wrapped Claude Code session keeps every control")
        XCTAssertTrue(claude.canControl)
    }

    func testEachMachineSaysItsClients() throws {
        let rows = try JSONDecoder().decode([MachineRow].self, from: fixture("kimi-machines"))
        let items = Machines.items(rows, routerBuild: "tree:abc", now: Date(timeIntervalSince1970: 1_791_860_000))
        let mac = try XCTUnwrap(items.first { $0.id == "m-mac" })
        XCTAssertEqual(mac.clients.map(\.name), ["Claude Code", "Kimi Code", "Codex", "opencode"], "in the order reported; one the console does not name, as sent")
        XCTAssertEqual(mac.clients[0].line, "Claude Code: CLI 2.1.267 · VS Code 2.1.296 · home folder")
        XCTAssertEqual(mac.clients[1].line, "Kimi Code: CLI 2.1.1 · VS Code 0.8.1 · home folder")
        XCTAssertEqual(mac.clients[2].parts, ["no CLI", "Cursor 26.1007.21434", "home folder"])
        XCTAssertEqual(mac.clients[3].parts, ["CLI, version not read", "no editor extension", "no home folder"])
        XCTAssertEqual(try XCTUnwrap(items.first { $0.id == "m-win" }).clients, [], "a machine that reported none")
        XCTAssertEqual(Machines.noClients, "Its agent has reported no Claude Code, Kimi Code or Codex.")
    }

    func testStartingAKimiCodeSessionAsksForAMachineWithItAndAKimiModel() throws {
        let rows = try JSONDecoder().decode([MachineRow].self, from: fixture("kimi-machines"))
        XCTAssertEqual(SessionStart.machines(rows, client: "kimi-code").map(\.label), ["studio-mac"], "online, with Kimi Code reported")
        let catalogue = try JSONDecoder().decode([CatalogueModel].self, from: fixture("kimi-models"))
        XCTAssertEqual(SessionStart.models(catalogue, client: "kimi-code").map(\.name), ["Kimi K3", "Kimi for Coding"], "the catalogue's Kimi Code models, by name")

        let hosted = SessionStart.body(hosted: true, machine: "m-mac", cwd: " /Users/you/code/app ", model: "k3", client: "kimi-code")
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(hosted)) as? [String: String])
        XCTAssertEqual(fields, ["machine": "m-mac", "cwd": "/Users/you/code/app", "model": "k3"], "a hosted router relays it to the machine; Kimi Code is the default client")
        let local = SessionStart.body(hosted: false, machine: "m-mac", cwd: "/Users/you/code/app", model: "k3", client: "kimi-code")
        XCTAssertNil(local.machine, "a local router starts it on itself")
        let reply = try JSONDecoder().decode(StartReply.self, from: Data(#"{"ok":true,"session":"session_new"}"#.utf8))
        XCTAssertEqual(reply.session, "session_new")
    }
}
