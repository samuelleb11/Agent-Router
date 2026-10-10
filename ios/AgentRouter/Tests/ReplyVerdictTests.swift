import XCTest
@testable import AgentRouter

/// A model, mode or effort change, or a side question, that the router refuses is shown as a
/// refusal, in the router's words, whatever status came with it.
///
/// The router refuses each with an error status and `{ok: false, reason}` (src/live.rs,
/// `bad`), and no router has ever answered one with `ok: false` in a 2xx. One must never read
/// as success all the same. These replies were read by their status alone, so a 200 saying `ok: false`
/// would have said "Model set to opus", "Now plan.", "Effort set to high" or "Answered in
/// 0s", and put "(no answer)" in the feed. They are read as the live replies are now
/// (`Verdict`): the router's words, and nothing done.
///
/// Each call goes through the session screen's own feed and the app's own client to the
/// stand-in router `RefusalTests` uses, so nothing leaves the process. The refusals are the
/// router's, word for word (`live::set_model`, `set_mode`, `set_effort` and `aside`), each in a
/// 200 and in the status a current router sends it with.
final class ReplyVerdictTests: XCTestCase {
    /// An open session's feed, calling the stand-in.
    @MainActor
    private func feed() throws -> LiveFeed {
        StandInRouter.reset()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [StandInRouter.self]
        let router = RouterState(
            router: Router(name: "Stand-in", host: "router.invalid", port: 443, scheme: .https),
            password: nil,
            calls: URLSession(configuration: config)
        )
        let json = #"{"session_id":"s-1","cwd":"/Users/sample/code/p","project":"-Users-sample-code-p","git_branch":null,"title":null,"created_ms":0,"last_activity_ms":0,"active":true,"state":"working","archived":false,"wrapped":true,"bytes":10}"#
        let session = try JSONDecoder().decode(LiveSession.self, from: Data(json.utf8))
        return LiveFeed(session: session, client: router.liveClient)
    }

    private func refusal(_ words: String) -> String { #"{"ok":false,"reason":"\#(words)"}"# }

    /// The CLI refused the model without saying why: the router's words for that.
    @MainActor
    func testAModelChangeTheRouterRefusesWithOkFalseIsShownAsARefusal() async throws {
        let feed = try feed()
        let words = "the session refused the model"
        for status in [200, 422] {
            StandInRouter.answer("/api/live/model", status, refusal(words))
            await feed.setModel("opus")
            XCTAssertEqual(feed.notice, LiveFeed.Notice(text: words, ok: false), "\(status) is a refusal, not \"Model set to opus\"")
        }
        XCTAssertEqual(StandInRouter.posts, ["/api/live/model", "/api/live/model"])
    }

    /// Plan mode refused, as the CLI may refuse a mode: the router's words for that.
    @MainActor
    func testAModeChangeTheRouterRefusesWithOkFalseIsShownAsARefusal() async throws {
        let feed = try feed()
        let words = "the session refused that mode"
        for status in [200, 422] {
            StandInRouter.answer("/api/live/mode", status, refusal(words))
            await feed.setMode("plan")
            XCTAssertEqual(feed.notice, LiveFeed.Notice(text: words, ok: false), "\(status) is a refusal, not \"Now plan.\"")
        }
        XCTAssertEqual(StandInRouter.posts, ["/api/live/mode", "/api/live/mode"])
    }

    /// A session that is not under the wrapper cannot take an effort level.
    @MainActor
    func testAnEffortChangeTheRouterRefusesWithOkFalseIsShownAsARefusal() async throws {
        let feed = try feed()
        let words = "Only a session under the wrapper can change effort; restart it after installing the wrapper."
        for status in [200, 503] {
            StandInRouter.answer("/api/live/effort", status, refusal(words))
            await feed.setEffort("high")
            XCTAssertEqual(feed.notice, LiveFeed.Notice(text: words, ok: false), "\(status) is a refusal, not \"Effort set to high\"")
        }
        XCTAssertEqual(StandInRouter.posts, ["/api/live/effort", "/api/live/effort"])
    }

    /// The fork gave no answer in time. The question stays in the feed, and no answer joins it.
    @MainActor
    func testASideQuestionTheRouterRefusesWithOkFalseIsShownAsARefusal() async throws {
        let feed = try feed()
        let words = "No answer within three minutes."
        for status in [200, 504] {
            StandInRouter.answer("/api/live/aside", status, refusal(words))
            await feed.aside("What is left to do?")
            XCTAssertEqual(feed.notice, LiveFeed.Notice(text: words, ok: false), "\(status) is a refusal, not \"Answered in 0s\"")
            XCTAssertFalse(feed.rows.contains { $0.item.kind == "aside" }, "\(status): nothing answered, so no answer in the feed")
        }
        XCTAssertEqual(StandInRouter.posts, ["/api/live/aside", "/api/live/aside"])
    }
}
