// What a window's projection says, and how loudly. Mirrors the dashboard's wording so
// the phone and the Mac never disagree about the same numbers — including the three-state
// verdict, where red is reserved for a crossing a quiet week would not undo.

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
        // Red is for a crossing that survives even a quiet week. When the band straddles
        // the reset the honest answer is that it could go either way, and saying that in
        // red spends the reader's alarm on a coin flip.
        if w.call == "tight", let at = w.exhaustDate {
            let early = (w.resetDate ?? at).timeIntervalSince(at)
            let quiet = Int(((w.projectedEndUtilLow ?? w.used ?? 0) * 100).rounded())
            return Projection(
                tone: .warn,
                text: "\(Fmt.hours(left)) of work left · too close to call — on this pace it runs out \(Fmt.duration(early)) before reset, on a quieter week it ends ≈\(quiet)%",
                basis: basis)
        }
        if w.call == "crosses", let at = w.exhaustDate {
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
        let pct = { (v: Double) in Int((v * 100).rounded()) }
        // Disclose the spread where there is one: it is why the line above hedges.
        var duty = "\(pct(w.dutyFraction ?? 0))% of the clock spent working"
        if let d = w.duty, w.hasBand {
            duty = "\(pct(d.mid))% of the clock spent working, \(pct(d.low))–\(pct(d.high))% across your days"
        }
        return "\(rate), fitted on \(Fmt.hours(w.measuredActiveHours)) · \(duty)"
    }
}
