import XCTest
import os
@testable import AgentRouter

/// A live stream the router refuses with an error status reaches the screen in the router's
/// own words.
///
/// A stream once threw its bare status without reading the body, and the feed then retried
/// in silence, so a hosted router's "the machine running this session (mini) is offline"
/// never reached the page at all. A hosted router relays each session's stream to the
/// machine holding it, and refuses as `machines::Relay` does: 503 naming the machine when it
/// is offline — back later, never a session that ended — 404 when no machine holds the
/// session, and 502 when the relay failed. Each is read the way every call's error status is
/// (`RouterClient.failure`); the feed goes on retrying, and says why in its header.
///
/// The stream goes through the app's own client to a stand-in router: a URL protocol on the
/// client's stream session, so nothing leaves the process.
final class StreamRefusalTests: XCTestCase {
    /// A hosted router's words for a machine that is offline (src/machines.rs, `Relay`).
    private let offline = "the machine running this session (mini) is offline"

    private func client() -> RouterClient {
        StreamStandIn.reset()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [StreamStandIn.self]
        let session = URLSession(configuration: config)
        return RouterClient(baseURL: URL(string: "https://router.invalid")!, password: nil, calls: session, streams: session)
    }

    private func refusal(_ words: String) -> String { #"{"ok":false,"error":"\#(words)"}"# }

    /// Everything the stream yields, or how it ended.
    @MainActor
    private func drain(_ events: AsyncThrowingStream<SSEEvent, Error>) async -> Error? {
        do {
            for try await _ in events {}
            return nil
        } catch {
            return error
        }
    }

    @MainActor
    func testAnErrorStatusOnALiveStreamIsSaidInTheRoutersWords() async throws {
        let client = client()
        for (session, status, words) in [
            ("gone", 404, "No such session."),
            ("down", 503, offline),
            ("relay", 502, "mini did not answer in time"),
        ] {
            StreamStandIn.answer("/api/live/stream?session=\(session)", status, refusal(words))
            let ended = await drain(client.events("/api/live/stream?session=\(session)"))
            XCTAssertEqual(ended as? RouterClient.Failure, .refused(status, words), "\(status) is not an empty stream")
            XCTAssertEqual(LiveFeed.describe(try XCTUnwrap(ended)), words)
        }
    }

    /// With nothing to say for itself, an error status is named — the one that was sent.
    @MainActor
    func testAnErrorStatusWithNoWordsNamesTheStatusTheRouterSent() async throws {
        let client = client()
        StreamStandIn.answer("/api/live/stream?session=wordless", 503, "")
        let wordless = await drain(client.events("/api/live/stream?session=wordless"))
        XCTAssertEqual(wordless as? RouterClient.Failure, .status(503, ""))
        XCTAssertEqual(LiveFeed.describe(try XCTUnwrap(wordless)), "Router returned 503")

        StreamStandIn.answer("/api/live/stream?session=plain", 500, "upstream exploded")
        let plain = await drain(client.events("/api/live/stream?session=plain"))
        XCTAssertEqual(LiveFeed.describe(try XCTUnwrap(plain)), "Router returned 500: upstream exploded")
    }

    /// Offline is not ended: the feed says what the router said, keeps the header on it
    /// while it retries — without raising the notice again on every retry — and reads the
    /// session again as soon as its machine is back.
    @MainActor
    func testASessionWhoseMachineIsOfflineReadsAsOfflineUntilItIsBack() async throws {
        let client = client()
        StreamStandIn.offline(for: 3)
        let json = #"{"session_id":"offline","cwd":"/code/p","project":"-code-p","git_branch":null,"title":null,"created_ms":0,"last_activity_ms":0,"active":true,"state":"working","archived":false,"wrapped":true,"bytes":10}"#
        let session = try JSONDecoder().decode(LiveSession.self, from: Data(json.utf8))
        let feed = LiveFeed(session: session, client: client)
        feed.retryAfter = .milliseconds(50)
        let running = Task { await feed.run() }
        defer { running.cancel() }

        try await eventually("the refusal") { feed.refused != nil }
        XCTAssertEqual(feed.refused, offline)
        XCTAssertEqual(feed.notice, LiveFeed.Notice(text: offline, ok: false))
        XCTAssertEqual(feed.linkLine, offline, "the header says it, not \"reconnecting…\"")
        XCTAssertFalse(feed.connected)
        XCTAssertNil(feed.status, "nothing said the session ended")

        // Dismissed, it stays dismissed while the router keeps saying the same.
        feed.notice = nil
        try await eventually("the retries") { StreamStandIn.offlineConnections >= 3 }
        XCTAssertNil(feed.notice, "said once, not again on every retry")
        XCTAssertEqual(feed.linkLine, offline)

        try await eventually("the machine back") { feed.status?.state == "working" }
        XCTAssertNil(feed.refused, "the stream delivers again, so nothing stands refused")
    }

    @MainActor
    private func eventually(_ what: String, within: Duration = .seconds(8), _ done: () -> Bool) async throws {
        let end = ContinuousClock.now.advanced(by: within)
        while !done() {
            guard ContinuousClock.now < end else { return XCTFail("never happened: \(what)") }
            try await Task.sleep(for: .milliseconds(10))
        }
    }
}

/// A router with no network under it, for streams: answers each path and query from a
/// table, and the offline session's stream with a 503 naming its machine until that machine
/// is back, then with a stream of one status. Anything not in the table is a 404 with
/// nothing to say.
final class StreamStandIn: URLProtocol {
    struct Reply: Sendable {
        var status: Int
        var body: String
    }

    private struct State: Sendable {
        var table: [String: Reply] = [:]
        var offlineFor = 0
        var offlineConnections = 0
    }

    private static let state = OSAllocatedUnfairLock<State>(initialState: State())
    private static let offlinePath = "/api/live/stream?session=offline"

    static func reset() {
        state.withLock { $0 = State() }
    }

    static func answer(_ path: String, _ status: Int, _ body: String) {
        state.withLock { $0.table[path] = Reply(status: status, body: body) }
    }

    /// Refuse the offline session's stream this many times before its machine is back.
    static func offline(for connections: Int) {
        state.withLock { $0.offlineFor = connections }
    }

    static var offlineConnections: Int { state.withLock { $0.offlineConnections } }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let url = request.url else { return }
        let key = url.path + (url.query.map { "?\($0)" } ?? "")
        let reply: Reply = Self.state.withLock { s in
            guard key == Self.offlinePath else { return s.table[key] ?? Reply(status: 404, body: "") }
            s.offlineConnections += 1
            if s.offlineConnections <= s.offlineFor {
                return Reply(status: 503, body: #"{"ok":false,"error":"the machine running this session (mini) is offline"}"#)
            }
            return Reply(status: 200, body: "event: status\ndata: {\"state\":\"working\",\"detail\":\"Thinking\",\"quiet_ms\":0}\n\n")
        }
        let type = reply.status == 200 ? "text/event-stream" : reply.body.hasPrefix("{") ? "application/json" : "text/plain"
        let response = HTTPURLResponse(url: url, statusCode: reply.status, httpVersion: "HTTP/1.1",
                                       headerFields: ["content-type": type])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(reply.body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}
