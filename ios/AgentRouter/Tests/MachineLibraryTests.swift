import XCTest
@testable import AgentRouter

/// Each machine's Library on the phone, as the console's Machines screen has it (AGEN-0178): its
/// standing in the console's words and each client's own, a sync of one machine's Library from
/// here, and what the machine answered. The fixtures are the router's replies with the
/// identities replaced.
final class MachineLibraryTests: XCTestCase {
    private func items() throws -> [Machines.Item] {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "library-machines", withExtension: "json"))
        let rows = try JSONDecoder().decode([MachineRow].self, from: Data(contentsOf: url))
        return Machines.items(rows, routerBuild: nil, now: Date(timeIntervalSince1970: 1_791_860_000))
    }

    private func item(_ id: String) throws -> Machines.Item { try XCTUnwrap(try items().first { $0.id == id }) }

    func testEachStandingIsSaidInTheConsolesWords() throws {
        XCTAssertEqual(try item("m-win").library, Machines.Standing(text: "Library: 1 item could not be placed", needs: true))
        XCTAssertEqual(try item("m-old").library, Machines.Standing(text: "Its agent predates the Library — run the installer on it again", needs: true))
        XCTAssertEqual(try item("m-off").library?.text, "Takes no part in the Library")
        XCTAssertEqual(try item("m-new").library?.text, "Nothing heard of its Library yet")
        XCTAssertNil(try item("m-none").library, "a router that keeps no Library says nothing of one")
        let replaced = try item("m-lin")
        XCTAssertNil(replaced.library, "Claude Code's line says the machine's own standing, so it is not said twice")
        XCTAssertEqual(replaced.clients.first?.library?.text, "Library in step; it put back agent-router, review over a copy changed there")
        let behind = LibraryStanding(state: "behind", behind: 1, failed: 0, replaced: [], atMs: nil, clients: nil)
        XCTAssertEqual(Machines.standing(behind), Machines.Standing(text: "Library behind on 1 item", needs: true))
    }

    func testEachClientSaysItsOwnStanding() throws {
        let mac = try item("m-mac")
        XCTAssertEqual(mac.clients.map(\.library?.text), ["Library in step", "Library behind on 2 items"])
        XCTAssertEqual(mac.clients[1].line, "Kimi Code: CLI 2.1.1 · no editor extension · home folder, Library behind on 2 items")
        XCTAssertEqual(mac.clients[1].library?.needs, true)
    }

    func testOnlyAnOnlineMachineOfARouterWithALibraryOffersASync() throws {
        XCTAssertTrue(try item("m-mac").syncs)
        XCTAssertFalse(try item("m-old").syncs, "an offline machine syncs when it connects again")
        XCTAssertFalse(try item("m-none").syncs, "a router that keeps no Library")
    }

    func testASyncNamesItsMachineAndSaysWhatItAnswered() throws {
        let body = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(LibrarySyncBody(machine: "m-mac"))) as? [String: String])
        XCTAssertEqual(body, ["machine": "m-mac"])
        let every = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(LibrarySyncBody())) as? [String: String])
        XCTAssertEqual(every, [:], "no machine named tells every one")
        let reply = #"{"machines":[{"machine":"m-mac","label":"studio-mac","outcome":"synced","report":{"items":[{"outcome":"in_step"},{"outcome":"updated"}]}},{"machine":"m-lin","label":"build-box","outcome":"synced","report":{"items":[{"outcome":"in_step"}]}},{"machine":"m-old","label":"old-laptop","outcome":"offline"},{"machine":"m-win","label":"desk-pc","outcome":"failed","why":"the agent did not answer"},{"machine":"m-p","label":"p","outcome":"predates"},{"machine":"m-o","label":"o","outcome":"opted_out"}]}"#
        let told = try JSONDecoder().decode(LibrarySyncReply.self, from: Data(reply.utf8)).machines.map(Machines.told)
        XCTAssertEqual(told, [
            "studio-mac: synced, 1 changed",
            "build-box: synced, already in step",
            "old-laptop: offline — it syncs when it connects again",
            "desk-pc: failed — the agent did not answer",
            "p: its agent predates the Library — run the installer on it again",
            "o: takes no part in the Library",
        ])
        XCTAssertTrue(Machines.connectHow.contains("Machine access token"), "the console's words for connecting a machine")
    }
}
