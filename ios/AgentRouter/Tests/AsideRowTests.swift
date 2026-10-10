import XCTest
@testable import AgentRouter

/// A side question's own row says it is one — "you · side question", the web view's words for
/// it (SE-C160) — and never "you · queued", whether the question is answered or refused.
///
/// A side question is answered in a fork the session never sees (src/live.rs, `aside`): it is
/// never queued into the session, and no transcript ever has it. The phone echoed it as a
/// message sent all the same, and nothing ever changed that, so its row went on reading
/// "you · queued", dimmed, as if the session were still to take it up.
///
/// Each question goes through the session screen's own feed and the app's own client to the
/// stand-in router `RefusalTests` uses, so nothing leaves the process. The answer is the
/// router's reply, field for field, and the refusal its words for a fork that gave no answer
/// in time.
final class AsideRowTests: XCTestCase {
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

    /// Answered: the question, still marked as one, and the side answer under it.
    @MainActor
    func testAnAnsweredSideQuestionIsMarkedAsOneNeverAsQueued() async throws {
        let feed = try feed()
        StandInRouter.answer("/api/live/aside", 200,
            #"{"ok":true,"answer":"Two tests are left.","fork":"f-1","cost":0.02,"took_ms":18000}"#)
        await feed.aside("What is left to do?")
        XCTAssertEqual(feed.rows.map(\.item.kind), ["text", "aside"], "the question, then its answer")
        let question = try XCTUnwrap(feed.rows.first)
        XCTAssertEqual(question.item.text, "What is left to do?")
        XCTAssertEqual(question.label, "you · side question")
        XCTAssertFalse(question.pending, "never queued: the session never receives it")
        XCTAssertNil(feed.rows.last?.label, "the answer is the router's, not the person's")
        XCTAssertEqual(StandInRouter.posts, ["/api/live/aside"])
    }

    /// Refused, in a 200 and in the 504 a current router sends: the question stays as it was
    /// asked, marked as one, with no answer under it, and the router's words are the notice.
    @MainActor
    func testARefusedSideQuestionIsMarkedAsOneNeverAsQueued() async throws {
        let words = "No answer within three minutes."
        for status in [200, 504] {
            let feed = try feed()
            StandInRouter.answer("/api/live/aside", status, #"{"ok":false,"reason":"\#(words)"}"#)
            await feed.aside("What is left to do?")
            XCTAssertEqual(feed.notice, LiveFeed.Notice(text: words, ok: false), "\(status): the router's words")
            XCTAssertEqual(feed.rows.map(\.item.text), ["What is left to do?"], "\(status): the question, and no answer")
            let question = try XCTUnwrap(feed.rows.first)
            XCTAssertEqual(question.label, "you · side question", "\(status)")
            XCTAssertFalse(question.pending, "\(status): never queued")
            XCTAssertEqual(StandInRouter.posts, ["/api/live/aside"])
        }
    }

    /// What it is told apart from: a message sent is queued until the transcript has it, and
    /// says so as before.
    @MainActor
    func testAMessageSentIsStillQueued() async throws {
        let feed = try feed()
        StandInRouter.answer("/api/live/send", 200, #"{"ok":true}"#)
        await feed.send("Carry on.")
        let sent = try XCTUnwrap(feed.rows.first)
        XCTAssertEqual(sent.label, "you · queued")
        XCTAssertTrue(sent.pending)
        XCTAssertFalse(sent.sideQuestion)
        XCTAssertEqual(StandInRouter.posts, ["/api/live/send"])
    }
}
