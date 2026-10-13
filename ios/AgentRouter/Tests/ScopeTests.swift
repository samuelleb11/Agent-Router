import XCTest
@testable import AgentRouter

/// Search and a project's places on the phone, as the console's Overview has them (AGEN-0186):
/// a search from a scoped Usage page sends that page's provider, account, project, session and
/// the start of its range, never an end; one from the router's page searches everything; and a
/// scoped project says its repository and each directory by its machine.
final class ScopeTests: XCTestCase {
    func testASearchFromAScopedPageSearchesThatScope() {
        let scope = UsageScope(provider: "kimi", account: "kimi-main", project: "gitlab.example.com/you/app", session: "s1")
        let q = ScopedSearch.query("deploy", kind: "text", scope: scope, since: "1791800000000")
        XCTAssertEqual(q["provider"] ?? nil, "kimi")
        XCTAssertEqual(q["account"] ?? nil, "kimi-main")
        XCTAssertEqual(q["project"] ?? nil, "gitlab.example.com/you/app")
        XCTAssertEqual(q["session"] ?? nil, "s1")
        XCTAssertEqual(q["since"] ?? nil, "1791800000000")
        XCTAssertEqual(q["q"] ?? nil, "deploy")
        XCTAssertEqual(q["kind"] ?? nil, "text")
        XCTAssertFalse(q.keys.contains("until"), "never an end, as the console's Search tab never sends one")
        XCTAssertEqual(ScopedSearch.caption(scope: scope), "Searching Session s1 only, over the range the page was on.")
    }

    func testASearchFromTheRoutersPageSearchesEverything() {
        let q = ScopedSearch.query("deploy", kind: nil, scope: nil, since: nil)
        XCTAssertEqual(q.compactMapValues { $0 }, ["q": "deploy", "limit": "25"], "nothing narrows it")
        XCTAssertNil(ScopedSearch.caption(scope: nil))
        XCTAssertNil(ScopedSearch.caption(scope: UsageScope()))
        XCTAssertEqual(SearchRoute(routerId: UUID(uuidString: "00000000-0000-0000-0000-000000000001")!).scope, nil)
    }

    func testAScopedProjectSaysWhereItRan() throws {
        let rows = try JSONDecoder().decode([ProjectRow].self, from: Data(#"""
        [{"project":"gitlab.example.com/you/app","repository":"gitlab.example.com/you/app","sessions":3,"requests":10,"prompt_tokens":1,"cost":1,"output_tokens":1,"cache_read_tokens":1,"subagent_tokens":0,"errors":0,
          "directories":[{"cwd":"/Users/you/code/app","machine":{"id":"m-mac","label":"studio-mac"},"remotes":[]},{"cwd":"C:\\code\\app","machine":{"id":"m-win","label":"  "},"remotes":[]}]},
         {"project":null,"sessions":1,"requests":1,"prompt_tokens":1,"cost":0,"output_tokens":1,"cache_read_tokens":0,"subagent_tokens":0,"errors":0,"directories":[]}]
        """#.utf8))
        let places = try XCTUnwrap(ProjectPlaces.of("gitlab.example.com/you/app", in: rows))
        XCTAssertEqual(places.repository, "gitlab.example.com/you/app")
        XCTAssertEqual(places.directories.map(\.label), ["studio-mac · /Users/you/code/app", #"C:\code\app"#], "a blank machine label names no machine")
        XCTAssertNil(ProjectPlaces.of(ProjectPath.unresolved, in: rows), "the sessions with no directory have nowhere to say")
        XCTAssertNil(ProjectPlaces.of(nil, in: rows), "no project scoped")
        XCTAssertNil(ProjectPlaces.of("elsewhere", in: rows))
    }
}
