import SwiftUI
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

    /// A row as the router lists it. `extra` is spliced in as further keys — what a
    /// current router adds; left empty, the row is exactly what an older one sends.
    private func session(_ id: String, project: String, agoHours: Double, active: Bool = false,
                         state: String = "idle", archived: Bool = false, wrapped: Bool = false,
                         extra: String = "", now: Date) -> LiveSession {
        let json = """
        {"session_id":"\(id)","cwd":"/code/\(project)","project":"-code-\(project)","git_branch":null,"title":null,
         "created_ms":0,"last_activity_ms":\(Int(now.addingTimeInterval(-agoHours * 3600).timeIntervalSince1970 * 1000)),
         "active":\(active),"state":"\(state)","archived":\(archived),"wrapped":\(wrapped),"bytes":10\(extra)}
        """
        return try! JSONDecoder().decode(LiveSession.self, from: Data(json.utf8))
    }

    /// A session at work or waiting on you stays in view however long ago it last wrote —
    /// questions have waited five hours, and the transcript is silent while the model
    /// thinks. An ended session folds like any other, and an older router's rows fold
    /// exactly as they always did.
    func testWorkingAndWaitingSessionsAreNeverFoldedAway() {
        let now = Date()
        let rows = [
            session("wait", project: "p", agoHours: 20, active: true, state: "ask",
                    extra: #","activity":"waiting","tone":"attention""#, now: now),
            session("bgbg", project: "p", agoHours: 30, active: true, state: "working",
                    extra: #","activity":"background","tone":"busy""#, now: now),
            session("done", project: "p", agoHours: 20, extra: #","activity":"closed","open":false,"tone":"closed""#, now: now),
            session("fail", project: "q", agoHours: 14, state: "failed", extra: #","activity":"failed","tone":"error""#, now: now),
            // An older router: its `ask` was never enough to hold a row in view, and a
            // phone reading that router must go on showing what it showed before.
            session("olda", project: "q", agoHours: 20, state: "ask", now: now),
        ]
        let g = SessionGroups.build(rows, now: now)
        XCTAssertEqual(g.recent.flatMap { $0.sessions.map(\.shortId) }, ["wait", "bgbg"])
        XCTAssertEqual(Set(g.older.flatMap { $0.sessions.map(\.shortId) }), ["done", "fail", "olda"])
        XCTAssertEqual(g.recent.first?.look.color, .orange, "the project says someone is waiting before it says work")
        XCTAssertTrue(SessionGroups.isCurrent(rows[0], now: now))
        XCTAssertFalse(SessionGroups.isCurrent(rows[2], now: now), "an ended session is not a tab of its own")
        XCTAssertTrue(SessionGroups.isCurrent(rows[4], now: now), "the tab strip always kept an older router's ask")
    }

    /// Every client colours a state from the router's tone, in this app's palette.
    func testDotsTakeTheRoutersTone() {
        let now = Date()
        func dot(_ extra: String, active: Bool = false, state: String = "idle") -> StateLook {
            session("t", project: "p", agoHours: 0, active: active, state: state, extra: extra, now: now).look
        }
        XCTAssertEqual(dot(#","activity":"generating","tone":"busy""#, active: true, state: "working"),
                       StateLook(color: .green, hollow: false, halo: true, label: "working"))
        XCTAssertEqual(dot(#","activity":"tool","tone":"busy""#, active: true, state: "working").color, .green)
        XCTAssertEqual(dot(#","activity":"background","tone":"busy""#, active: true, state: "working").color, .green)
        XCTAssertEqual(dot(#","activity":"waiting","tone":"attention""#, active: true, state: "ask").color, .orange)
        XCTAssertEqual(dot(#","activity":"stalled","tone":"warn""#, active: true, state: "stalled").color, .yellow)
        XCTAssertEqual(dot(#","activity":"failed","tone":"error""#, state: "failed").color, .red)
        XCTAssertEqual(dot(#","activity":"idle","tone":"idle""#), StateLook(color: .gray, hollow: false, halo: false, label: "idle"))
        XCTAssertEqual(dot(#","activity":"closed","tone":"closed""#), StateLook(color: .gray, hollow: true, halo: false, label: "ended"))
        // No tone: the activity's own. Neither: the legacy state, as before.
        XCTAssertEqual(dot(#","activity":"stalled""#, active: true, state: "stalled").color, .yellow)
        XCTAssertTrue(dot(#","activity":"closed""#).hollow)
        XCTAssertEqual(dot("", state: "stalled").color, .yellow)
        // `tone` wins over the legacy word it would otherwise be read from.
        XCTAssertEqual(dot(#","activity":"background","tone":"busy""#, active: true, state: "idle").color, .green)
    }

    /// The router row and the router page count what is at work and what is waiting on
    /// you; an ended session is neither. An older router still says "N active".
    @MainActor
    func testCountsSayWorkingAndWaiting() {
        let now = Date()
        let state = RouterState(router: Router(name: "Mac", host: "127.0.0.1"), password: nil)
        state.sessions = [
            session("gen1", project: "p", agoHours: 0, active: true, state: "working", extra: #","activity":"generating""#, now: now),
            session("bg01", project: "p", agoHours: 1, active: true, state: "working", extra: #","activity":"background""#, now: now),
            session("tool", project: "p", agoHours: 0, active: true, state: "working", extra: #","activity":"tool""#, now: now),
            session("stal", project: "p", agoHours: 0, active: true, state: "stalled", extra: #","activity":"stalled""#, now: now),
            session("wait", project: "p", agoHours: 9, active: true, state: "ask", extra: #","activity":"waiting""#, now: now),
            session("done", project: "p", agoHours: 0, extra: #","activity":"closed","open":false"#, now: now),
            session("idle", project: "p", agoHours: 0, extra: #","activity":"idle""#, now: now),
            session("fail", project: "p", agoHours: 0, state: "failed", extra: #","activity":"failed""#, now: now),
        ]
        // Working is generating, tool, background and stalled; waiting is waiting — the web
        // view's count. Failed, idle and ended are neither.
        XCTAssertEqual(state.sessionCounts?.working, 4)
        XCTAssertEqual(state.sessionCounts?.waiting, 1)
        XCTAssertEqual(state.liveSummary, "4 working · 1 waiting")
        state.sessions = [session("idle", project: "p", agoHours: 0, extra: #","activity":"idle""#, now: now)]
        XCTAssertNil(state.liveSummary)

        state.sessions = [
            session("a", project: "p", agoHours: 0, active: true, state: "working", now: now),
            session("b", project: "p", agoHours: 0, active: true, now: now),
            session("c", project: "p", agoHours: 5, now: now),
        ]
        XCTAssertNil(state.sessionCounts)
        XCTAssertEqual(state.liveSummary, "2 active")
    }

    /// Every activity draws — a row with its line and capsule, and its dot — from both a
    /// current router's payload and an older one's. The rows are shaped as the router
    /// shapes them: `background` only with background work, an end only on an ended session.
    @MainActor
    func testRowsAndDotsDrawForEveryActivity() throws {
        let now = Date()
        let asOf = Int(now.timeIntervalSince1970 * 1000)
        let since = asOf - 95_000
        var rows = ["closed", "idle", "generating", "tool", "waiting", "background", "failed", "stalled", "compacting"].map { a in
            let own = a == "background" ? #","background":{"agents":1,"workflows":0,"shells":0}"#
                : a == "closed" ? #","open":false,"ended_ms":\#(since),"ended_how":"exit","mid_turn":true"# : ""
            return session(a, project: "p", agoHours: 0.2, active: a != "closed" && a != "idle" && a != "failed", wrapped: a != "closed",
                           extra: #","activity":"\#(a)","detail":"Doing \#(a)","since_ms":\#(since),"as_of_ms":\#(asOf)"# + own,
                           now: now)
        }
        for r in rows {
            XCTAssertNotNil(r.doing, "a current router's row always says what it is doing: \(r.sessionId)")
        }
        rows.append(session("legacy", project: "p", agoHours: 1, active: true, state: "working", now: now))
        XCTAssertNil(rows.last?.doing, "an older router's row has no line, as before")
        for r in rows {
            let row = ImageRenderer(content: SessionRowView(session: r).frame(width: 390))
            XCTAssertNotNil(row.uiImage, r.sessionId)
            XCTAssertNotNil(ImageRenderer(content: StateDot(look: r.look)).uiImage, r.sessionId)
        }
    }

    /// Waits for something the feed does on the main actor, without hanging the suite.
    @MainActor
    private func eventually(_ what: String, _ condition: () -> Bool) async throws {
        let end = ContinuousClock.now + .seconds(3)
        while !condition() {
            guard ContinuousClock.now < end else { return XCTFail("never happened: \(what)") }
            try await Task.sleep(for: .milliseconds(10))
        }
    }

    /// A phone that walks off the LAN hears no error for 45 s. The header must stop
    /// vouching for the last status long before that, and the connection is dropped so
    /// the retry can start afresh.
    @MainActor
    func testASilentStreamStopsVouchingForItsLastStatus() async throws {
        let feed = LiveFeed(session: session("abcd", project: "p", agoHours: 0, wrapped: true, now: .now), client: nil)
        feed.staleAfter = .milliseconds(300)
        let (events, input) = AsyncThrowingStream<SSEEvent, Error>.makeStream()
        let started = ContinuousClock.now
        let connection = Task { try await feed.follow(events) }
        input.yield(SSEEvent(event: "status", data: #"{"state":"tool","detail":"Running Bash","quiet_ms":1200,"activity":"tool","tone":"busy"}"#))
        try await eventually("the first status") { feed.connected }
        XCTAssertTrue(feed.canStop)
        do {
            try await connection.value
            XCTFail("silence must end the connection")
        } catch is LiveFeed.Silence {}
        XCTAssertGreaterThanOrEqual(ContinuousClock.now - started, .milliseconds(300))
        XCTAssertFalse(feed.connected)
        XCTAssertFalse(feed.canStop, "no Stop on a status nothing vouches for any more")
        XCTAssertEqual(feed.status?.detail, "Running Bash", "kept, so the header says reconnecting rather than connecting")
        input.finish()
    }

    /// Rows arrive before the first status and do not vouch for one; a stream the router
    /// closes leaves the header reconnecting, not frozen on its last line.
    @MainActor
    func testAClosedStreamLeavesTheHeaderReconnecting() async throws {
        let feed = LiveFeed(session: session("abcd", project: "p", agoHours: 0, wrapped: true, now: .now), client: nil)
        let (events, input) = AsyncThrowingStream<SSEEvent, Error>.makeStream()
        let connection = Task { try await feed.follow(events) }
        input.yield(SSEEvent(event: nil, data: #"{"role":"assistant","kind":"text","text":"hello","sidechain":false}"#))
        try await eventually("the backfilled row") { feed.rows.count == 1 }
        XCTAssertFalse(feed.connected, "a row is not a status: the header still says connecting")
        input.yield(SSEEvent(event: "status", data: #"{"state":"working","detail":"Thinking","quiet_ms":10}"#))
        try await eventually("the first status") { feed.connected }
        XCTAssertTrue(feed.canStop, "an older router's `working` still offers Stop")
        input.finish()
        try await connection.value
        XCTAssertFalse(feed.connected)
        XCTAssertNotNil(feed.status)
    }

    /// Leaving the page ends the connection at once, not when the silence timer would.
    @MainActor
    func testLeavingThePageEndsTheStreamPromptly() async throws {
        let feed = LiveFeed(session: session("abcd", project: "p", agoHours: 0, now: .now), client: nil)
        feed.staleAfter = .seconds(60)
        let (events, input) = AsyncThrowingStream<SSEEvent, Error>.makeStream()
        let connection = Task { try await feed.follow(events) }
        try await Task.sleep(for: .milliseconds(50))
        let cancelled = ContinuousClock.now
        connection.cancel()
        _ = try? await connection.value
        XCTAssertLessThan(ContinuousClock.now - cancelled, .seconds(2))
        XCTAssertFalse(feed.connected)
        input.finish()
    }

    /// An answer that names no prompt lands on whichever one is waiting; with a permission
    /// request waiting and the question's id unknown, that would allow the request. So
    /// nothing is sent — not the answer, not a message — and the form keeps its choices.
    /// (The client points at a port nothing listens on: the look at what is waiting fails
    /// there and changes nothing, and the guard must return before any answer or message
    /// is posted — a broken one fails here instead of reaching anything real.)
    @MainActor
    func testAQuestionIsNeverAnsweredOntoAPermissionRequest() async throws {
        let feed = LiveFeed(session: session("abcd", project: "p", agoHours: 0, wrapped: true, now: .now),
                            client: RouterClient(baseURL: URL(string: "http://127.0.0.1:9")!, password: nil))
        feed.pendingPrompt = try JSONDecoder().decode(LivePrompt.self, from: Data(#"{"pending":true,"request_id":"perm-1","tool":"Bash","question":false}"#.utf8))
        await feed.answer(["Which?": "B"], questions: ["Which?"], fallback: ["Which: B"], row: UUID())
        XCTAssertEqual(feed.notice?.ok, false)
        XCTAssertEqual(feed.notice?.text, "A permission request is waiting too — answer it first, then submit these again.")
        XCTAssertTrue(feed.rows.isEmpty, "nothing was sent, not even as a message")
        XCTAssertNotNil(feed.pendingPrompt, "the permission card stays up")
    }

    /// Stop is for a turn that is running on a wrapped session, read from a live stream:
    /// never for background work alone, an ended session, or a status gone stale.
    @MainActor
    func testStopIsOfferedOnlyForARunningTurn() throws {
        let feed = LiveFeed(session: session("abcd", project: "p", agoHours: 0, wrapped: true, now: .now), client: nil)
        func offers(_ json: String) throws -> Bool {
            feed.status = try JSONDecoder().decode(LiveStatus.self, from: Data(json.utf8))
            feed.connected = true
            return feed.canStop
        }
        XCTAssertTrue(try offers(#"{"state":"working","detail":"Thinking","quiet_ms":1,"activity":"generating"}"#))
        XCTAssertTrue(try offers(#"{"state":"tool","detail":"Running Bash","quiet_ms":1,"activity":"tool"}"#))
        XCTAssertTrue(try offers(#"{"state":"working","detail":"No progress","quiet_ms":1,"activity":"stalled"}"#))
        XCTAssertFalse(try offers(#"{"state":"working","detail":"1 background agent running","quiet_ms":1,"activity":"background"}"#))
        XCTAssertFalse(try offers(#"{"state":"ask","detail":"Waiting for your answer","quiet_ms":1,"activity":"waiting"}"#))
        XCTAssertFalse(try offers(#"{"state":"idle","detail":"Session ended","quiet_ms":1,"activity":"closed"}"#))
        XCTAssertTrue(try offers(#"{"state":"working","detail":"Working","quiet_ms":1}"#))
        feed.connected = false
        XCTAssertFalse(feed.canStop, "the last status is history once the stream is gone")

        let unwrapped = LiveFeed(session: session("efgh", project: "p", agoHours: 0, now: .now), client: nil)
        unwrapped.status = try JSONDecoder().decode(LiveStatus.self, from: Data(#"{"state":"working","detail":"Thinking","quiet_ms":1,"activity":"generating"}"#.utf8))
        unwrapped.connected = true
        XCTAssertFalse(unwrapped.canStop, "nothing would reach an unwrapped session")
    }

    /// A form stays on screen after its question was answered in the editor, and the model
    /// may have asked another since. Its answers go only to the question it was drawn for —
    /// the same questions in the same order, the web view's rule — and otherwise nothing is
    /// sent, not even as a message: named or not, the router would put them on the question
    /// waiting now, and the editor's dialog for that one would close unseen.
    @MainActor
    func testAnObsoleteQuestionFormIsNotSentOntoTheLiveOne() async throws {
        func prompt(_ json: String) throws -> LivePrompt { try JSONDecoder().decode(LivePrompt.self, from: Data(json.utf8)) }
        let q2 = try prompt(#"{"pending":true,"request_id":"req-Q2","tool":"AskUserQuestion","question":true,"input":{"questions":[{"question":"Which database?","header":"DB","multiSelect":false,"options":[{"label":"Postgres"},{"label":"SQLite"}]}]},"suggestions":[],"count":1}"#)
        let different = LiveFeed.AnswerTarget.refused("Not sent: a different question is waiting now.")
        XCTAssertEqual(LiveFeed.target(for: ["Which framework?"], question: q2, permission: nil), different)
        XCTAssertEqual(LiveFeed.target(for: ["Which database?"], question: q2, permission: nil), .named("req-Q2"))
        XCTAssertEqual(LiveFeed.target(for: ["Which database?", "Which cache?"], question: q2, permission: nil), different,
                       "every question the form asks, not just the first")
        // A prompt that does not list its questions is taken at its word, as on the web view.
        let bare = try prompt(#"{"pending":true,"request_id":"req-Q3","tool":"AskUserQuestion","question":true}"#)
        XCTAssertEqual(LiveFeed.target(for: ["Anything?"], question: bare, permission: nil), .named("req-Q3"))
        let permission = try prompt(#"{"pending":true,"request_id":"perm-1","tool":"Bash","question":false}"#)
        XCTAssertEqual(LiveFeed.target(for: ["Which?"], question: nil, permission: permission),
                       .refused("A permission request is waiting too — answer it first, then submit these again."))
        XCTAssertEqual(LiveFeed.target(for: ["Which?"], question: nil, permission: nil), .unnamed,
                       "nothing under the wrapper to name: the router answers a lone prompt or says why not")

        // Through the feed, the reviewer's case: Q1's form submitted while Q2 waits. The client
        // points at a port nothing listens on, so a send that should not happen fails here.
        let feed = LiveFeed(session: session("abcd", project: "p", agoHours: 0, wrapped: true, now: .now),
                            client: RouterClient(baseURL: URL(string: "http://127.0.0.1:9")!, password: nil))
        feed.questionPrompt = q2
        await feed.answer(["Which framework?": "React"], questions: ["Which framework?"], fallback: ["Framework: React"], row: UUID())
        XCTAssertEqual(feed.notice, LiveFeed.Notice(text: "Not sent: a different question is waiting now.", ok: false))
        XCTAssertTrue(feed.rows.isEmpty, "not sent as a message either")
    }

    /// Every row from a current router says what the session is doing and for how long —
    /// idle and ended included, as the web view says it — measured on the router's clock
    /// (`as_of_ms`), never this phone's. The router clock here is eight days behind the
    /// device's, so a line measured on the device would read days, not minutes.
    func testEveryRowSaysWhatItIsDoingAndSinceWhen() {
        let asOf = 1_789_990_000_000
        func line(_ activity: String, _ detail: String, since: Int? = nil, ended: Int? = nil,
                  asOf: Int? = asOf) -> String? {
            var keys = #","activity":"\#(activity)","detail":"\#(detail)""#
            if let since { keys += #","since_ms":\#(since)"# }
            if let ended { keys += #","ended_ms":\#(ended)"# }
            if let asOf { keys += #","as_of_ms":\#(asOf)"# }
            return session("a", project: "p", agoHours: 0, extra: keys, now: .now).doing
        }
        XCTAssertEqual(line("idle", "Waiting for you", since: asOf - 3_600_000), "Waiting for you · 1h 00m")
        // The real shape (3d034b56): the shell is named in `detail`, with no `background` key.
        XCTAssertEqual(line("idle", "Waiting for you · background shell running", since: asOf - 36_000_000),
                       "Waiting for you · background shell running · 10h 00m")
        XCTAssertEqual(line("closed", "Session ended", since: asOf - 10_800_000, ended: asOf - 10_800_000),
                       "Session ended · 3 h ago")
        XCTAssertEqual(line("closed", "Session ended mid-turn", ended: asOf - 120_000), "Session ended mid-turn · 2 min ago")
        XCTAssertEqual(line("closed", "Session ended"), "Session ended", "no end on record: the router's words alone")
        XCTAssertEqual(line("failed", "API error: 529 Overloaded", since: asOf - 60_000), "API error: 529 Overloaded · 1m 00s")
        XCTAssertEqual(line("generating", "Thinking", since: asOf - 95_000), "Thinking · 1m 35s")
        XCTAssertEqual(line("waiting", "Bash wants to run", since: asOf - 60_000), "Bash wants to run · 1m 00s")
        XCTAssertEqual(line("tool", "Running Bash", since: asOf - 4_000), "Running Bash", "a count of a few seconds would only flicker")
        XCTAssertEqual(line("tool", "Running Bash", since: asOf - 5_000), "Running Bash · 5s")
        XCTAssertEqual(line("compacting", "Compacting", since: asOf - 30_000), "Compacting · 30s",
                       "an activity this build does not know still gets its line")
        let device = Int(Date().timeIntervalSince1970 * 1000)
        XCTAssertEqual(line("generating", "Thinking", since: device - 95_000, asOf: nil), "Thinking",
                       "without the router's clock there is nothing to measure against: never this phone's")
        XCTAssertNil(session("olde", project: "p", agoHours: 0, active: true, state: "working", now: .now).doing,
                     "an older router's rows stay exactly as they were")
    }

    /// The open view's header says how long ago an ended session ended on the router's
    /// clock, as its row does: a phone whose clock is off must not say "12 min ago" for two.
    func testTheHeaderMeasuresEverythingOnTheRoutersClock() throws {
        func status(_ json: String) throws -> LiveStatus { try JSONDecoder().decode(LiveStatus.self, from: Data(json.utf8)) }
        let asOf = 1_789_990_000_000
        let ended = try status(#"{"state":"idle","detail":"Session ended","quiet_ms":120000,"queued":0,"activity":"closed","open":false,"ended_ms":\#(asOf - 120_000),"as_of_ms":\#(asOf),"tone":"closed"}"#)
        XCTAssertEqual(ended.line, "Session ended · 2 min ago")
        let thinking = try status(#"{"state":"working","detail":"Thinking","quiet_ms":95000,"queued":2,"activity":"generating","since_ms":\#(asOf - 95_000),"as_of_ms":\#(asOf),"tone":"busy"}"#)
        XCTAssertEqual(thinking.line, "Thinking · 1m 35s · 2 queued")
        XCTAssertEqual(try status(#"{"state":"tool","detail":"Running Bash","quiet_ms":1200,"activity":"tool"}"#).line, "Running Bash")
        XCTAssertEqual(try status(#"{"state":"working","detail":"Working","quiet_ms":7000}"#).line, "Working · 7s",
                       "an older router's quiet_ms, as before")
    }

    /// One ranking for a project's dot on the web view and both phones: waiting on you, then
    /// an error, then a stall, then work, then idle — and the empty ring only once every
    /// session in it has ended. From an older router, whether anything is live, as before.
    func testTheProjectDotRanksTheSameEverywhere() {
        let now = Date()
        func dot(_ activities: [String]) -> StateLook {
            let rows = activities.enumerated().map { i, a in
                session("s\(i)", project: "p", agoHours: 0, extra: #","activity":"\#(a)""#, now: now)
            }
            return SessionGroups.Project(name: "p", sessions: rows).look
        }
        XCTAssertEqual(dot(["stalled", "closed"]), StateLook(color: .yellow, hollow: false, halo: true, label: "stalled"))
        XCTAssertEqual(dot(["generating", "stalled"]).color, .yellow, "a stall is not hidden behind the work beside it")
        XCTAssertEqual(dot(["generating", "failed"]), StateLook(color: .red, hollow: false, halo: true, label: "failed"))
        XCTAssertEqual(dot(["failed", "stalled"]).color, .red)
        XCTAssertEqual(dot(["failed", "waiting", "stalled", "tool"]), StateLook(color: .orange, hollow: false, halo: true, label: "waiting for you"))
        XCTAssertEqual(dot(["idle", "background"]), StateLook(color: .green, hollow: false, halo: true, label: "working"))
        XCTAssertEqual(dot(["idle", "closed"]), StateLook(color: .gray, hollow: false, halo: false, label: "idle"))
        XCTAssertEqual(dot(["closed", "closed"]), StateLook(color: .gray, hollow: true, halo: false, label: "ended"))
        XCTAssertEqual(dot(["compacting", "closed"]), StateLook(color: .gray, hollow: false, halo: false, label: "idle"),
                       "a word this build does not know is neutral, and not an ended project")
        // An older router's rows carry no activity: live or not, exactly as before.
        let legacy = [session("l1", project: "p", agoHours: 0, active: true, state: "working", now: now),
                      session("l2", project: "p", agoHours: 0, state: "stalled", now: now)]
        XCTAssertEqual(SessionGroups.Project(name: "p", sessions: legacy).look,
                       StateLook(color: .green, hollow: false, halo: true, label: "working"))
        XCTAssertEqual(SessionGroups.Project(name: "p", sessions: [legacy[1]]).look,
                       StateLook(color: .gray, hollow: false, halo: false, label: "idle"))
    }

    /// Ten seconds of nothing on the stream — no row, no status — and the header stops
    /// vouching: the same limit on the web view and both phones. The reconnect that follows
    /// replays the backfill, which replaces the feed rather than doubling it.
    @MainActor
    func testSilenceMeansTenSecondsAndAReconnectRebuildsTheFeed() async throws {
        let feed = LiveFeed(session: session("abcd", project: "p", agoHours: 0, wrapped: true, now: .now), client: nil)
        XCTAssertEqual(feed.staleAfter, .seconds(10))
        let backfill = [#"{"role":"user","kind":"text","text":"hi","sidechain":false}"#,
                        #"{"role":"assistant","kind":"text","text":"hello","sidechain":false}"#]
        for attempt in 1...2 {
            let (events, input) = AsyncThrowingStream<SSEEvent, Error>.makeStream()
            let connection = Task { try await feed.follow(events) }
            for row in backfill { input.yield(SSEEvent(event: nil, data: row)) }
            input.finish()
            try await connection.value
            XCTAssertEqual(feed.rows.map(\.item.text), ["hi", "hello"], "connection \(attempt)")
        }
    }

    /// The open session's tab says what its stream says, like the header beside it — the
    /// list row under it can be five seconds old — and goes back to the row once the stream
    /// stops vouching. Every other tab is its list row.
    @MainActor
    func testTheOpenTabFollowsTheStream() throws {
        let now = Date()
        let open = session("open", project: "p", agoHours: 0, wrapped: true, extra: #","activity":"idle","tone":"idle""#, now: now)
        let other = session("othr", project: "p", agoHours: 0, extra: #","activity":"idle","tone":"idle""#, now: now)
        let feed = LiveFeed(session: open, client: nil)
        feed.status = try JSONDecoder().decode(LiveStatus.self, from: Data(#"{"state":"tool","detail":"Running Bash","quiet_ms":1200,"activity":"tool","tone":"busy"}"#.utf8))
        feed.connected = true
        XCTAssertEqual(feed.look(of: open), StateLook(color: .green, hollow: false, halo: true, label: "running a tool"))
        XCTAssertEqual(feed.look(of: other), other.look)
        feed.connected = false
        XCTAssertEqual(feed.look(of: open), open.look, "a status nothing vouches for is not shown as current")
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

    /// A local router's rows, which name no machine, and a hosted router's rows from a single
    /// machine list exactly as they always did: one part, no machine heading, and in it
    /// what `build` makes of every row.
    func testOneMachineOrNoneListsExactlyAsBefore() {
        let now = Date()
        func rows(_ machine: String) -> [LiveSession] {
            [
                session("aaaa", project: "project-b", agoHours: 0.1, active: true, state: "working", extra: machine, now: now),
                session("bbbb", project: "project-a", agoHours: 2, extra: machine, now: now),
                session("cccc", project: "project-a", agoHours: 20, extra: machine, now: now),
                session("dddd", project: "zeta", agoHours: 30, archived: true, extra: machine, now: now),
            ]
        }
        for (what, list) in [("a local router", rows("")),
                             ("one machine", rows(#","machine":{"id":"5f0c2a8e","label":"studio"}"#))] {
            let parts = SessionGroups.byMachine(list, now: now)
            XCTAssertEqual(parts.count, 1, what)
            XCTAssertNil(parts.first?.name, "\(what): no machine heading")
            XCTAssertEqual(parts.first?.groups, SessionGroups.build(list, now: now), what)
            let g = parts.first?.groups
            XCTAssertEqual(g?.recent.map(\.name), ["project-a", "project-b"], what)
            XCTAssertEqual(g?.older.flatMap { $0.sessions.map(\.shortId) }, ["cccc"], what)
            XCTAssertEqual(g?.archived.map(\.shortId), ["dddd"], what)
        }
        XCTAssertEqual(SessionGroups.byMachine([], now: now).map(\.name), [nil], "an empty list is one empty part")
    }

    /// Sessions from two machines: each machine under its own heading, alphabetically, and
    /// under it exactly what that machine's sessions alone would make — the same project on
    /// both stays two projects, and each machine has its own older and archived sessions.
    func testTwoMachinesEachGetTheirOwnHeading() {
        let now = Date()
        let studio = #","machine":{"id":"5f0c2a8e","label":"studio"}"#
        let laptop = #","machine":{"id":"9b3e1d47","label":"Laptop"}"#
        let rows = [
            session("s-ar", project: "agent-router", agoHours: 0.1, active: true, state: "working", extra: studio, now: now),
            session("l-ar", project: "agent-router", agoHours: 1, extra: laptop, now: now),
            session("s-ns", project: "northstar", agoHours: 2, extra: studio, now: now),
            session("s-old", project: "northstar", agoHours: 30, extra: studio, now: now),
            session("l-arc", project: "agent-router", agoHours: 3, archived: true, extra: laptop, now: now),
        ]
        let parts = SessionGroups.byMachine(rows, now: now)
        XCTAssertEqual(parts.map(\.name), ["Laptop", "studio"], "machines alphabetically, whatever their case")
        XCTAssertEqual(Set(parts.map(\.id)).count, 2, "one identity per machine")
        guard parts.count == 2 else { return XCTFail("two machines, two parts") }
        let (l, s) = (parts[0].groups, parts[1].groups)
        XCTAssertEqual(l.recent.map(\.name), ["agent-router"])
        XCTAssertEqual(l.recent.flatMap { $0.sessions.map(\.shortId) }, ["l-ar"])
        XCTAssertTrue(l.older.isEmpty)
        XCTAssertEqual(l.archived.map(\.shortId), ["l-arc"], "a machine's shelf is under that machine")
        XCTAssertEqual(s.recent.map(\.name), ["agent-router", "northstar"])
        XCTAssertEqual(s.recent[0].sessions.map(\.shortId), ["s-ar"], "the same project on another machine is another project")
        XCTAssertEqual(s.older.flatMap { $0.sessions.map(\.shortId) }, ["s-old"])
        XCTAssertTrue(s.archived.isEmpty)
        XCTAssertEqual(s, SessionGroups.build(rows.filter { $0.machineName == "studio" }, now: now))
        XCTAssertEqual(l, SessionGroups.build(rows.filter { $0.machineName == "Laptop" }, now: now))

        // A row the router named no machine for, beside machines it did name, is not lost:
        // it has a heading of its own, after the named ones.
        let stray = session("none", project: "agent-router", agoHours: 0.5, now: now)
        let withStray = SessionGroups.byMachine(rows + [stray], now: now)
        XCTAssertEqual(withStray.map(\.name), ["Laptop", "studio", SessionGroups.unnamedMachine])
        XCTAssertEqual(withStray.last?.groups.recent.flatMap { $0.sessions.map(\.shortId) }, ["none"])
        XCTAssertEqual(Set(withStray.map(\.id)).count, 3)
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
