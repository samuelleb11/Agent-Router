import XCTest
@testable import AgentRouter

/// The phone must say what the dashboard says, for every state a window can be in.
final class ProjectionTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_789_200_000)

    private func window(_ json: String) throws -> Forecast.Window {
        try JSONDecoder().decode(Forecast.Window.self, from: Data(json.utf8))
    }

    func testRunsOutBeforeTheResetIsRed() throws {
        let w = try window(#"{"used":0.98,"reset_at":1789308000,"window_start":1788840000,"burn_per_active_hour":0.0121,"active_hours_left":1.65,"duty_fraction":0.139,"exhausts_at":1789243200000,"resets_first":false,"projected_end_util":1.0,"basis":"this-window","measured_active_hours":7.4,"readings":2639,"expired":false,"series":[]}"#)
        let p = try XCTUnwrap(Projection.describe(w, now: now))
        XCTAssertEqual(p.tone, .bad)
        XCTAssertTrue(p.text.hasPrefix("1.6 h of work left · runs out in 12h 00m"), p.text)
        XCTAssertTrue(p.text.hasSuffix("before it resets"), p.text)
        XCTAssertTrue(try XCTUnwrap(p.basis).contains("fitted on 7.4 h"))
    }

    /// The band's reason for existing: a crossing 2 h before a reset 95 h out, from a
    /// record whose days ran 25–41% duty. Amber and worded as a coin flip, not red.
    func testAStraddlingBandIsAmberAndSaysSo() throws {
        let w = try window(#"{"used":0.49,"reset_at":1789750800,"window_start":1789146000,"burn_per_active_hour":0.0167,"active_hours_left":30.3,"duty_fraction":0.329,"duty":{"low":0.248,"mid":0.329,"high":0.409},"exhausts_at":1789743000000,"exhausts_at_early":1789674000000,"exhausts_at_late":null,"resets_first":false,"verdict":"tight","projected_end_util":1.0,"projected_end_util_low":0.883,"projected_end_util_high":1.0,"basis":"this-window","measured_active_hours":29.1,"readings":7935,"expired":false,"series":[]}"#)
        let p = try XCTUnwrap(Projection.describe(w))
        XCTAssertEqual(p.tone, .warn, "a coin flip must not be red")
        XCTAssertTrue(p.text.contains("too close to call"), p.text)
        XCTAssertTrue(p.text.contains("ends ≈88%"), "the quiet end is the reassuring half: \(p.text)")
        XCTAssertTrue(try XCTUnwrap(p.basis).contains("25–41% across your days"), p.basis ?? "")
    }

    /// A router that predates the band sends no verdict and no duty range. Everything it
    /// does send must still read exactly as it did.
    func testAnOlderRouterStillRendersAsItAlwaysDid() throws {
        let w = try window(#"{"used":0.98,"reset_at":1789308000,"window_start":1788840000,"burn_per_active_hour":0.0121,"active_hours_left":1.65,"duty_fraction":0.139,"exhausts_at":1789243200000,"resets_first":false,"projected_end_util":1.0,"basis":"this-window","measured_active_hours":7.4,"readings":2639,"expired":false,"series":[]}"#)
        XCTAssertEqual(w.call, "crosses")
        XCTAssertFalse(w.hasBand)
        let p = try XCTUnwrap(Projection.describe(w))
        XCTAssertEqual(p.tone, .bad)
        XCTAssertTrue(try XCTUnwrap(p.basis).contains("14% of the clock spent working"), p.basis ?? "")
        XCTAssertFalse(try XCTUnwrap(p.basis).contains("across your days"), "no spread, nothing to disclose")
    }

    func testResetsFirstIsCalm() throws {
        let w = try window(#"{"used":0.06,"reset_at":1789213800,"window_start":1789195800,"burn_per_active_hour":0.0536,"active_hours_left":17.5,"duty_fraction":0.955,"exhausts_at":null,"resets_first":true,"projected_end_util":0.26,"basis":"this-window","measured_active_hours":1.1,"readings":273,"expired":false,"series":[]}"#)
        let p = try XCTUnwrap(Projection.describe(w, now: now))
        XCTAssertEqual(p.tone, .plain)
        XCTAssertEqual(p.text, "18 h of work left · resets first, ending ≈26%")
    }

    func testNearlyFullAtResetWarns() throws {
        let w = try window(#"{"used":0.5,"reset_at":1789213800,"window_start":1789195800,"burn_per_active_hour":0.1,"active_hours_left":5,"duty_fraction":1,"exhausts_at":null,"resets_first":true,"projected_end_util":0.93,"basis":"this-window","measured_active_hours":2,"readings":10,"expired":false}"#)
        XCTAssertEqual(Projection.describe(w, now: now)?.tone, .warn)
    }

    func testABorrowedRateNeverClaimsAFit() throws {
        let w = try window(#"{"used":0.01,"reset_at":1789213800,"window_start":1789195800,"burn_per_active_hour":0.05,"active_hours_left":19.8,"duty_fraction":1,"exhausts_at":null,"resets_first":true,"projected_end_util":0.25,"basis":"trailing-windows","measured_active_hours":0,"readings":12,"expired":false}"#)
        let basis = try XCTUnwrap(Projection.describe(w, now: now)?.basis)
        XCTAssertFalse(basis.contains("fitted on"))
        XCTAssertTrue(basis.contains("recent average"))
    }

    func testNothingToGoOnAndExpiredSayNothingAlarming() throws {
        let none = try window(#"{"used":0,"reset_at":1789213800,"window_start":1789195800,"burn_per_active_hour":0,"active_hours_left":null,"duty_fraction":0.01,"exhausts_at":null,"resets_first":true,"projected_end_util":0,"basis":"none","measured_active_hours":0,"readings":1,"expired":false}"#)
        let p = try XCTUnwrap(Projection.describe(none, now: now))
        XCTAssertEqual(p.tone, .plain)
        XCTAssertNil(p.basis)
        let expired = try window(#"{"expired":true,"reset_at":1789123200,"seen_at":1789105528587}"#)
        let e = try XCTUnwrap(Projection.describe(expired, now: now))
        XCTAssertEqual(e.tone, .plain)
        XCTAssertTrue(e.text.hasPrefix("Window rolled over"), e.text)
        XCTAssertNil(Projection.describe(nil))
    }

    func testHoursReadAsHours() {
        XCTAssertEqual(Fmt.hours(0.25), "15 min")
        XCTAssertEqual(Fmt.hours(2.4), "2.4 h")
        XCTAssertEqual(Fmt.hours(81), "81 h")
        XCTAssertEqual(Fmt.hours(nil), "—")
        XCTAssertEqual(Fmt.usd(1787.35), "$1,787")
        XCTAssertEqual(Fmt.usd(0.891), "$0.891")
        XCTAssertEqual(Fmt.compact(2_676_119_385), "2.68B")
        XCTAssertEqual(Fmt.compact(999_600), "1.00M")
    }
}
