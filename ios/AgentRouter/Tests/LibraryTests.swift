import XCTest
@testable import AgentRouter

/// A hosted router's Library on the phone, as the console's Library screen has it (AGEN-0181):
/// its items, their use and versions, each provenance in the console's words, and the
/// requests that make a version live, reject one, archive an item and sync every machine.
/// The fixtures are the router's replies with made-up items.
final class LibraryTests: XCTestCase {
    private func fixture<T: Decodable>(_ name: String, as: T.Type) throws -> T {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: name, withExtension: "json"))
        return try JSONDecoder().decode(T.self, from: Data(contentsOf: url))
    }

    func testTheItemsDecodeInOrderWithTheirStandingInWords() throws {
        let items = Library.ordered(try fixture("library-items", as: LibraryList.self).items)
        XCTAssertEqual(items.map(\.id), ["skill/agent-router", "skill/review", "command/deploy", "subagent/tester"], "by kind, then name")
        XCTAssertEqual(items.map(Library.liveWords), ["v1 live", "v3 live", "none live", "v1 live"])
        XCTAssertEqual(items.map { Library.sourceWords($0.source) },
                       ["comes with the router’s build", "imported by the source team-skills", "uploaded and published here", "uploaded and published here"])
        XCTAssertEqual(items[3].pending, 1)
        XCTAssertNotNil(items[2].archivedAt)
        XCTAssertEqual(items.map { Library.usedWords($0.uses30d ?? 0) }, ["not used in 30 days", "used 12 times in 30 days", "used once in 30 days", "not used in 30 days"])
        XCTAssertEqual(Library.kindWord("subagent"), "Subagent")
        XCTAssertEqual(Library.stateWord("pending"), "waits for approval")
    }

    func testEachProvenanceIsSaidInTheConsolesWords() throws {
        let d = try fixture("library-item", as: LibraryItemDetail.self)
        XCTAssertEqual(d.versions.map { Library.provenanceWords($0.provenance) },
                       ["published from studio-mac by a Claude Code session", "uploaded from the console", "from somewhere the console does not know"])
        XCTAssertEqual(Library.sessionOf(d.versions[0].provenance), "11111111-2222-3333-4444-555555555555", "opens the session that published it")
        XCTAssertNil(Library.sessionOf(d.versions[1].provenance))
        let review = try fixture("library-items", as: LibraryList.self).items.first { $0.name == "review" }
        XCTAssertEqual(review.map { Library.provenanceWords($0.live!.provenance) }, "imported from gitlab.example.com/team/skills at 01234567, skills/review")
        XCTAssertEqual(Library.provenanceWords(.token("ci-publisher")), "published with the token ci-publisher")
        XCTAssertEqual(Library.provenanceWords(.router(build: "tree:abc")), "built into the router")
        XCTAssertEqual(Library.provenanceWords(.machine(machine: "abcdef0123456789", label: nil, session: nil)), "published from abcdef01")
        XCTAssertEqual(d.files.map(\.path), ["tester.md", "scripts/run.sh"])
    }

    func testTheUseIsReadWithWhatTheSkillsAddToEveryTurn() throws {
        let u = try fixture("library-usage", as: LibraryUsage.self)
        XCTAssertEqual(u.days.count, 30)
        XCTAssertEqual(Library.listingWords(u.listing), "The Library’s skills add about 1,180 tokens to every turn.")
        XCTAssertTrue(Library.listingWords(nil).hasPrefix("What the Library’s skills add to every turn is not measured yet"))
        let review = try XCTUnwrap(u.items.first)
        XCTAssertEqual(review.machines.map(Library.machineWords), ["studio-mac", "a machine not known"])
        XCTAssertEqual(review.projects.first?.uses, 12)
    }

    func testEachDecisionPostsToItsVersionsRoute() throws {
        XCTAssertEqual(Library.acts("pending"), [.approve, .reject])
        XCTAssertEqual(Library.acts("superseded"), [.restore])
        XCTAssertEqual(Library.acts("live"), [])
        XCTAssertEqual(Library.acts("rejected"), [])
        XCTAssertEqual(Library.acts("superseded").map(Library.actWord), ["Make live again"])
        XCTAssertEqual(Library.actPath(kind: "subagent", name: "tester", version: 3, .approve), "/api/library/items/subagent/tester/versions/3/live")
        XCTAssertEqual(Library.actPath(kind: "subagent", name: "tester", version: 1, .restore), "/api/library/items/subagent/tester/versions/1/live")
        XCTAssertEqual(Library.actPath(kind: "subagent", name: "tester", version: 3, .reject), "/api/library/items/subagent/tester/versions/3/reject")
        XCTAssertEqual(Library.itemPath(kind: "skill", name: "review") + "/archive", "/api/library/items/skill/review/archive")
        XCTAssertEqual(Library.filePath(kind: "subagent", name: "tester", version: 3, path: "scripts/run me.sh"),
                       "/api/library/items/subagent/tester/versions/3/files/scripts/run%20me.sh", "each segment encoded, the slashes kept")
        let every = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(LibrarySyncBody())) as? [String: Any])
        XCTAssertTrue(every.isEmpty, "Sync every machine now names none")
        XCTAssertEqual(Library.mainFile(kind: "skill", name: "review"), "SKILL.md")
        XCTAssertEqual(Library.mainFile(kind: "command", name: "deploy"), "deploy.md")
    }

    func testAMarkdownFileIsShownWithoutItsFrontmatter() {
        XCTAssertEqual(Library.body("---\nname: review\ndescription: x\n---\n\n# Review\nBody"), "# Review\nBody")
        XCTAssertEqual(Library.body("# No frontmatter"), "# No frontmatter")
        XCTAssertEqual(Library.body("---\nunterminated"), "---\nunterminated")
    }
}
