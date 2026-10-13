import XCTest
@testable import AgentRouter

/// Settings → Clients on the phone, as the console has it (AGEN-0183): the clients and their
/// models, the preview, check, apply and restore requests, apply offered on a local router only,
/// and each shared file named as the console names it. The fixtures are the router's replies with
/// the identities replaced.
final class ClientSetupTests: XCTestCase {
    private func fixture<T: Decodable>(_ name: String, as: T.Type) throws -> T {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: name, withExtension: "json"))
        return try JSONDecoder().decode(T.self, from: Data(contentsOf: url))
    }

    func testTheClientsAndTheirModelsDecode() throws {
        let s = try fixture("client-setup", as: ClientSetupReply.self)
        XCTAssertEqual(s.clients.map(\.id), ["claude-code", "codex", "opencode"])
        XCTAssertEqual(s.clients.map(ClientSetup.status), ["Routed", "Not configured", "Not configured"])
        XCTAssertEqual(ClientSetup.installation(s.clients[0], hosted: false), "2.1.267")
        XCTAssertEqual(ClientSetup.installation(s.clients[2], hosted: false), "Not detected on this machine")
        XCTAssertEqual(ClientSetup.installation(s.clients[2], hosted: true), "Check on your client’s machine")
        XCTAssertEqual(s.models.map(ClientSetup.modelLabel), ["claude-opus-5-5 · Anthropic · claude-opus-5-5", "Kimi K3 · Kimi Code · k3", "GPT-5.1 Codex · OpenAI · gpt-5.1-codex"])
        XCTAssertEqual(s.models[2].available, false)
        XCTAssertEqual(s.models[2].capabilities?.compaction, true)
    }

    func testEachRequestIsBuiltAsTheConsoleSendsIt() throws {
        let input = ClientSetup.input(model: "k3", url: " https://router.example.com ", token: "", project: "  ")
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(input)) as? [String: String])
        XCTAssertEqual(fields, ["model": "k3", "url": "https://router.example.com"], "a blank token and project are left out")
        let applying = ClientSetup.input(model: "k3", url: "http://127.0.0.1:8787", token: "art_x", project: "/Users/you/code/app", revision: "e3b0c442")
        let applied = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(applying)) as? [String: String])
        XCTAssertEqual(applied, ["model": "k3", "url": "http://127.0.0.1:8787", "token": "art_x", "project": "/Users/you/code/app", "revision": "e3b0c442"],
                       "apply sends the preview's revision, which the router refuses when stale")
        XCTAssertEqual(ClientSetup.path("codex", "preview"), "/api/clients/codex/preview")
        XCTAssertEqual(ClientSetup.path("claude-code", "restore"), "/api/clients/claude-code/restore")
        let restore = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(RestoreBody())) as? [String: String])
        XCTAssertEqual(restore, [:], "the global configuration names no project")
    }

    func testApplyIsALocalRoutersAlone() throws {
        let s = try fixture("client-setup", as: ClientSetupReply.self)
        let preview = try fixture("client-preview", as: ClientPreview.self)
        XCTAssertTrue(ClientSetup.offersApply(writes: true, client: s.clients[1], preview: preview))
        XCTAssertFalse(ClientSetup.offersApply(writes: false, client: s.clients[1], preview: preview), "a hosted router offers copy and share only")
        XCTAssertFalse(ClientSetup.offersApply(writes: true, client: s.clients[2], preview: preview), "a client the router cannot write")
        XCTAssertFalse(ClientSetup.offersApply(writes: true, client: s.clients[1], preview: nil), "nothing to apply before a preview")
        XCTAssertTrue(ClientSetup.offersRestore(writes: true, client: s.clients[0], project: ""), "its configuration is in place")
        XCTAssertFalse(ClientSetup.offersRestore(writes: true, client: s.clients[1], project: ""))
        XCTAssertTrue(ClientSetup.offersRestore(writes: true, client: s.clients[1], project: "/Users/you/code/app"))
        XCTAssertFalse(ClientSetup.offersRestore(writes: false, client: s.clients[0], project: ""))
        XCTAssertTrue(ClientSetup.asksProject(local: true, client: "claude-code"))
        XCTAssertFalse(ClientSetup.asksProject(local: true, client: "codex"), "Codex has no project configuration")
        XCTAssertFalse(ClientSetup.asksProject(local: false, client: "claude-code"))
    }

    func testThePreviewAndItsFilesAreNamedAsTheConsoleNamesThem() throws {
        let p = try fixture("client-preview", as: ClientPreview.self)
        XCTAssertEqual(p.checks.map(\.status), ["pass", "warning"])
        XCTAssertNotNil(p.content)
        XCTAssertEqual(p.catalogueContent, #"{"models":[]}"#)
        XCTAssertEqual(ClientSetup.filename(client: "claude-code", format: "json"), "settings.local.json")
        XCTAssertEqual(ClientSetup.filename(client: "codex", format: "toml"), "config.toml")
        XCTAssertEqual(ClientSetup.filename(client: "opencode", format: "jsonc"), "opencode.jsonc")
        XCTAssertEqual(ClientSetup.catalogueFilename, "agent-router-models.json")
        let checked = try JSONDecoder().decode(ClientPreview.self, from: Data(#"{"ok":true,"client":"codex","model":"k3","provider":"kimi","protocol":"responses","translated":true,"can_apply":false,"path":null,"format":"toml","warnings":[],"checks":[],"revision":"r"}"#.utf8))
        XCTAssertNil(checked.content, "a check carries no content")
    }

    func testEachClientsLimitsAndOutcomesAreSaid() throws {
        XCTAssertEqual(ClientSetup.limits(client: "codex", provider: "anthropic", nativeProtocol: "messages", capabilities: nil).count, 3, "compaction, web search, switching providers")
        XCTAssertEqual(ClientSetup.limits(client: "codex", provider: "openai", nativeProtocol: "responses", capabilities: nil).count, 2)
        XCTAssertEqual(ClientSetup.limits(client: "claude-code", provider: "openai", nativeProtocol: "responses", capabilities: nil).count, 1)
        XCTAssertEqual(ClientSetup.limits(client: "claude-code", provider: "anthropic", nativeProtocol: "messages", capabilities: nil), [])
        let applied = try JSONDecoder().decode(ClientApplied.self, from: Data(#"{"ok":true,"client":"codex","path":"/x","changed":true,"warnings":[]}"#.utf8))
        XCTAssertEqual(ClientSetup.appliedWords(applied, name: "Codex"), "Saved Codex configuration. Start a new client session to use it.")
        let kept = try JSONDecoder().decode(ClientRestored.self, from: Data(#"{"ok":true,"client":"codex","path":"/x","changed":true,"preserved_edits":true}"#.utf8))
        XCTAssertTrue(ClientSetup.restoredWords(kept).hasPrefix("Restored unchanged settings."))
    }
}
