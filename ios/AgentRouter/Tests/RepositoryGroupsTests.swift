import XCTest
@testable import AgentRouter

/// A project is a git repository, not a directory. The router names each live session's
/// repository (`repository`: the canonical spelling of its directory's `origin` remote, else of
/// its first), and the list names the project by the repository's last segment — so a
/// repository's clones and lane worktrees on one machine are one project, as on the web console
/// and the Android app. A row without one is named by its directory, as it always was. The
/// project's totals are asked for by its key, `project=`: the repository, else the directory.
final class RepositoryGroupsTests: XCTestCase {
    private let repository = "gitlab.example.com/acme/ui-kit-web"

    /// A live row as the router writes it, with `repository` only when one is given.
    private func row(_ id: String, cwd: String?, repository: String? = nil, agoMs: Int = 60_000,
                     machine: (String, String)? = nil) throws -> LiveSession {
        let now = 1_790_000_000_000
        let wireCwd = cwd.map { "\"\($0)\"" } ?? "null"
        var fields = [
            #""session_id":"\#(id)""#,
            #""cwd":\#(wireCwd)"#,
            #""project":"-Users-sample-code-\#(id)""#,
            #""last_activity_ms":\#(now - agoMs)"#,
            #""as_of_ms":\#(now)"#,
            #""active":false,"state":"idle","archived":false,"wrapped":true,"bytes":10"#,
            #""activity":"idle","tone":"idle""#,
        ]
        if let repository { fields.append(#""repository":"\#(repository)""#) }
        if let machine { fields.append(#""machine":{"id":"\#(machine.0)","label":"\#(machine.1)"}"#) }
        return try JSONDecoder().decode(LiveSession.self, from: Data("{\(fields.joined(separator: ","))}".utf8))
    }

    private var now: Date { Date(timeIntervalSince1970: 1_790_000_000) }

    // ---------- (1) grouping ----------

    func testAClonesWorktreesAreOneProjectNamedByTheRepository() throws {
        let main = try row("aaaa1111", cwd: "/Users/sample/code/ui-kit-web", repository: repository)
        let lane = try row("bbbb2222", cwd: "/Users/sample/code/ui-kit-web-lane-2", repository: repository, agoMs: 30_000)
        let other = try row("cccc3333", cwd: "/Users/sample/code/notes")

        XCTAssertEqual(main.projectName, "ui-kit-web")
        XCTAssertEqual(lane.projectName, "ui-kit-web")
        // No repository: the directory's last segment, as before.
        XCTAssertEqual(other.projectName, "notes")

        let groups = SessionGroups.build([main, lane, other], now: now)
        XCTAssertEqual(groups.recent.map(\.name), ["notes", "ui-kit-web"])
        let project = try XCTUnwrap(groups.recent.first { $0.name == "ui-kit-web" })
        // Newest first within the project, whichever directory each runs in.
        XCTAssertEqual(project.sessions.map(\.sessionId), ["bbbb2222", "aaaa1111"])
        XCTAssertEqual(groups.recent.first { $0.name == "notes" }?.sessions.map(\.sessionId), ["cccc3333"])
    }

    func testWithoutARepositoryTwoDirectoriesStayTwoProjects() throws {
        let main = try row("aaaa1111", cwd: "/Users/sample/code/ui-kit-web")
        let lane = try row("bbbb2222", cwd: "/Users/sample/code/ui-kit-web-lane-2")
        XCTAssertEqual(SessionGroups.build([main, lane], now: now).recent.map(\.name), ["ui-kit-web", "ui-kit-web-lane-2"])
    }

    func testAnEmptyRepositoryNamesTheProjectByItsDirectory() throws {
        XCTAssertEqual(try row("aaaa1111", cwd: "/Users/sample/code/notes", repository: "").projectName, "notes")
        // Neither: the router's own name, as before.
        XCTAssertEqual(try row("aaaa1111", cwd: nil).projectName, "-Users-sample-code-aaaa1111")
        // A repository and no directory still names the project.
        XCTAssertEqual(try row("aaaa1111", cwd: nil, repository: repository).projectName, "ui-kit-web")
    }

    func testOneRepositoryOnTwoMachinesIsAProjectOnEach() throws {
        let mac = try row("aaaa1111", cwd: "/Users/sample/code/ui-kit-web", repository: repository, machine: ("m-1", "mac"))
        let linux = try row("bbbb2222", cwd: "/home/sample/code/ui-kit-web", repository: repository, machine: ("m-2", "linux"))
        let lane = try row("cccc3333", cwd: "/Users/sample/code/ui-kit-web-lane-3", repository: repository, machine: ("m-1", "mac"))
        let parts = SessionGroups.byMachine([mac, linux, lane], now: now)
        XCTAssertEqual(parts.count, 2)
        for part in parts {
            XCTAssertEqual(part.groups.recent.map(\.name), ["ui-kit-web"], part.name ?? "unnamed")
        }
        let onMac = try XCTUnwrap(parts.first { $0.name == "mac" })
        XCTAssertEqual(Set(onMac.groups.recent[0].sessions.map(\.sessionId)), ["aaaa1111", "cccc3333"])
    }

    // ---------- (2) decoding ----------

    func testARowDecodesWithAndWithoutARepository() throws {
        let with = try row("aaaa1111", cwd: "/Users/sample/code/ui-kit-web-lane-2", repository: repository)
        XCTAssertEqual(with.repository, repository)
        XCTAssertEqual(with.cwd, "/Users/sample/code/ui-kit-web-lane-2")

        let without = try row("aaaa1111", cwd: "/Users/sample/code/ui-kit-web-lane-2")
        XCTAssertNil(without.repository)
        XCTAssertEqual(without.projectName, "ui-kit-web-lane-2")

        // Null, or a shape this build does not expect, reads as none rather than failing the list.
        for value in ["null", "42", #"{"url":"x"}"#] {
            let json = #"{"session_id":"s","cwd":"/code/p","project":"-code-p","last_activity_ms":0,"active":false,"state":"idle","archived":false,"wrapped":true,"bytes":1,"repository":\#(value)}"#
            let decoded = try JSONDecoder().decode(LiveSession.self, from: Data(json.utf8))
            XCTAssertNil(decoded.repository, value)
            XCTAssertEqual(decoded.projectName, "p", value)
        }
    }

    func testTheUsageRowsDecodeTheRepositoryKeysAndDoWithoutThem() throws {
        let projects = #"""
        [{"project":"gitlab.example.com/acme/ui-kit-web","repository":"gitlab.example.com/acme/ui-kit-web",
          "directories":[{"cwd":"/Users/sample/code/ui-kit-web","machine":{"id":"m-1","label":"mac"},
                          "remotes":[{"name":"origin","url":"git@gitlab.example.com:acme/ui-kit-web.git","repository":"gitlab.example.com/acme/ui-kit-web"}]},
                         {"cwd":"/Users/sample/code/ui-kit-web-lane-2","machine":{"id":"m-1"},"remotes":[]}],
          "sessions":2,"requests":4,"prompt_tokens":1000,"cost":0.1,"output_tokens":10,"cache_read_tokens":900,"subagent_tokens":0,"errors":0,"models":"claude-opus-5","first_seen":1,"last_seen":2},
         {"project":"/Users/sample/code/notes","sessions":1,"requests":1,"prompt_tokens":10,"cost":0.01,"output_tokens":1,"cache_read_tokens":0,"subagent_tokens":0,"errors":0,"models":null,"first_seen":1,"last_seen":2},
         {"project":null,"repository":null,"directories":[],"sessions":1,"requests":1,"prompt_tokens":10,"cost":0.01,"output_tokens":1,"cache_read_tokens":0,"subagent_tokens":0,"errors":0,"models":null,"first_seen":1,"last_seen":2}]
        """#
        let rows = try JSONDecoder().decode([ProjectRow].self, from: Data(projects.utf8))
        XCTAssertEqual(rows.map(\.name), ["ui-kit-web", "notes", "—"])
        XCTAssertEqual(rows.map(\.id), [repository, "/Users/sample/code/notes", ProjectPath.unresolved])
        XCTAssertEqual(rows[0].repository, repository)
        XCTAssertEqual(rows[0].directories?.map(\.cwd), ["/Users/sample/code/ui-kit-web", "/Users/sample/code/ui-kit-web-lane-2"])
        XCTAssertEqual(rows[0].directories?.first?.machine, MachineRef(id: "m-1", label: "mac"))
        XCTAssertEqual(rows[0].directories?.last?.machine, MachineRef(id: "m-1", label: nil))
        XCTAssertEqual(rows[0].directories?.first?.remotes.first?.repository, repository)
        XCTAssertNil(rows[1].repository)
        XCTAssertNil(rows[1].directories)
        XCTAssertEqual(rows[2].directories, [])

        let sessions = #"""
        [{"id":"1a2b3c4d-0000-4000-8000-00000000000a","project":"gitlab.example.com/acme/ui-kit-web","cwd":"/Users/sample/code/ui-kit-web-lane-2",
          "repository":"gitlab.example.com/acme/ui-kit-web","machine":{"id":"m-1","label":"mac"},"git_branch":"main","entrypoint":"claude-cli/2.1.269",
          "started_at":1,"last_seen_at":2,"requests":4,"prompt_tokens":1000,"cost":0.1,"output_tokens":10,"cache_read_tokens":900,"subagent_tokens":0,"errors":0,"subagents":0,"models":"claude-opus-5"},
         {"id":"2b3c4d5e-0000-4000-8000-00000000000b","project":"/Users/sample/code/notes","git_branch":null,"entrypoint":null,
          "started_at":1,"last_seen_at":2,"requests":4,"prompt_tokens":1000,"cost":0.1,"output_tokens":10,"cache_read_tokens":900,"subagent_tokens":0,"errors":0,"subagents":0,"models":null}]
        """#
        let proxied = try JSONDecoder().decode([ProxySessionRow].self, from: Data(sessions.utf8))
        XCTAssertEqual(proxied.map(\.projectName), ["ui-kit-web", "notes"])
        XCTAssertEqual(proxied[0].cwd, "/Users/sample/code/ui-kit-web-lane-2")
        XCTAssertEqual(proxied[0].repository, repository)
        XCTAssertEqual(proxied[0].machine, MachineRef(id: "m-1", label: "mac"))
        XCTAssertNil(proxied[1].cwd)
        XCTAssertNil(proxied[1].repository)
        XCTAssertNil(proxied[1].machine)
    }

    // ---------- (3) the totals request ----------

    func testTheProjectTotalsAreAskedForByTheRepositoryElseTheDirectory() throws {
        let inRepository = try row("aaaa1111", cwd: "/Users/sample/code/ui-kit-web-lane-2", repository: repository)
        XCTAssertEqual(inRepository.projectKey, repository)
        XCTAssertEqual(LiveFeed.projectTotalsPath(inRepository),
                       "/api/live/project?project=gitlab.example.com%2Facme%2Fui-kit-web")

        let plain = try row("bbbb2222", cwd: "/Users/sample/code/notes")
        XCTAssertEqual(plain.projectKey, "/Users/sample/code/notes")
        XCTAssertEqual(LiveFeed.projectTotalsPath(plain), "/api/live/project?project=%2FUsers%2Fsample%2Fcode%2Fnotes")

        // An empty repository is none; neither, and nothing is asked.
        XCTAssertEqual(try row("cccc3333", cwd: "/Users/sample/code/notes", repository: "").projectKey, "/Users/sample/code/notes")
        XCTAssertNil(LiveFeed.projectTotalsPath(try row("dddd4444", cwd: nil)))

        // Never `cwd=`, and the key arrives as written, whatever it holds.
        for session in [inRepository, plain] {
            let path = try XCTUnwrap(LiveFeed.projectTotalsPath(session))
            let url = try XCTUnwrap(URL(string: path, relativeTo: URL(string: "https://router.invalid")))
            let items = URLComponents(url: url, resolvingAgainstBaseURL: true)?.queryItems
            XCTAssertEqual(url.path, "/api/live/project")
            XCTAssertEqual(items, [URLQueryItem(name: "project", value: session.projectKey)])
        }
        let odd = #"C:\Users\sample\code\a&b+c #1"#
        XCTAssertEqual(RouterClient.queryValue(odd), "C%3A%5CUsers%5Csample%5Ccode%5Ca%26b%2Bc%20%231")
    }
}
