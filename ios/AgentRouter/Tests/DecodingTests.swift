import XCTest
@testable import AgentRouter

/// The router's real payloads decode. The forecast fixture is a captured
/// `/api/forecast` with the identities replaced.
final class DecodingTests: XCTestCase {
    func testTokenOnlyRouterAuthStatusKeepsActiveTokenCount() throws {
        let auth = try JSONDecoder().decode(AuthStatus.self, from: Data(#"{"configured":false,"access_tokens":3,"remote_bind":true,"from_loopback":false,"mode":"server"}"#.utf8))
        XCTAssertEqual(auth.accessTokens, 3, "A token-only router must not be reported as unguarded")
        XCTAssertTrue(auth.hasCredentials)
        XCTAssertEqual(auth.credentialStatus, "access tokens")
        for tokens in ["", #","access_tokens":null"#, #","access_tokens":0"#] {
            let data = Data("{\"configured\":false,\"remote_bind\":true,\"from_loopback\":false\(tokens)}".utf8)
            let empty = try JSONDecoder().decode(AuthStatus.self, from: data)
            XCTAssertEqual(empty.accessTokens, 0)
            XCTAssertFalse(empty.hasCredentials)
        }
        let legacy = try JSONDecoder().decode(AuthStatus.self, from: Data(#"{"configured":true,"remote_bind":true,"from_loopback":false}"#.utf8))
        XCTAssertTrue(legacy.hasCredentials)
    }

    func testCodexSessionIsNamedAndWatchedEvenIfWrappedIsSet() throws {
        let s = try JSONDecoder().decode(LiveSession.self, from: Data(#"{"session_id":"codex:12345678-abcd","project":"app","last_activity_ms":0,"active":false,"state":"idle","archived":false,"wrapped":true,"bytes":0,"client":"codex","hosted":false}"#.utf8))
        XCTAssertEqual(s.clientName, "Codex")
        XCTAssertEqual(s.shortId, "12345678")
        XCTAssertFalse(s.canControl)
        XCTAssertFalse(s.canSend)
        XCTAssertFalse(s.canAskAside)
        XCTAssertNotNil(s.watchedReason)
    }

    func testOpenAIAccountDecodesItsCodexLoginAndEncodesAPIKeyProvider() throws {
        let a = try JSONDecoder().decode(Account.self, from: Data(#"{"id":"codex-default","label":"Codex login","active":1,"source":"codex","provider":"openai"}"#.utf8))
        XCTAssertEqual(a.providerName, "OpenAI")
        XCTAssertTrue(a.isCodexLogin)
        XCTAssertFalse(a.takesUsageKey)
        let body = AddAccountBody(label: "OpenAI", source: "token", token: "sk-proj-example", provider: "openai")
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(body)) as? [String: String])
        XCTAssertEqual(fields["provider"], "openai")
        XCTAssertEqual(fields["source"], "token")
    }

    private func fixture(_ name: String) throws -> Data {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: name, withExtension: "json"))
        return try Data(contentsOf: url)
    }

    func testForecastFixtureDecodes() throws {
        let f = try JSONDecoder().decode(Forecast.self, from: fixture("forecast"))
        XCTAssertEqual(f.accounts.count, 3)
        XCTAssertEqual(f.accounts.filter(\.isActive).count, 1, "exactly one account serves")
        XCTAssertGreaterThan(f.global.cost, 0)
        XCTAssertGreaterThan(f.global.pace30d.cost, f.global.cost, "a month is longer than a week")
        XCTAssertEqual(f.global.accounts, 3)
        // Every shape a window takes is in the fixture: absent, expired, and projected.
        let windows = f.accounts.flatMap { [$0.fiveHour, $0.sevenDay] }
        XCTAssertTrue(windows.contains { $0?.isExpired == true }, "an expired stub")
        XCTAssertTrue(windows.contains { $0?.series?.isEmpty == false }, "a live window with a series")
        let live = try XCTUnwrap(windows.first { $0?.isExpired == false && $0?.activeHoursLeft != nil })
        XCTAssertNotNil(live?.resetDate)
        XCTAssertNotNil(live?.startDate)
    }

    func testQuotaResetsAtInBothSpellings() throws {
        let headers = #"{"five_hour":{"utilization":3.0,"resets_at":"@1789213800"},"seven_day":{"utilization":9.0,"resets_at":"@1789750800"},"severity":"normal","seen_at":1789198720215,"account_id":"a","source":"response-headers","age_ms":185670}"#
        let q = try JSONDecoder().decode(Quota.self, from: Data(headers.utf8))
        XCTAssertEqual(q.fiveHour?.fraction, 0.03)
        XCTAssertEqual(q.fiveHour?.resetDate?.timeIntervalSince1970, 1_789_213_800)
        let iso = #"{"five_hour":{"utilization":42,"resets_at":"2026-09-12T07:50:00Z"},"severity":"warning"}"#
        let q2 = try JSONDecoder().decode(Quota.self, from: Data(iso.utf8))
        XCTAssertEqual(q2.fiveHour?.resetDate, ISO8601DateFormatter().date(from: "2026-09-12T07:50:00Z"))
        XCTAssertNil(q2.sevenDay)
        // The endpoint answers `{"error": …}` with 200 when it has nothing; that must not fail.
        let empty = try JSONDecoder().decode(Quota.self, from: Data(#"{"error":"no credential"}"#.utf8))
        XCTAssertNil(empty.fiveHour)
    }

    func testAccountsAuthAndHealth() throws {
        let accounts = #"[{"id":"personal","label":"Personal","source":"keychain","profile_dir":null,"email":"me@example.test","org_name":null,"subscription_type":"max","active":1,"created_at":1,"usage_key":1,"has_token":true,"ready":true,"detail":"sk-ant-oat01 (subscription OAuth)","expires_at":0},{"id":"work","label":"Work","source":"token","email":null,"active":0,"created_at":2,"usage_key":0,"has_token":false,"ready":false,"detail":"no token stored","expires_at":0}]"#
        let list = try JSONDecoder().decode([Account].self, from: Data(accounts.utf8))
        XCTAssertEqual(list.map(\.title), ["me@example.test", "Work"])
        XCTAssertTrue(list[0].isActive)
        XCTAssertEqual(list[1].ready, false)

        let auth = try JSONDecoder().decode(AuthStatus.self, from: Data(#"{"configured":true,"updated_at":1789202515395,"remote_bind":true,"from_loopback":false}"#.utf8))
        XCTAssertTrue(auth.configured && auth.remoteBind && !auth.fromLoopback)

        let health = try JSONDecoder().decode(Health.self, from: Data(#"{"ok":true,"pid":48291,"version":"0.1.0"}"#.utf8))
        XCTAssertEqual(health.pid, 48291)
    }

    /// `/api/auth` says how the router is run: a hosted router adds `mode`, `password_pinned`
    /// and `public_url`, a current local router says it is local, and one from before hosting
    /// sends none of the three — and is local.
    func testAuthSaysWhetherTheRouterIsHosted() throws {
        func auth(_ json: String) throws -> AuthStatus { try JSONDecoder().decode(AuthStatus.self, from: Data(json.utf8)) }
        let hosted = try auth(#"{"configured":true,"updated_at":1790000000000,"from_loopback":false,"mode":"server","password_pinned":true,"public_url":"https://agent-router.example.com","remote_bind":true,"lan_url":"https://agent-router.example.com","env_override":false}"#)
        XCTAssertEqual(hosted.mode, "server")
        XCTAssertEqual(hosted.passwordPinned, true)
        XCTAssertEqual(hosted.publicURL, "https://agent-router.example.com")
        XCTAssertTrue(hosted.isHosted)
        XCTAssertTrue(hosted.configured && hosted.remoteBind && !hosted.fromLoopback)

        let local = try auth(#"{"configured":true,"updated_at":null,"from_loopback":true,"mode":"local","password_pinned":false,"public_url":null,"remote_bind":false}"#)
        XCTAssertEqual(local.mode, "local")
        XCTAssertEqual(local.passwordPinned, false)
        XCTAssertNil(local.publicURL)
        XCTAssertFalse(local.isHosted)

        let old = try auth(#"{"configured":true,"updated_at":1789202515395,"remote_bind":true,"from_loopback":false}"#)
        XCTAssertNil(old.mode)
        XCTAssertNil(old.passwordPinned)
        XCTAssertNil(old.publicURL)
        XCTAssertFalse(old.isHosted, "a router that names no mode is local")
        XCTAssertEqual(old.updatedAt, 1_789_202_515_395)
        XCTAssertTrue(old.configured && old.remoteBind && !old.fromLoopback)

        // A shape this build does not expect costs that key, never the status — which would
        // make the router read as unreachable.
        let odd = try auth(#"{"configured":false,"remote_bind":false,"from_loopback":true,"mode":7,"password_pinned":"yes","public_url":{}}"#)
        XCTAssertNil(odd.mode)
        XCTAssertNil(odd.passwordPinned)
        XCTAssertNil(odd.publicURL)
        XCTAssertFalse(odd.isHosted)
        // The keys every router sends are exactly as strict as before.
        XCTAssertThrowsError(try auth(#"{"configured":true,"mode":"server","password_pinned":true,"public_url":null}"#))
    }

    func testBasicAuthorizationHeaderCarriesThePassword() throws {
        let header = RouterClient.authorization(password: "hunter22")
        XCTAssertTrue(header.hasPrefix("Basic "))
        let decoded = try XCTUnwrap(Data(base64Encoded: String(header.dropFirst(6))))
        XCTAssertEqual(String(decoding: decoded, as: UTF8.self), "app:hunter22")
    }

    // MARK: - Session liveness: the keys a current router adds, and the rows an older one sends

    private func row(_ json: String) throws -> LiveSession {
        try JSONDecoder().decode(LiveSession.self, from: Data(json.utf8))
    }

    private func status(_ json: String) throws -> LiveStatus {
        try JSONDecoder().decode(LiveStatus.self, from: Data(json.utf8))
    }

    /// A current router's row: every key it added decodes, and the legacy ones still say
    /// what older builds expect.
    func testASessionRowCarriesTheRoutersActivity() throws {
        let s = try row(#"""
        {"session_id":"5e6f7081-0000","cwd":"/code/agent-router","project":"-code-agent-router","git_branch":"dev",
         "title":"Session liveness","created_ms":1790000000000,"last_activity_ms":1790000100000,"active":true,
         "state":"ask","archived":false,"wrapped":true,"bytes":123456,
         "activity":"waiting","open":true,"detail":"Bash wants to run","since_ms":1790000090000,
         "waiting_on":{"kind":"permission","tool":"Bash","request_id":"req-1","tool_use_id":"toolu_1","summary":"git status"},
         "tone":"attention","source":"registry","as_of_ms":1790000120000}
        """#)
        XCTAssertEqual(s.activity, .waiting)
        XCTAssertEqual(s.isOpen, true)
        XCTAssertEqual(s.detail, "Bash wants to run")
        XCTAssertEqual(s.waitingOn?.kind, "permission")
        XCTAssertEqual(s.waitingOn?.tool, "Bash")
        XCTAssertEqual(s.waitingOn?.requestId, "req-1")
        XCTAssertEqual(s.waitingOn?.toolUseId, "toolu_1")
        XCTAssertEqual(s.waitingOn?.summary, "git status")
        XCTAssertEqual(s.tone, .attention)
        XCTAssertEqual(s.source, "registry")
        XCTAssertEqual(s.asOfMs, 1_790_000_120_000)
        XCTAssertEqual(s.elapsed, 30, "measured against the router's clock, not the phone's")
        XCTAssertTrue(s.isLive)
        XCTAssertTrue(s.needsYou)
        XCTAssertFalse(s.isWorking)
        XCTAssertEqual(s.state, "ask", "the legacy word is still there for the builds already installed")
        XCTAssertEqual(s.look, StateLook(color: .orange, hollow: false, halo: true, label: "waiting for you"))

        let closed = try row(#"""
        {"session_id":"738b7b63","project":"-p","last_activity_ms":1790000000000,"active":false,"state":"idle",
         "archived":false,"wrapped":false,"bytes":1,"activity":"closed","open":false,"detail":"Session ended mid-turn",
         "ended_ms":1789990000000,"ended_how":"unknown","mid_turn":true,"tone":"closed","source":"registry"}
        """#)
        XCTAssertEqual(closed.activity, .closed)
        XCTAssertEqual(closed.isOpen, false)
        XCTAssertEqual(closed.endedMs, 1_789_990_000_000)
        XCTAssertEqual(closed.endedHow, "unknown")
        XCTAssertEqual(closed.midTurn, true)
        XCTAssertFalse(closed.isLive)
        XCTAssertEqual(closed.look, StateLook(color: .gray, hollow: true, halo: false, label: "ended"), "a hollow ring, never a plain idle dot")

        let busy = try row(#"""
        {"session_id":"bg","project":"-p","last_activity_ms":1790000000000,"active":true,"state":"working",
         "archived":false,"wrapped":true,"bytes":1,"activity":"background","open":true,
         "detail":"2 background agents running","since_ms":1789999000000,
         "background":{"agents":2,"workflows":0,"shells":1},"tone":"busy"}
        """#)
        XCTAssertEqual(busy.background, BackgroundWork(agents: 2, workflows: 0, shells: 1))
        XCTAssertTrue(busy.isWorking)
        XCTAssertEqual(busy.look.color, .green)
    }

    /// An older router sends none of the new keys: the row decodes as it always did, and
    /// is drawn exactly as it always was.
    func testALegacyRowDecodesAndDrawsAsBefore() throws {
        let s = try row(#"{"session_id":"aaaa","cwd":null,"project":"-p","git_branch":null,"title":null,"created_ms":null,"last_activity_ms":1790000000000,"active":true,"state":"working","archived":false,"wrapped":false,"bytes":10}"#)
        XCTAssertNil(s.activity)
        XCTAssertNil(s.isOpen)
        XCTAssertNil(s.detail)
        XCTAssertNil(s.sinceMs)
        XCTAssertNil(s.endedMs)
        XCTAssertNil(s.waitingOn)
        XCTAssertNil(s.background)
        XCTAssertNil(s.tone)
        XCTAssertNil(s.asOfMs)
        XCTAssertNil(s.elapsed)
        XCTAssertTrue(s.isLive, "an older router's `active` still counts")
        XCTAssertFalse(s.needsYou)
        XCTAssertEqual(s.look, StateLook(color: .green, hollow: false, halo: true, label: "working"))
        for (state, color) in [("ask", StateColor.ColorName.orange), ("stalled", .yellow), ("failed", .red), ("idle", .gray)] {
            let r = try row(#"{"session_id":"b","project":"-p","last_activity_ms":1,"active":false,"state":"\#(state)","archived":false,"wrapped":false,"bytes":1}"#)
            XCTAssertEqual(r.look, StateLook(color: color, hollow: false, halo: false, label: state), state)
        }
        // The legacy keys are exactly as strict as before: a row without them is refused.
        XCTAssertThrowsError(try row(#"{"session_id":"c","project":"-p","active":false,"state":"idle","archived":false,"wrapped":false,"bytes":1}"#))
    }

    /// A key added later that arrives malformed must not cost the list: on an installed
    /// phone one row that fails to decode fails them all.
    func testAMalformedNewKeyReadsAsAbsent() throws {
        let rows = try JSONDecoder().decode([LiveSession].self, from: Data(#"""
        [{"session_id":"a","project":"-p","last_activity_ms":1,"active":false,"state":"idle","archived":false,"wrapped":false,"bytes":1,
          "activity":7,"open":"yes","detail":null,"since_ms":"soon","ended_ms":1.5,"mid_turn":"no","waiting_on":"x",
          "background":[1],"tone":null,"as_of_ms":{}},
         {"session_id":"b","project":"-p","last_activity_ms":2,"active":false,"state":"idle","archived":false,"wrapped":false,"bytes":1}]
        """#.utf8))
        XCTAssertEqual(rows.map(\.sessionId), ["a", "b"])
        XCTAssertNil(rows[0].activity)
        XCTAssertNil(rows[0].isOpen)
        XCTAssertNil(rows[0].sinceMs)
        XCTAssertNil(rows[0].endedMs)
        XCTAssertNil(rows[0].midTurn)
        XCTAssertNil(rows[0].waitingOn)
        XCTAssertNil(rows[0].background)
        XCTAssertNil(rows[0].tone)
        XCTAssertNil(rows[0].asOfMs)
        let st = try status(#"{"state":"idle","detail":"Waiting for you","quiet_ms":5,"activity":["x"],"waiting_on":3,"since_ms":"x"}"#)
        XCTAssertNil(st.activity)
        XCTAssertNil(st.waitingOn)
        XCTAssertEqual(st.detail, "Waiting for you")
    }

    /// A hosted router names the machine each session runs on; a local router never does,
    /// and its rows read exactly as before.
    func testASessionRowNamesItsMachineOnAHostedRouter() throws {
        let hosted = try row(#"""
        {"session_id":"5e6f7081-0000","cwd":"/code/agent-router","project":"-code-agent-router","git_branch":"dev",
         "title":"Hosting","created_ms":1790000000000,"last_activity_ms":1790000100000,"active":true,"state":"working",
         "archived":false,"wrapped":true,"bytes":10,"activity":"generating","tone":"busy","as_of_ms":1790000120000,
         "machine":{"id":"5f0c2a8e-3b1d-4c6f-9e7a-2d4b6c8e0f13","label":"studio"}}
        """#)
        XCTAssertEqual(hosted.machine, SessionMachine(id: "5f0c2a8e-3b1d-4c6f-9e7a-2d4b6c8e0f13", label: "studio"))
        XCTAssertEqual(hosted.machineName, "studio")
        XCTAssertEqual(hosted.activity, .generating, "the rest of the row reads as it always did")
        XCTAssertEqual(hosted.projectName, "agent-router")

        let local = try row(#"{"session_id":"aaaa","cwd":null,"project":"-p","git_branch":null,"title":null,"created_ms":null,"last_activity_ms":1790000000000,"active":true,"state":"working","archived":false,"wrapped":false,"bytes":10}"#)
        XCTAssertNil(local.machine)
        XCTAssertNil(local.machineName)

        // A blank label is named by the start of the machine's id.
        let blank = try row(#"{"session_id":"b","project":"-p","last_activity_ms":1,"active":false,"state":"idle","archived":false,"wrapped":false,"bytes":1,"machine":{"id":"0a1b2c3d-4e5f","label":"  "}}"#)
        XCTAssertEqual(blank.machineName, "0a1b2c3d")
        // A machine that is not the object this build expects reads as none, and never costs
        // the list.
        let rows = try JSONDecoder().decode([LiveSession].self, from: Data(#"""
        [{"session_id":"c","project":"-p","last_activity_ms":1,"active":false,"state":"idle","archived":false,"wrapped":false,"bytes":1,"machine":"studio"},
         {"session_id":"d","project":"-p","last_activity_ms":1,"active":false,"state":"idle","archived":false,"wrapped":false,"bytes":1,"machine":{"label":7}},
         {"session_id":"e","project":"-p","last_activity_ms":1,"active":false,"state":"idle","archived":false,"wrapped":false,"bytes":1,"machine":null}]
        """#.utf8))
        XCTAssertEqual(rows.map(\.sessionId), ["c", "d", "e"])
        XCTAssertEqual(rows.map(\.machine), [nil, nil, nil])
    }

    /// A word this build has never heard is kept and drawn neutral gray — never guessed
    /// at from the legacy state, never a crash, and never a Stop button.
    func testAnUnknownActivityIsNeutral() throws {
        let s = try row(#"{"session_id":"a","project":"-p","last_activity_ms":1,"active":true,"state":"working","archived":false,"wrapped":true,"bytes":1,"activity":"compacting"}"#)
        XCTAssertEqual(s.activity, .unknown("compacting"))
        XCTAssertEqual(s.look.color, .gray)
        XCTAssertFalse(s.look.hollow)
        XCTAssertEqual(s.look.label, "compacting")
        XCTAssertTrue(s.isLive, "the router's own `active` still decides whether it is live")
        XCTAssertFalse(s.isWorking)
        // The router's tone, when it sends one this build knows, is its explicit choice.
        let toned = try row(#"{"session_id":"a","project":"-p","last_activity_ms":1,"active":true,"state":"working","archived":false,"wrapped":true,"bytes":1,"activity":"compacting","tone":"busy"}"#)
        XCTAssertEqual(toned.look.color, .green)
        // An unknown tone falls back to the activity's own.
        let odd = try row(#"{"session_id":"a","project":"-p","last_activity_ms":1,"active":true,"state":"ask","archived":false,"wrapped":true,"bytes":1,"activity":"waiting","tone":"violet"}"#)
        XCTAssertEqual(odd.tone, .unknown("violet"))
        XCTAssertEqual(odd.look.color, .orange)

        let st = try status(#"{"state":"working","detail":"Compacting","quiet_ms":10,"activity":"compacting"}"#)
        XCTAssertFalse(st.isBusy, "no Stop for an activity this build cannot vouch for")
        XCTAssertEqual(st.look.color, .gray)
    }

    /// The stream's status event in both shapes, and what each activity lets the header do.
    func testTheStatusEventInBothShapes() throws {
        let old = try status(#"{"state":"working","detail":"Working","quiet_ms":10}"#)
        XCTAssertNil(old.activity)
        XCTAssertTrue(old.isBusy, "an older router's `working` still offers Stop")
        XCTAssertTrue(old.isActive)
        XCTAssertEqual(old.look, StateLook(color: .green, hollow: false, halo: true, label: "working"))
        XCTAssertFalse(try status(#"{"state":"idle","detail":"Waiting for you","quiet_ms":10}"#).isBusy)
        XCTAssertTrue(try status(#"{"state":"ask","detail":"Question","quiet_ms":10}"#).needsYou)

        let thinking = try status(#"{"state":"working","detail":"Thinking","quiet_ms":110000,"queued":1,"activity":"generating","open":true,"since_ms":1790000000000,"as_of_ms":1790000110000,"tone":"busy"}"#)
        XCTAssertEqual(thinking.activity, .generating)
        XCTAssertEqual(thinking.isOpen, true)
        XCTAssertEqual(thinking.sinceMs, 1_790_000_000_000)
        XCTAssertEqual(thinking.asOfMs, 1_790_000_110_000)
        XCTAssertEqual(thinking.queued, 1)
        XCTAssertTrue(thinking.isBusy)

        let tool = try status(#"{"state":"tool","detail":"Running Bash","quiet_ms":10,"activity":"tool","tone":"busy"}"#)
        XCTAssertTrue(tool.isBusy)
        let stalled = try status(#"{"state":"working","detail":"No progress","quiet_ms":700000,"activity":"stalled","tone":"warn"}"#)
        XCTAssertTrue(stalled.isBusy, "a stalled turn is still a turn Stop can end")
        XCTAssertEqual(stalled.look.color, .yellow)

        let background = try status(#"{"state":"working","detail":"2 background agents running","quiet_ms":5000,"queued":0,"activity":"background","open":true,"background":{"agents":2,"workflows":0,"shells":0},"tone":"busy"}"#)
        XCTAssertFalse(background.isBusy, "no Stop for background work: an interrupt could discard its results")
        XCTAssertTrue(background.isActive)
        XCTAssertEqual(background.background?.agents, 2)
        XCTAssertEqual(background.look, StateLook(color: .green, hollow: false, halo: true, label: "working in the background"))

        let waiting = try status(#"{"state":"ask","detail":"Waiting for your answer","quiet_ms":61000,"activity":"waiting","waiting_on":{"kind":"question","tool":"AskUserQuestion","request_id":"r-9"},"tone":"attention"}"#)
        XCTAssertTrue(waiting.needsYou)
        XCTAssertFalse(waiting.isBusy)
        XCTAssertEqual(waiting.waitingOn?.requestId, "r-9")
        XCTAssertEqual(waiting.look.color, .orange)

        let failed = try status(#"{"state":"failed","detail":"API error: rate limited","quiet_ms":10,"activity":"failed","tone":"error"}"#)
        XCTAssertFalse(failed.isBusy)
        XCTAssertFalse(failed.isActive)
        XCTAssertEqual(failed.look.color, .red)

        let closed = try status(#"{"state":"idle","detail":"Session ended","quiet_ms":3600000,"queued":0,"activity":"closed","open":false,"ended_ms":1789996400000,"tone":"closed"}"#)
        XCTAssertFalse(closed.isBusy)
        XCTAssertFalse(closed.isActive)
        XCTAssertEqual(closed.endedMs, 1_789_996_400_000)
        XCTAssertEqual(closed.look, StateLook(color: .gray, hollow: true, halo: false, label: "ended"))
    }

    /// The prompt says how many are waiting, and an answer names the one it settles.
    func testThePromptCountAndTheAnswerNamesItsPrompt() throws {
        let p = try JSONDecoder().decode(LivePrompt.self, from: Data(#"{"pending":true,"request_id":"r1","tool":"Bash","question":false,"display_name":"Bash","description":"git status","reason":null,"input":{},"suggestions":[{"index":0,"label":"Always allow Bash(git status:*) for the rest of this session"}],"count":3}"#.utf8))
        XCTAssertEqual(p.count, 3)
        XCTAssertTrue(p.isPermission)
        XCTAssertFalse(p.isQuestion)
        let q = try JSONDecoder().decode(LivePrompt.self, from: Data(#"{"pending":true,"request_id":"r2","tool":"AskUserQuestion","question":true,"suggestions":[]}"#.utf8))
        XCTAssertNil(q.count, "an older router reports only the one")
        XCTAssertTrue(q.isQuestion)
        XCTAssertFalse(q.isPermission)
        XCTAssertFalse(try JSONDecoder().decode(LivePrompt.self, from: Data(#"{"pending":false}"#.utf8)).isQuestion)

        func keys(_ body: AnswerBody) throws -> [String: Any] {
            try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(body)) as? [String: Any])
        }
        let allow = try keys(AnswerBody(session: "s", allow: true, suggestion: 0, requestId: "r1"))
        XCTAssertEqual(allow["request_id"] as? String, "r1")
        XCTAssertNil(allow["requestId"])
        XCTAssertEqual(allow["allow"] as? Bool, true)
        let legacy = try keys(AnswerBody(session: "s", answers: ["Which?": "B"]))
        XCTAssertNil(legacy["request_id"], "left out when unknown, so an older router sees what it always saw")
        XCTAssertEqual(Set(legacy.keys), ["session", "answers"])
    }

    /// A waiting question lists its questions in the tool's input — what tells a form drawn
    /// for an earlier question from the one waiting now. Any other prompt's input is the
    /// tool's own business, of any shape, and one this build does not expect costs the
    /// list, never the prompt: the permission card must keep coming.
    func testAPromptSaysWhichQuestionsItPuts() throws {
        func prompt(_ json: String) throws -> LivePrompt { try JSONDecoder().decode(LivePrompt.self, from: Data(json.utf8)) }
        let q = try prompt(#"{"pending":true,"request_id":"r2","tool":"AskUserQuestion","question":true,"display_name":null,"description":null,"reason":null,"input":{"questions":[{"question":"Which database?","header":"DB","multiSelect":false,"options":[{"label":"Postgres","description":""}]},{"question":"And the cache?"}]},"suggestions":[],"count":1}"#)
        XCTAssertEqual(q.questions, ["Which database?", "And the cache?"])
        XCTAssertTrue(q.asks(["Which database?", "And the cache?"]))
        XCTAssertFalse(q.asks(["And the cache?", "Which database?"]), "in the same order")
        XCTAssertFalse(q.asks(["Which database?"]), "all of them")
        XCTAssertFalse(q.asks(["Which framework?", "And the cache?"]))

        let bare = try prompt(#"{"pending":true,"request_id":"r3","tool":"AskUserQuestion","question":true}"#)
        XCTAssertNil(bare.questions)
        XCTAssertTrue(bare.asks(["Anything?"]), "a prompt that lists nothing is taken at its word")

        let bash = try prompt(#"{"pending":true,"request_id":"p1","tool":"Bash","question":false,"input":{"command":"ls","questions":"not a list"},"suggestions":[{"index":0,"label":"Always allow"}],"count":2}"#)
        XCTAssertNil(bash.questions)
        XCTAssertTrue(bash.isPermission)
        XCTAssertEqual(bash.suggestions?.map(\.label), ["Always allow"])
        XCTAssertEqual(bash.count, 2)
        XCTAssertNil(try prompt(#"{"pending":true,"request_id":"p2","tool":"Write","question":false,"input":null}"#).questions)

        // An entry with no text stays in the list, so it can match no form.
        let odd = try prompt(#"{"pending":true,"question":true,"input":{"questions":[{"question":"A"},7,{"question":3}]}}"#)
        XCTAssertEqual(odd.questions, ["A", nil, nil])
        XCTAssertFalse(odd.asks(["A"]))
        XCTAssertFalse(odd.asks(["A", "B", "C"]))
        XCTAssertNil(try prompt(#"{"pending":false,"count":0}"#).questions)
    }
}
