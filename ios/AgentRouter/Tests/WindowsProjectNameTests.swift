import XCTest
@testable import AgentRouter

/// A hosted router hears from Windows machines, whose directories are `C:\Users\…`. Every list
/// that names a project from its directory — the dashboard's projects and sessions, a search's
/// sessions and hits, the usage page narrowed to one project — names it by its last segment,
/// split on `/` and `\` alike, as the live list always has and as the web console does
/// (SE-C222). Split on `/` alone, such a project was named by its whole path.
final class WindowsProjectNameTests: XCTestCase {
    private let windows = #"C:\Users\sample\code\agent-router"#

    /// A directory as the router writes it in JSON: each backslash escaped, or null for none.
    private func wire(_ path: String?) -> String {
        guard let path else { return "null" }
        return "\"" + path.replacingOccurrences(of: "\\", with: "\\\\") + "\""
    }

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try JSONDecoder().decode(T.self, from: Data(json.utf8))
    }

    private func projectRow(_ path: String?) throws -> ProjectRow {
        try decode(ProjectRow.self, #"{"project":\#(wire(path)),"sessions":1,"requests":4,"prompt_tokens":1000,"cost":0.1,"output_tokens":10,"cache_read_tokens":900,"subagent_tokens":0,"errors":0,"models":"claude-opus-5","first_seen":1,"last_seen":2}"#)
    }

    private func proxySession(_ path: String?) throws -> ProxySessionRow {
        try decode(ProxySessionRow.self, #"{"id":"2631369d-0980-4930-b3d1-3fd7396c4a20","project":\#(wire(path)),"git_branch":"main","entrypoint":"claude-cli/2.1.269 (external, claude-vscode)","started_at":1,"last_seen_at":2,"requests":4,"prompt_tokens":1000,"cost":0.1,"output_tokens":10,"cache_read_tokens":900,"subagent_tokens":0,"errors":0,"subagents":0,"models":"claude-opus-5"}"#)
    }

    private func search(_ path: String?) throws -> SearchResults {
        try decode(SearchResults.self, #"""
        {"query":"body_cap","expression":"body_cap",
         "sessions":[{"session_id":"9fda36f0-d6e8-4623-93b6-5cae13e706c8","project":\#(wire(path)),
                      "session_label":null,"git_branch":"dev","blocks":3,"hits":4,
                      "first_at":1790000000000,"last_at":1790000009000}],
         "hits":[{"block_id":41,"kind":"tool_use","chars":120,
                  "session_id":"9fda36f0-d6e8-4623-93b6-5cae13e706c8","role":"assistant","ordinal":7,
                  "request_id":48761,"at":1790000009000,"project":\#(wire(path)),
                  "session_label":null,"snippet":"why is \#u{2039}body_cap\#u{203A} 64 KB","rank":-8.04}]}
        """#)
    }

    func testTheProjectListNamesAWindowsProjectByItsLastSegment() throws {
        XCTAssertEqual(try projectRow(windows).name, "agent-router")
        XCTAssertEqual(try proxySession(windows).projectName, "agent-router")
    }

    func testASearchHitNamesAWindowsProjectByItsLastSegment() throws {
        let found = try search(windows)
        XCTAssertEqual(found.sessions.first?.projectName, "agent-router")
        XCTAssertEqual(found.hits.first?.projectName, "agent-router")
    }

    func testTheUsagePageNarrowedToAWindowsProjectIsTitledByItsLastSegment() {
        XCTAssertEqual(UsageScope(project: windows).title, "agent-router")
    }

    /// What each list calls a project, from the directory the router recorded for it.
    private var lists: [(String, (String?) throws -> String)] {
        [
            ("the project list", { try self.projectRow($0).name }),
            ("the dashboard's sessions", { try self.proxySession($0).projectName }),
            ("a search's sessions", { try self.search($0).sessions.first?.projectName ?? "no row" }),
            ("a search hit", { try self.search($0).hits.first?.projectName ?? "no row" }),
            // The usage page is narrowed to a row tapped in the project list, by the row's id.
            ("the usage page", { UsageScope(project: try self.projectRow($0).id).title }),
        ]
    }

    func testEveryListNamesADirectoryAsTheWebConsoleDoesEdgeCasesIncluded() throws {
        let cases: [(String?, String)] = [
            (windows, "agent-router"),
            // A trailing separator ends no segment.
            (windows + "\\", "agent-router"),
            ("/Users/sample/code/agent-router/", "agent-router"),
            // A path that mixes the two splits on both.
            (#"C:/Users/sample\code/agent-router"#, "agent-router"),
            ("agent-router", "agent-router"),
            (#"C:\"#, "C:"),
            // Separators alone name no segment, and are named as written.
            ("/", "/"),
            // No directory at all: the app's dash, where the web says "(unresolved)" — never a
            // blank name.
            ("", "—"),
            (nil, "—"),
        ]
        for (list, name) in lists {
            for (path, expected) in cases {
                XCTAssertEqual(try name(path), expected, "\(list), for \(path ?? "nil")")
            }
        }
    }

    func testTheLiveListAndEveryOtherListAgreeOnAProjectsName() throws {
        for path in [windows, windows + "\\", #"C:/Users/sample\code/agent-router"#, "/Users/sample/code/agent-router"] {
            let live = try decode(LiveSession.self, #"{"session_id":"d6e4d7da","cwd":\#(wire(path)),"project":"C--Users-sample-code-agent-router","last_activity_ms":0,"active":false,"state":"idle","archived":false,"wrapped":true,"bytes":10}"#)
            XCTAssertEqual(live.projectName, "agent-router", path)
            for (list, name) in lists {
                XCTAssertEqual(try name(path), live.projectName, "\(list), for \(path)")
            }
        }
    }
}
