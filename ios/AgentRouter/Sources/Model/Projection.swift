// What a window's projection says, and how loudly. Mirrors the dashboard's wording so
// the phone and the Mac never disagree about the same numbers.

import Foundation

struct Projection: Equatable, Sendable {
    enum Tone: Sendable { case plain, warn, bad }
    let tone: Tone
    let text: String
    /// What the estimate was fitted on. Estimates say they are estimates.
    let basis: String?

    static func describe(_ w: Forecast.Window?, now: Date = .now) -> Projection? {
        guard let w else { return nil }
        if w.isExpired {
            let when = w.resetDate.map { Fmt.ago($0, now: now) } ?? "earlier"
            return Projection(tone: .plain, text: "Window rolled over \(when) · nothing seen since", basis: nil)
        }
        guard w.basis != "none", let left = w.activeHoursLeft else {
            return Projection(tone: .plain, text: "Not enough burn yet to project", basis: nil)
        }
        let basis = basisText(w)
        if let at = w.exhaustDate {
            let early = (w.resetDate ?? at).timeIntervalSince(at)
            return Projection(
                tone: .bad,
                text: "\(Fmt.hours(left)) of work left · runs out \(Fmt.relative(at, now: now)), \(Fmt.duration(early)) before it resets",
                basis: basis)
        }
        let end = Int(((w.projectedEndUtil ?? w.used ?? 0) * 100).rounded())
        return Projection(
            tone: end >= 90 ? .warn : .plain,
            text: "\(Fmt.hours(left)) of work left · resets first, ending ≈\(end)%",
            basis: basis)
    }

    private static func basisText(_ w: Forecast.Window) -> String? {
        guard let burn = w.burnPerActiveHour, burn > 0 else { return nil }
        let rate = String(format: "%.1f%% per hour of work", burn * 100)
        if w.basis == "trailing-windows" {
            return "\(rate) · nothing burnt yet this window, so this is your recent average"
        }
        let duty = Int(((w.dutyFraction ?? 0) * 100).rounded())
        return "\(rate), fitted on \(Fmt.hours(w.measuredActiveHours)) · \(duty)% of the clock spent working"
    }
}
