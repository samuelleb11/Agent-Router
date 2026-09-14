import XCTest
@testable import AgentRouter

/// The chat must read the router's stream exactly as the web view does.
final class LiveTests: XCTestCase {
    func testSSEFramesAssembleAndKeepAlivesAreIgnored() {
        var p = SSEParser()
        XCTAssertNil(p.feed(line: #"data: {"a":1}"#))
        XCTAssertEqual(p.feed(line: ""), SSEEvent(event: nil, data: #"{"a":1}"#))
        XCTAssertNil(p.feed(line: "event: status"))
        XCTAssertNil(p.feed(line: "data: {}"))
        XCTAssertEqual(p.feed(line: ""), SSEEvent(event: "status", data: "{}"))
        XCTAssertNil(p.feed(line: ": keep-alive"))
        XCTAssertNil(p.feed(line: ""), "a blank line with nothing pending delivers nothing")
        XCTAssertNil(p.feed(line: "data: a"))
        XCTAssertNil(p.feed(line: "data: b"))
        XCTAssertEqual(p.feed(line: "")?.data, "a\nb")
        XCTAssertNil(p.feed(line: "data:x"))
        XCTAssertEqual(p.feed(line: "")?.data, "x", "the space after the colon is optional")
        // The event name does not leak into the next frame.
        XCTAssertNil(p.feed(line: "data: y"))
        XCTAssertEqual(p.feed(line: "")?.event, nil)
    }

    /// The regression behind an evening of a blank feed: frames end with an empty line,
    /// and the byte splitter must deliver it.
    func testRawBytesBecomeFramesEmptyLinesIncluded() {
        var splitter = SSELineSplitter()
        var parser = SSEParser()
        var events: [SSEEvent] = []
        for b in Array("data: a\n\nevent: status\r\ndata: {}\r\n\r\n: ping\n\n".utf8) {
            if let line = splitter.feed(byte: b), let ev = parser.feed(line: line) { events.append(ev) }
        }
        XCTAssertEqual(events, [SSEEvent(event: nil, data: "a"), SSEEvent(event: "status", data: "{}")])
    }

    func testStreamItemsDecode() throws {
        let text = #"{"t":"2026-09-12T08:30:10.762Z","role":"assistant","kind":"tool_use","name":"Bash","text":"sed -n 1,5p x","dur_ms":1200,"sidechain":false}"#
        let it = try JSONDecoder().decode(LiveItem.self, from: Data(text.utf8))
        XCTAssertEqual(it.kind, "tool_use")
        XCTAssertEqual(it.name, "Bash")
        XCTAssertEqual(it.durMs, 1200)

        let ask = #"{"role":"assistant","kind":"ask","sidechain":false,"ask":{"questions":[{"question":"Which one?","header":"Pick","multiSelect":false,"options":[{"label":"A","description":"first"},{"label":"B"}]}]}}"#
        let a = try JSONDecoder().decode(LiveItem.self, from: Data(ask.utf8))
        let q = try XCTUnwrap(a.ask?.questions.first)
        XCTAssertEqual(q.title, "Pick")
        XCTAssertEqual(q.options?.map(\.label), ["A", "B"])
        XCTAssertFalse(q.isMulti)

        let st = try JSONDecoder().decode(LiveStatus.self, from: Data(#"{"state":"tool","detail":"in Bash","quiet_ms":5200}"#.utf8))
        XCTAssertTrue(st.isBusy)
        XCTAssertEqual(st.quietMs, 5200)
        XCTAssertNil(st.queued, "an older router says nothing about the queue")
        let queued = try JSONDecoder().decode(LiveStatus.self, from: Data(#"{"state":"working","detail":"Working","quiet_ms":10,"queued":2}"#.utf8))
        XCTAssertEqual(queued.queued, 2)
        let aside = try JSONDecoder().decode(AsideReply.self, from: Data(#"{"ok":true,"answer":"Four.","fork":"f-1","cost":0.02,"took_ms":18000}"#.utf8))
        XCTAssertEqual(aside.tookMs, 18000)

        let reply = try JSONDecoder().decode(LiveReply.self, from: Data(#"{"ok":true,"via":"peer","pid":12,"msg_id":"m","framed":true}"#.utf8))
        XCTAssertEqual(reply.framed, true)
        let refused = try JSONDecoder().decode(LiveReply.self, from: Data(#"{"ok":false,"reason":"No such session."}"#.utf8))
        XCTAssertEqual(refused.reason, "No such session.")

        let stats = try JSONDecoder().decode(SessionStats.self, from: Data(#"{"model":"claude-opus-5","effort":"high","input":10,"output":5,"cache_read":100,"cache_write":3,"cache_write_5m":3,"cache_write_1h":0,"cost":0.42,"total":118,"turns":2}"#.utf8))
        XCTAssertEqual(stats.total, 118)

        let shim = try JSONDecoder().decode(ShimStatus.self, from: Data(#"{"available":true,"installed":2,"targets":[{"extension":"anthropic.claude-code-2.1.268","path":"/x","state":"installed"},{"extension":"anthropic.claude-code-2.1.270","path":"/y","state":"not_installed"}]}"#.utf8))
        XCTAssertEqual(shim.newestUnwrapped?.ext, "anthropic.claude-code-2.1.270")
    }

    private func session(_ id: String, project: String, agoHours: Double, active: Bool = false,
                         state: String = "idle", archived: Bool = false, now: Date) -> LiveSession {
        let json = """
        {"session_id":"\(id)","cwd":"/code/\(project)","project":"-code-\(project)","git_branch":null,"title":null,
         "created_ms":0,"last_activity_ms":\(Int(now.addingTimeInterval(-agoHours * 3600).timeIntervalSince1970 * 1000)),
         "active":\(active),"state":"\(state)","archived":\(archived),"wrapped":false,"bytes":10}
        """
        return try! JSONDecoder().decode(LiveSession.self, from: Data(json.utf8))
    }

    func testSessionsGroupLikeTheWebView() {
        let now = Date()
        let rows = [
            session("aaaa", project: "project-b", agoHours: 0.1, active: true, state: "working", now: now),
            session("bbbb", project: "project-a", agoHours: 2, now: now),
            session("cccc", project: "project-a", agoHours: 20, now: now),
            session("dddd", project: "zeta", agoHours: 30, archived: true, now: now),
            session("eeee", project: "project-a", agoHours: 1, active: true, now: now),
            // Put away a moment ago, which is the ordinary case: you archive the session
            // you have just stopped using. The router clears `archived` by itself once it
            // moves again, so recency must not second-guess it here.
            session("ffff", project: "project-b", agoHours: 0.05, archived: true, now: now),
        ]
        let g = SessionGroups.build(rows, now: now)
        XCTAssertEqual(g.recent.map(\.name), ["project-a", "project-b"], "projects alphabetical, not by activity")
        XCTAssertEqual(g.recent[0].sessions.map(\.shortId), ["eeee", "bbbb"], "newest first within a project")
        XCTAssertEqual(g.older.flatMap { $0.sessions.map(\.shortId) }, ["cccc"])
        XCTAssertEqual(g.archived.map(\.shortId), ["ffff", "dddd"], "on the shelf however recently it moved, newest first")
        XCTAssertFalse(g.recent.flatMap(\.sessions).contains { $0.shortId == "ffff" }, "and not also on screen")
        XCTAssertTrue(g.recent[1].live)
        XCTAssertEqual(rows[0].projectName, "project-b")
        XCTAssertEqual(rows[0].label, "aaaa")
    }

    func testAnswersComposeTheWayThePersonWouldHaveTyped() throws {
        let qs = try JSONDecoder().decode([AskQuestion].self, from: Data(#"[{"question":"Colour?","header":"Colour","multiSelect":false,"options":[{"label":"Red"},{"label":"Blue"}]},{"question":"Toppings?","header":"Toppings","multiSelect":true,"options":[{"label":"Olives"},{"label":"Basil"},{"label":"Chili"}]},{"question":"Name?","options":[]}]"#.utf8))
        var sel: [String: AskAnswers.Selection] = [:]
        sel["Colour?"] = .init(chosen: ["Blue"])
        sel["Toppings?"] = .init(chosen: ["Chili", "Olives"], other: "extra cheese")
        let (answers, parts) = AskAnswers.compose(qs, sel)
        XCTAssertEqual(answers["Colour?"], "Blue")
        XCTAssertEqual(answers["Toppings?"], "Olives, Chili, extra cheese", "the tool's option order, then what was typed")
        XCTAssertNil(answers["Name?"], "an unanswered question is left out")
        XCTAssertEqual(parts, ["Colour: Blue", "Toppings: Olives, Chili, extra cheese"])
        // Typing Other alone is an answer; choosing nothing is not.
        XCTAssertEqual(AskAnswers.compose(qs, ["Name?": .init(other: "  Sam ")]).answers["Name?"], "Sam")
        XCTAssertTrue(AskAnswers.compose(qs, ["Name?": .init()]).parts.isEmpty)
    }

    func testMarkdownBlocksMatchTheWebView() {
        let src = "# Title\nSome **bold** text\n\n- one\n- two\n\n1. first\n2) second\n\n> quoted\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```\ncode\n```\n| not | a table |"
        let b = Markdown.blocks(src)
        XCTAssertEqual(b, [
            .heading(1, "Title"), .paragraph("Some **bold** text"),
            .bullets(["one", "two"]), .numbered(["first", "second"]),
            .quote("quoted"), .table([["a", "b"], ["1", "2"]]),
            .code("code"), .paragraph("| not | a table |"),
        ])
        XCTAssertEqual(Markdown.blocks("plain\nlines"), [.paragraph("plain\nlines")])
    }

    func testSwitchPayloadsDecode() throws {
        let o = try JSONDecoder().decode(LiveOptions.self, from: Data(#"{"models":["opus","opus[1m]","sonnet"],"efforts":["low","medium","high","xhigh","max"]}"#.utf8))
        XCTAssertEqual(o.efforts.count, 5)
        let ok = try JSONDecoder().decode(SwitchReply.self, from: Data(#"{"ok":true,"model":"opus","notices":["Switched to opus"]}"#.utf8))
        XCTAssertEqual(ok.notices?.first, "Switched to opus")
        let pending = try JSONDecoder().decode(SwitchReply.self, from: Data(#"{"ok":true,"model":"opus","pending":true}"#.utf8))
        XCTAssertEqual(pending.pending, true)
        let body = try JSONEncoder().encode(EffortBody(session: "s", effort: "xhigh"))
        XCTAssertEqual(String(decoding: body, as: UTF8.self).contains(#""effort":"xhigh""#), true)
    }

    func testCodeFencesSplitOutOfProse() {
        let segs = Markdown.split("Run this:\n```sh\nls -la\n```\nthen **stop**.")
        XCTAssertEqual(segs, [.prose("Run this:"), .code("ls -la"), .prose("then **stop**.")])
        XCTAssertEqual(Markdown.split("plain"), [.prose("plain")])
        XCTAssertEqual(Markdown.split("```\nunterminated"), [.code("unterminated")])
        XCTAssertEqual(String(Markdown.inline("a **b** c").characters), "a b c", "inline markdown is styling, not text")
    }
}
