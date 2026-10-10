import XCTest
@testable import AgentRouter

/// `GET /api/projects` groups requests by their session's working directory, and a session that
/// never recorded one has none: the reply then holds one row whose `project` is null, for all such
/// sessions. Declared a `String`, that one row failed the whole array, and the phone showed the
/// decode error and no projects for any range that held those sessions (AGEN-0027).
final class ProjectRowDecodingTests: XCTestCase {
    /// A reply as the router writes it (src/api.rs, `projects_inner`): two projects with a
    /// directory, a Windows machine's among them, and between them the sessions without one.
    private let reply = #"""
    [{"project":"/Users/sample/code/project-a","sessions":4,"requests":120,"prompt_tokens":9000000,"cost":30.5,"output_tokens":90000,"cache_read_tokens":8500000,"subagent_tokens":400000,"errors":1,"models":"claude-opus-5","first_seen":1789200000000,"last_seen":1789300000000},
     {"project":null,"sessions":3,"requests":45,"prompt_tokens":2000000,"cost":6.25,"output_tokens":20000,"cache_read_tokens":1900000,"subagent_tokens":0,"errors":0,"models":"claude-opus-5,claude-haiku-4-5","first_seen":1789100000000,"last_seen":1789290000000},
     {"project":"C:\\Users\\sample\\code\\project-b","sessions":1,"requests":8,"prompt_tokens":300000,"cost":0.9,"output_tokens":3000,"cache_read_tokens":280000,"subagent_tokens":0,"errors":0,"models":"claude-sonnet-4-5","first_seen":1789250000000,"last_seen":1789260000000}]
    """#

    /// One row with `project` as given, null for none, and the same figures whatever the path, so
    /// that the path alone tells two such rows apart.
    private static func row(_ project: String?) -> String {
        let wire = project.map { "\"" + $0.replacingOccurrences(of: "\\", with: "\\\\") + "\"" } ?? "null"
        return #"{"project":\#(wire),"sessions":3,"requests":45,"prompt_tokens":2000000,"cost":6.25,"output_tokens":20000,"cache_read_tokens":1900000,"subagent_tokens":0,"errors":0,"models":"claude-opus-5","first_seen":1789100000000,"last_seen":1789290000000}"#
    }

    /// The reply read as the app reads it: `RouterClient` decodes with a plain `JSONDecoder`.
    private func projects(_ json: String) throws -> [ProjectRow] {
        try JSONDecoder().decode([ProjectRow].self, from: Data(json.utf8))
    }

    func testAReplyHoldingARowWithoutADirectoryDecodesWhole() throws {
        let rows = try projects(reply)
        XCTAssertEqual(rows.count, 3)
        XCTAssertEqual(rows[0].project, "/Users/sample/code/project-a")
        XCTAssertNil(rows[1].project)
        XCTAssertEqual(rows[2].project, #"C:\Users\sample\code\project-b"#)
        // A row like any other: its figures arrive with it.
        XCTAssertEqual(rows[1].sessions, 3)
        XCTAssertEqual(rows[1].requests, 45)
        XCTAssertEqual(rows[1].cost, 6.25)
        XCTAssertEqual(rows[1].models, "claude-opus-5,claude-haiku-4-5")
        XCTAssertEqual(rows[1].lastSeen, 1789290000000)
    }

    func testThatRowIsNamedWithTheDash() throws {
        let rows = try projects(reply)
        XCTAssertEqual(rows.map(\.name), ["project-a", "—", "project-b"])
        // The phones' name for a missing path, which every other list gives a session without one.
        XCTAssertEqual(rows[1].name, ProjectPath.name(nil))
    }

    func testThatRowHasAStableIdentityNoRealPathShares() throws {
        let unresolved = try projects("[\(Self.row(nil))]")[0]
        // The router's own key for those sessions, and the same on every refresh.
        XCTAssertEqual(unresolved.id, "(unresolved)")
        XCTAssertEqual(try projects("[\(Self.row(nil))]")[0].id, unresolved.id)
        XCTAssertEqual(Set(try projects(reply).map(\.id)).count, 3)
        // No path's: a real directory is absolute (POSIX, a Windows drive, a share), and a path
        // that is merely empty, or the dash itself, is a row of its own. With the figures alike,
        // the path alone separates them, for `Identifiable` and `Hashable` both.
        for path in ["/", "/Users/sample/code/project-a", #"C:\"#, #"C:\Users\sample\code\project-b"#,
                     #"\\server\share\project-c"#, "", "—"] {
            let other = try projects("[\(Self.row(path))]")[0]
            XCTAssertNotEqual(other.id, unresolved.id, path)
            XCTAssertNotEqual(other, unresolved, path)
            XCTAssertEqual(Set([other, unresolved]).count, 2, path)
        }
    }

    func testTappingThatRowNarrowsThePageToTheSessionsWithoutADirectory() throws {
        let rows = try projects(reply)
        // What the list builds when a row is tapped (`ProjectsView`): the router's key for those
        // sessions, which its scope selects them by, as the web console's row does (OV-C136).
        let scope = UsageScope(project: rows[1].id)
        XCTAssertEqual(scope.project, "(unresolved)")
        // The page is titled as the row is named.
        XCTAssertEqual(scope.title, "—")
        // A row with a path narrows to that path, as before.
        XCTAssertEqual(UsageScope(project: rows[0].id).project, "/Users/sample/code/project-a")
        XCTAssertEqual(UsageScope(project: rows[0].id).title, "project-a")
        XCTAssertEqual(UsageScope(project: rows[2].id).title, "project-b")
    }
}
