import XCTest
@testable import AgentRouter

/// Settings → Clients on a hosted router with a machine chosen (AGEN-0197), as the console has it
/// (AGEN-0196): the machine-scoped read, preview, apply and restore requests, the connected
/// machines alone to choose from, and each client's state on that machine. The fixtures are the
/// router's replies with made-up identities.
final class RemoteClientSetupTests: XCTestCase {
    /// `GET /api/clients?machine=m-1`: the router's catalogue, and the machine's Claude Code and
    /// Codex as its agent read them.
    private let there = #"""
    {"mode":"server","url":"https://router.example.com","machine":"m-1","models":[{"id":"k3","provider":"kimi","display_name":"Kimi K3","account_id":"kimi","available":true}],
     "clients":[
      {"id":"claude-code","name":"Claude Code","protocol":"messages","installed":true,"version":"2.1.267","configured":true,"managed":true,"config_path":"/Users/you/.claude/settings.json","can_apply":true},
      {"id":"codex","name":"Codex","protocol":"responses","installed":false,"version":null,"configured":false,"managed":false,"config_path":"/Users/you/.codex/config.toml","can_apply":true},
      {"id":"opencode","name":"OpenCode","protocol":"messages","installed":false,"version":null,"configured":false,"managed":false,"config_path":null,"can_apply":false}]}
    """#

    func testEachRequestNamesTheChosenMachine() {
        XCTAssertEqual(ClientSetup.onMachine("/api/clients", "m-1"), "/api/clients?machine=m-1")
        XCTAssertEqual(ClientSetup.onMachine("/api/clients", nil), "/api/clients", "no machine: the router's own, or an export")
        XCTAssertEqual(ClientSetup.path("codex", "preview", machine: "m-1"), "/api/clients/codex/preview?machine=m-1")
        XCTAssertEqual(ClientSetup.path("claude-code", "apply", machine: "a b&c"), "/api/clients/claude-code/apply?machine=a%20b%26c")
        XCTAssertEqual(ClientSetup.path("codex", "restore", machine: "m-1"), "/api/clients/codex/restore?machine=m-1")
        XCTAssertEqual(ClientSetup.path("codex", "check"), "/api/clients/codex/check")
    }

    func testOnlyConnectedMachinesAreOffered() throws {
        let rows = try JSONDecoder().decode([MachineRow].self, from: Data(#"""
        [{"id":"m-1","label":"studio","platform":"macos","version":"0.9.0","build":"tree:a","online":true,"since_ms":1},
         {"id":"m-2","label":"laptop","platform":"linux","version":"0.9.0","build":"tree:b","online":false,"since_ms":1}]
        """#.utf8))
        XCTAssertEqual(ClientSetup.machineChoices(rows).map(\.label), ["studio"])
    }

    func testEachClientIsReadAsTheMachineHasIt() throws {
        let s = try JSONDecoder().decode(ClientSetupReply.self, from: Data(there.utf8))
        XCTAssertEqual(s.machine, "m-1")
        let (claude, codex, opencode) = (s.clients[0], s.clients[1], s.clients[2])
        XCTAssertEqual(ClientSetup.installation(claude, hosted: true, machine: "studio"), "2.1.267")
        XCTAssertEqual(ClientSetup.installation(codex, hosted: true, machine: "studio"), "Not detected on studio")
        XCTAssertEqual(ClientSetup.installation(opencode, hosted: true, machine: "studio"), "Check on your client’s machine", "OpenCode stays an export")
        XCTAssertEqual(ClientSetup.installation(claude, hosted: true), "Check on your client’s machine", "no machine chosen")
        XCTAssertEqual(ClientSetup.status(claude), "Routed")
        XCTAssertEqual(claude.configPath, "/Users/you/.claude/settings.json")
        let router = try JSONDecoder().decode(ClientSetupReply.self, from: Data(#"{"mode":"server","url":"https://router.example.com","clients":[],"models":[]}"#.utf8))
        XCTAssertNil(router.machine, "the router's own reading names no machine")
    }

    func testApplyAndRestoreAreOfferedOnTheMachineAfterAReview() throws {
        let s = try JSONDecoder().decode(ClientSetupReply.self, from: Data(there.utf8))
        let preview = try JSONDecoder().decode(ClientPreview.self, from: Data(#"""
        {"client":"codex","model":"k3","provider":"kimi","protocol":"responses","translated":true,"can_apply":true,"path":"/Users/you/.codex/config.toml","format":"toml","content":"model = \"k3\"","warnings":[],"checks":[],"revision":"e3b0c442"}
        """#.utf8))
        XCTAssertTrue(ClientSetup.offersApply(writes: true, client: s.clients[1], preview: preview), "the chosen machine writes")
        XCTAssertFalse(ClientSetup.offersApply(writes: true, client: s.clients[2], preview: preview), "OpenCode is exported")
        XCTAssertFalse(ClientSetup.offersApply(writes: false, client: s.clients[1], preview: preview), "no machine chosen: an export")
        XCTAssertTrue(ClientSetup.offersRestore(writes: true, client: s.clients[0], project: ""), "the router's configuration is in place there")
        XCTAssertFalse(ClientSetup.offersRestore(writes: true, client: s.clients[1], project: ""))
        let applied = ClientApplied(path: "/Users/you/.codex/config.toml", changed: true, warnings: [])
        XCTAssertEqual(ClientSetup.appliedWords(applied, name: "Codex", machine: "studio"), "Saved Codex configuration on studio. Start a new client session to use it.")
        XCTAssertEqual(ClientSetup.appliedWords(ClientApplied(path: "", changed: false, warnings: []), name: "Codex", machine: "studio"), "Codex on studio already has this configuration.")
    }
}
