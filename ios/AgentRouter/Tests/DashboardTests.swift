import XCTest
@testable import AgentRouter

/// The dashboard's payloads decode, and the buckets fill in the way the web view fills them.
final class DashboardTests: XCTestCase {
    private let summary = #"""
    {"totals":{"requests":12,"prompt_tokens":1200000,"cost":3.5,"input_tokens":1000,"output_tokens":9000,"thinking_tokens":2000,"cache_read_tokens":1150000,"cache_write_5m_tokens":40000,"cache_write_1h_tokens":9000,"cache_write_tokens":49000,"errors":1,"subagent_requests":3,"sessions":2,"projects":1,"first_seen":1789200000000,"last_seen":1789203600000,"unpriced":0,"estimated":0,"by_provider":[{"provider":"anthropic","requests":12,"prompt_tokens":1200000,"cost":3.5,"unpriced":0,"estimated":0,"output_tokens":9000,"cache_read_tokens":1150000,"cache_write_tokens":49000}]},
     "quota":{"rl_status":"allowed","rl_5h_utilization":0.1,"rl_7d_utilization":0.2,"rl_5h_reset":1,"rl_7d_reset":2,"started_at":3},
     "account":{"id":"a","label":"A","email":null,"org_name":null,"subscription_type":null},
     "context":[{"component":"tools","est_tokens":500000,"chars":2000000,"est_cost_once":2.5,"requests":12},{"component":"history","est_tokens":300000,"chars":1200000,"est_cost_once":1.5,"requests":12}],
     "models":[{"model":"claude-opus-5","requests":10,"prompt_tokens":1100000,"cost":3.2,"output_tokens":8000,"thinking_tokens":2000,"cache_read_tokens":1050000,"errors":1,"subagent_requests":3},{"model":"claude-haiku-4-5","requests":2,"prompt_tokens":100000,"cost":0.3,"output_tokens":1000,"thinking_tokens":0,"cache_read_tokens":100000,"errors":0,"subagent_requests":0}],
     "threads":[{"thread":"main","requests":9,"prompt_tokens":900000,"cost":2.9,"output_tokens":7000,"cache_read_tokens":860000,"agents":0},{"thread":"subagent","requests":3,"prompt_tokens":300000,"cost":0.6,"output_tokens":2000,"cache_read_tokens":290000,"agents":2}],
     "latency":{"ttfb_p50":812,"ttfb_p95":3200,"duration_p50":4000,"duration_p95":20000,"duration_max":58000},
     "stop_reasons":[{"stop_reason":"end_turn","requests":8},{"stop_reason":"tool_use","requests":4}],
     "by_account":[{"id":"a","label":"A","active":1,"requests":12,"prompt_tokens":1200000,"cost":3.5,"output_tokens":9000,"sessions":2,"last_seen":1789203600000}],
     "timeline":{"start":1789200000000,"end":1789203600000,"bucket_ms":900000,"rows":[{"t":1789200000000,"model":"claude-opus-5","requests":5,"prompt_tokens":500000,"cost":1.5,"output_tokens":4000,"cache_read_tokens":480000,"subagent_tokens":0,"errors":0},{"t":1789200000000,"model":"claude-haiku-4-5","requests":2,"prompt_tokens":100000,"cost":0.3,"output_tokens":1000,"cache_read_tokens":100000,"subagent_tokens":0,"errors":0},{"t":1789202700000,"model":"claude-opus-5","requests":5,"prompt_tokens":600000,"cost":1.7,"output_tokens":4000,"cache_read_tokens":570000,"subagent_tokens":300000,"errors":1}]},
     "providers":[{"provider":"anthropic","account":{"id":"a","label":"A","email":null,"org_name":null,"plan":null},"windows":{"five_hour":{"utilization":0.1,"resets_at":1789210000},"seven_day":{"utilization":0.21,"resets_at":1789700000}},"status":"allowed","at":1789203600000,"history":[{"t":1789200000000,"five_hour":0.05,"seven_day":0.2},{"t":1789202700000,"five_hour":0.1,"seven_day":0.21}]}]}
    """#

    func testSummaryDecodesAndBucketsFillTheRange() throws {
        let s = try JSONDecoder().decode(Summary.self, from: Data(summary.utf8))
        XCTAssertEqual(s.totals.requests, 12)
        XCTAssertEqual(s.context.map(\.component), ["tools", "history"])
        XCTAssertEqual(s.models.first?.model, "claude-opus-5")
        XCTAssertEqual(s.threads.count, 2)
        XCTAssertEqual(s.latency?.ttfbP50, 812)
        XCTAssertEqual(s.stopReasons.first?.stopReason, "end_turn")
        XCTAssertEqual(s.byAccount.first?.active, 1)
        XCTAssertEqual(s.providers.first?.history.count, 2)

        let b = Bucket.build(s.timeline, tz: TimeZone(secondsFromGMT: 0)!)
        // One hour at 15 minutes is five slots inclusive; the empty ones are still there.
        XCTAssertEqual(b.count, 5)
        XCTAssertEqual(b[0].prompt, 600000, "two models in one slot add up")
        XCTAssertEqual(b[0].families["opus"], 500000)
        XCTAssertEqual(b[0].families["haiku"], 100000)
        XCTAssertEqual(b[1].prompt, 0, "a quiet quarter hour is drawn as quiet")
        XCTAssertEqual(b[3].sub, 300000)
        XCTAssertEqual(b[3].errors, 1)
    }

    /// The search reply, as the router spells it. Counts arrive as integers and are
    /// read as Doubles; the snippet keeps its markers for the view to split on.
    func testSearchResultsDecode() throws {
        let json = #"""
        {"query":"body_cap","expression":"body_cap",
         "sessions":[{"session_id":"2b3c4d5e-0000-4000-8000-00000000000b","project":"/code/agent-router",
                      "session_label":null,"git_branch":"dev","started_at":1790000000000,
                      "blocks":3,"hits":4,"first_at":1790000000000,"last_at":1790000009000}],
         "hits":[{"block_id":41,"kind":"tool_use","chars":120,
                  "session_id":"2b3c4d5e-0000-4000-8000-00000000000b","role":"assistant","ordinal":7,
                  "request_id":48761,"at":1790000009000,"project":"/code/agent-router",
                  "session_label":null,"snippet":"why is \#u{2039}body_cap\#u{203A} 64 KB","rank":-8.04}]}
        """#
        let r = try JSONDecoder().decode(SearchResults.self, from: Data(json.utf8))
        XCTAssertEqual(r.sessions.first?.shortId, "2b3c4d5e")
        XCTAssertEqual(r.sessions.first?.projectName, "agent-router")
        XCTAssertEqual(r.sessions.first?.hits, 4)

        let hit = try XCTUnwrap(r.hits.first)
        XCTAssertEqual(hit.kindName, "tool call", "the wire's vocabulary is not the reader's")
        XCTAssertEqual(hit.id, "2b3c4d5e-0000-4000-8000-00000000000b-41-7",
                       "a block can appear at several places, so the block alone is not an identity")
        XCTAssertTrue(hit.snippet.contains("\u{2039}"), "the markers survive for the view to split on")
    }

    /// A nullable project and a missing label must not fail the whole reply: the join
    /// behind them is a LEFT JOIN, and a request whose transcript never arrived has
    /// neither.
    func testSearchToleratesMissingProjectAndLabel() throws {
        let json = #"{"query":"x","expression":"x","sessions":[],"hits":[{"block_id":1,"kind":"text","chars":3,"session_id":"s","role":"user","ordinal":0,"request_id":2,"at":1790000000000,"project":null,"session_label":null,"snippet":"x","rank":-1.0}]}"#
        let r = try JSONDecoder().decode(SearchResults.self, from: Data(json.utf8))
        XCTAssertEqual(r.hits.first?.projectName, "—")
        XCTAssertTrue(r.sessions.isEmpty)
    }

    func testListsDecode() throws {
        let projects = try JSONDecoder().decode([ProjectRow].self, from: Data(#"[{"project":"/code/project-b","sessions":3,"requests":40,"prompt_tokens":5000000,"cost":12.5,"output_tokens":40000,"cache_read_tokens":4800000,"subagent_tokens":100000,"errors":0,"models":"claude-opus-5,claude-haiku-4-5","first_seen":1,"last_seen":2}]"#.utf8))
        XCTAssertEqual(projects[0].name, "project-b")
        let sessions = try JSONDecoder().decode([ProxySessionRow].self, from: Data(#"[{"id":"1a2b3c4d-0000-4000-8000-00000000000a","project":"/code/project-b","git_branch":"main","entrypoint":"claude-cli/2.1.269 (external, claude-vscode)","started_at":1,"last_seen_at":2,"requests":4,"prompt_tokens":1000,"cost":0.1,"output_tokens":10,"cache_read_tokens":900,"subagent_tokens":0,"errors":0,"subagents":0,"models":"claude-opus-5","first_request_at":1,"last_request_at":2}]"#.utf8))
        XCTAssertEqual(sessions[0].client, "VS Code")
        XCTAssertEqual(sessions[0].shortId, "1a2b3c4d")
        let requests = try JSONDecoder().decode([RequestRow].self, from: Data(#"[{"id":7,"request_id":"req_1","session_id":"s","project":"/code/project-b","model":"claude-opus-5","status":200,"ttfb_ms":812,"duration_ms":4000,"stop_reason":"tool_use","is_sidechain":0,"agent_id":null,"started_at":1789200000000,"error":null,"prompt_tokens":100000,"input_tokens":10,"output_tokens":500,"thinking_tokens":100,"cost":0.06,"cache_read_tokens":99000,"cache_creation_5m_tokens":990,"cache_creation_1h_tokens":0,"cache_write_tokens":990,"tools":"Bash,Read","context":"system:1000,tools:20000,history:79000"}]"#.utf8))
        XCTAssertEqual(requests[0].toolList, ["Bash", "Read"])
        XCTAssertEqual(requests[0].contextSplit.map(\.0), ["system", "tools", "history"])
        XCTAssertFalse(requests[0].isError)
        let tools = try JSONDecoder().decode(ToolsPayload.self, from: Data(#"{"produced":[{"name":"Bash","results":30,"chars":120000,"est_tokens":30000,"max_tokens":8000,"est_cost_once":0.15,"errors":2}],"called":[{"name":"Bash","calls":31,"requests":20,"subagent_calls":4}]}"#.utf8))
        XCTAssertEqual(tools.called[0].subagentCalls, 4)
        XCTAssertEqual(tools.produced[0].maxTokens, 8000)
    }

    func testSettingsPayloadsDecode() throws {
        let setup = try JSONDecoder().decode(SetupStatus.self, from: Data(#"{"service":{"registered":true,"supported":true,"platform":"macos"},"url":"http://127.0.0.1:8787","global_settings_path":"/Users/x/.claude/settings.json","managed":[{"path":"/Users/x/.claude/settings.json","scope":"global","value":"http://127.0.0.1:8787","previous":null,"backup_path":null,"applied_at":1}]}"#.utf8))
        XCTAssertTrue(setup.globalRouted)
        XCTAssertTrue(setup.service.registered)
        let login = try JSONDecoder().decode(LoginStatus.self, from: Data(#"{"phase":"awaiting_sign_in","url":"https://claude.ai/oauth/x","label":"work","account_id":null,"email":null,"error":null,"tail":"..."}"#.utf8))
        XCTAssertEqual(login.phase, "awaiting_sign_in")
        let full = try JSONDecoder().decode([Account].self, from: Data(#"[{"id":"p","label":"Personal","source":"keychain","profile_dir":null,"email":"me@x.test","org_name":null,"subscription_type":"max","active":1,"created_at":1,"usage_key":1,"has_token":true,"ready":true,"detail":"sk-ant-oat01 (subscription OAuth)","expires_at":1789300000000}]"#.utf8))
        XCTAssertTrue(full[0].hasUsageKey)
        XCTAssertNotNil(full[0].expiry)
        let reply = try JSONDecoder().decode(OkReply.self, from: Data(#"{"ok":false,"reason":"Not a known extension binary."}"#.utf8))
        XCTAssertEqual(reply.problem, "Not a known extension binary.")
    }

    func testPushPayloadsAndTapsDecode() throws {
        let st = try JSONDecoder().decode(PushStatus.self, from: Data(#"{"configured":true,"team_id":"ABCDE12345","key_id":"ABC123DEF4","bundle_id":"ca.whnet.agent-router","key_file":"imported from AuthKey_ABC123DEF4.p8","devices":[{"token_tail":"a1b2c3","name":"My iPhone","environment":"development","router_ref":"2A7C2F7E-0000-4000-8000-000000000001","registered_at":1,"last_seen_at":2,"last_result":"delivered"}],"last_error":null,"last_sent":{"at":3,"what":"project-a: Bash wants to run"}}"#.utf8))
        XCTAssertEqual(st.devices.first?.tokenTail, "a1b2c3")
        XCTAssertEqual(st.bundleId, "ca.whnet.agent-router")
        XCTAssertEqual(st.keyFile, "imported from AuthKey_ABC123DEF4.p8")
        let body = try JSONSerialization.jsonObject(with: JSONEncoder().encode(PushRegisterBody(token: "ab", name: "p", environment: "development", routerRef: "r", teamId: "ABCDE12345", bundleId: "ca.whnet.agent-router"))) as? [String: Any]
        XCTAssertEqual(body?["team_id"] as? String, "ABCDE12345")
        XCTAssertEqual(body?["bundle_id"] as? String, "ca.whnet.agent-router")
        XCTAssertEqual(st.lastSent?.what, "project-a: Bash wants to run")
        let open = PushRegistrar.destination(of: ["router_ref": "2A7C2F7E-0000-4000-8000-000000000001", "session": "abc-1", "aps": ["alert": "x"]])
        XCTAssertEqual(open?.sessionId, "abc-1")
        XCTAssertEqual(open?.routerId.uuidString, "2A7C2F7E-0000-4000-8000-000000000001")
        XCTAssertNil(PushRegistrar.destination(of: ["session": "abc-1"]), "no router, nowhere to go")
        XCTAssertNil(PushRegistrar.destination(of: ["router_ref": "not-a-uuid", "session": "abc-1"]))
    }

    func testShimStatusCarriesTheAutoSwitchAndItsLog() throws {
        let json = #"{"available":true,"installed":1,"targets":[{"extension":"anthropic.claude-code-2.1.270","path":"/x/claude","state":"installed"}],"auto":true,"auto_log":[{"at":1700000000000,"what":"automatic wrapper: installed into 2.1.270"}]}"#
        let s = try JSONDecoder().decode(ShimStatus.self, from: Data(json.utf8))
        XCTAssertEqual(s.auto, true)
        XCTAssertEqual(s.autoLog?.first?.what, "automatic wrapper: installed into 2.1.270")
        XCTAssertNil(s.newestUnwrapped, "the newest extension is wrapped")
        // An older router: no switch, no log, still decodes.
        let old = try JSONDecoder().decode(ShimStatus.self, from: Data(#"{"available":true,"installed":0,"targets":[{"extension":"e","path":"/p","state":"not_installed"}]}"#.utf8))
        XCTAssertNil(old.auto)
        XCTAssertEqual(old.newestUnwrapped?.ext, "e")
    }

    func testRangesAndFormats() {
        XCTAssertNil(UsageRange.all.sinceMs)
        XCTAssertEqual(UsageRange.d7.sinceMs, 7 * 86_400_000)
        XCTAssertEqual(Fmt.shortModel("claude-opus-5-20260115"), "opus-5-20260115")
        XCTAssertEqual(Fmt.bucketName(900_000), "15 minutes")
        XCTAssertEqual(Fmt.bucketName(3_600_000), "hour")
        XCTAssertEqual(Fmt.ms(812), "812 ms")
        XCTAssertEqual(Fmt.ms(3200), "3.2 s")
        XCTAssertEqual(Bucket.family("claude-sonnet-5"), "sonnet")
        XCTAssertEqual(Bucket.family("gpt-x"), "other")
        XCTAssertEqual(Fmt.share(27.0 / 8474.0), "0.3%")
        XCTAssertEqual(Fmt.share(0.99), "99%")
        XCTAssertEqual(Fmt.share(0), "0%")
    }
}
