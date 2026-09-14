import XCTest
@testable import AgentRouter

/// The router's real payloads decode. The forecast fixture is a captured
/// `/api/forecast` with the identities replaced.
final class DecodingTests: XCTestCase {
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

    func testBasicAuthorizationHeaderCarriesThePassword() throws {
        let header = RouterClient.authorization(password: "hunter22")
        XCTAssertTrue(header.hasPrefix("Basic "))
        let decoded = try XCTUnwrap(Data(base64Encoded: String(header.dropFirst(6))))
        XCTAssertEqual(String(decoding: decoded, as: UTF8.self), "app:hunter22")
    }
}
