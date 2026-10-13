import XCTest
@testable import AgentRouter

/// Codex sessions on the phone, as the console has them (AGEN-0177): one the machine's managed
/// Codex backend owns is driven; one it owns but is not connected to, and one only watched from
/// its transcript, hold every control in the router's words; none takes a side question; and a
/// Codex session can be started from the phone with any catalogue model.
final class CodexClientTests: XCTestCase {
    private func fixture(_ name: String) throws -> Data {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: name, withExtension: "json"))
        return try Data(contentsOf: url)
    }

    private func sessions() throws -> [LiveSession] { try JSONDecoder().decode([LiveSession].self, from: fixture("codex-live-sessions")) }

    func testAManagedCodexSessionIsDriven() throws {
        let managed = try sessions()[0]
        XCTAssertEqual(managed.clientName, "Codex")
        XCTAssertEqual(managed.shortId, "0199aaaa")
        XCTAssertTrue(managed.canControl, "the managed backend owns it: messages, Stop, model, effort and mode")
        XCTAssertTrue(managed.canSend)
        XCTAssertNil(managed.watchedReason)
        XCTAssertNil(managed.controls.effort)
        XCTAssertNil(managed.controls.question, "its approvals and questions are answered from here")
        XCTAssertNil(managed.controls.modes, "the router's modes")
    }

    func testADisconnectedAndAWatchedCodexSessionHoldEveryControl() throws {
        let s = try sessions()
        let disconnected = s[1], watched = s[2]
        XCTAssertFalse(disconnected.canControl)
        XCTAssertEqual(disconnected.watchedReason, "Managed Codex is disconnected. Reconnect or resume it before sending controls.")
        XCTAssertEqual(disconnected.controls.effort, disconnected.watchedReason)
        XCTAssertFalse(watched.canControl)
        XCTAssertFalse(watched.canSend)
        XCTAssertEqual(watched.watchedReason, "This Codex session is watched from its local transcript. Send messages and answer approvals in the terminal or IDE that owns it.")
        XCTAssertEqual(watched.controls.question, watched.watchedReason)
    }

    func testNoCodexSessionTakesASideQuestion() throws {
        for s in try sessions() {
            XCTAssertFalse(s.canAskAside, s.sessionId)
            XCTAssertNotNil(s.controls.aside)
        }
        XCTAssertEqual(try sessions()[0].controls.aside, "Side questions are not available for Codex yet.")
    }

    func testStartingACodexSessionSendsItsClientAndOffersEveryModel() throws {
        XCTAssertEqual(SessionStart.clients, ["codex", "kimi-code"], "in the console's order")
        XCTAssertEqual(SessionStart.firstClient, "kimi-code")
        let machines = try JSONDecoder().decode([MachineRow].self, from: fixture("kimi-machines"))
        XCTAssertEqual(SessionStart.machines(machines, client: "codex").map(\.id), ["m-mac"], "online, with Codex reported")
        let catalogue = try JSONDecoder().decode([CatalogueModel].self, from: fixture("kimi-models"))
        XCTAssertEqual(SessionStart.models(catalogue, client: "codex").map(\.id), catalogue.map(\.id), "every catalogue model, any provider")
        let body = SessionStart.body(hosted: true, machine: "m-mac", cwd: "/Users/you/code/app", model: "gpt-5.1-codex", client: "codex")
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(body)) as? [String: String])
        XCTAssertEqual(fields, ["machine": "m-mac", "cwd": "/Users/you/code/app", "model": "gpt-5.1-codex", "client": "codex"])
        XCTAssertEqual(SessionStart.noModel("codex"), "No model is available for Codex — configure a provider in Settings.")
        XCTAssertEqual(SessionStart.noMachine("codex"), "No machine online has Codex.")
    }
}
