import XCTest
@testable import AgentRouter

/// Against a real router on the LAN, through the app's own networking — the path the
/// phone takes, arriving as a non-loopback peer. Skipped unless
/// /private/tmp/agent-router-live-test.json names a router and its password.
final class LiveNetworkTests: XCTestCase {
    struct Config: Decodable { let base: String; let password: String; let session: String }

    /// From `TEST_RUNNER_ROUTER_*` environment variables (xcodebuild forwards those to
    /// the runner, on a device too), or from a file on the simulator's host.
    private func config() throws -> Config {
        let env = ProcessInfo.processInfo.environment
        if let base = env["ROUTER_BASE"], let pw = env["ROUTER_PASSWORD"], let s = env["ROUTER_SESSION"] {
            return Config(base: base, password: pw, session: s)
        }
        guard let data = try? Data(contentsOf: URL(fileURLWithPath: "/private/tmp/agent-router-live-test.json")) else {
            throw XCTSkip("no router configured")
        }
        return try JSONDecoder().decode(Config.self, from: data)
    }

    func testTheAppCanListStreamAndReadStatsOverTheLAN() async throws {
        let c = try config()
        let client = RouterClient(baseURL: URL(string: c.base)!, password: c.password)
        let health: Health = try await client.get("/_router/health")
        XCTAssertTrue(health.ok)
        let auth: AuthStatus = try await client.get("/api/auth")
        print("router sees this test as loopback: \(auth.fromLoopback)")
        let sessions: [LiveSession] = try await client.get("/api/live/sessions")
        XCTAssertFalse(sessions.isEmpty)
        let stats: SessionStats = try await client.get("/api/live/stats?session=\(c.session)")
        XCTAssertGreaterThanOrEqual(stats.turns, 0)

        // Bounded: a stream that yields nothing must fail the test, not hang it.
        struct TimedOut: Error {}
        let stream = client.events("/api/live/stream?session=\(c.session)")
        let (items, statuses) = try await withThrowingTaskGroup(of: (Int, Int).self) { group in
            group.addTask {
                var items = 0, statuses = 0
                for try await ev in stream {
                    if ev.event == "status" { statuses += 1 } else if ev.event == nil { items += 1 }
                    if statuses >= 1 && items >= 1 { break }
                }
                return (items, statuses)
            }
            group.addTask {
                try await Task.sleep(for: .seconds(15))
                throw TimedOut()
            }
            let first = try await group.next()!
            group.cancelAll()
            return first
        }
        XCTAssertGreaterThan(items, 0, "backfilled rows")
        XCTAssertGreaterThan(statuses, 0, "a status event")

        // Only a non-loopback peer is asked for the password at all.
        guard !auth.fromLoopback else { return }
        let wrong = RouterClient(baseURL: URL(string: c.base)!, password: c.password + "x")
        do {
            let _: AuthStatus = try await wrong.get("/api/auth")
            XCTFail("a wrong password must be refused")
        } catch let e as RouterClient.Failure {
            XCTAssertEqual(e, .unauthorized)
        }
        do {
            for try await _ in wrong.events("/api/live/stream?session=\(c.session)") {
                XCTFail("must not stream")
                break
            }
            XCTFail("a wrong password must be refused on the stream too")
        } catch let e as RouterClient.Failure {
            XCTAssertEqual(e, .unauthorized)
        }
    }
}
