// Numbers the way the dashboard shows them.

import Foundation

enum Fmt {
    static func hours(_ h: Double?) -> String {
        guard let h else { return "—" }
        if h < 1 { return "\(Int((h * 60).rounded())) min" }
        if h < 10 { return String(format: "%.1f h", h) }
        return "\(Int(h.rounded())) h"
    }

    static func usd(_ v: Double?) -> String {
        guard let v else { return "—" }
        let a = abs(v)
        if a >= 1000 { return "$" + Int(a.rounded()).formatted() }
        if a >= 1 { return String(format: "$%.2f", a) }
        if a > 0 { return String(format: a >= 0.01 ? "$%.3f" : "$%.4f", a) }
        return "$0"
    }

    static func compact(_ v: Double?) -> String {
        guard let v else { return "—" }
        let a = abs(v)
        if a >= 1e9 || (a / 1e6).rounded() >= 1000 { return String(format: "%.2fB", v / 1e9) }
        if a >= 1e6 || (a / 1e3).rounded() >= 1000 { return String(format: "%.2fM", v / 1e6) }
        if a >= 1e4 { return String(format: "%.0fk", v / 1e3) }
        if a >= 1e3 { return String(format: "%.1fk", v / 1e3) }
        return "\(Int(v.rounded()))"
    }

    static func count(_ v: Double?) -> String {
        guard let v else { return "—" }
        return Int(v.rounded()).formatted()
    }

    /// Whole percent, for meters.
    static func percent(_ fraction: Double?) -> String {
        guard let f = fraction else { return "—" }
        return "\(Int((f * 100).rounded()))%"
    }

    /// A share the way the dashboard's `pct()` shows it: a decimal under ten percent,
    /// so 27 errors in 8,474 requests reads 0.3%, not 0%.
    static func share(_ fraction: Double?) -> String {
        guard let f = fraction, f.isFinite else { return "—" }
        let p = f * 100
        return (p >= 10 || p == 0) ? "\(Int(p.rounded()))%" : String(format: "%.1f%%", p)
    }

    /// "in 2h 10m", "in 3d 4h", "now".
    static func relative(_ date: Date, now: Date = .now) -> String {
        let s = date.timeIntervalSince(now)
        if s <= 0 { return "now" }
        let m = Int((s / 60).rounded())
        if m < 60 { return "in \(m) min" }
        let h = m / 60
        if h < 24 { return "in \(h)h \(String(format: "%02d", m % 60))m" }
        return "in \(h / 24)d \(h % 24)h"
    }

    /// "just now", "3 min ago", "2 h ago", "4 d ago".
    static func ago(_ date: Date, now: Date = .now) -> String {
        let s = Int(now.timeIntervalSince(date).rounded())
        if s < 45 { return "just now" }
        let m = Int((Double(s) / 60).rounded())
        if m < 60 { return "\(m) min ago" }
        let h = Int((Double(m) / 60).rounded())
        if h < 24 { return "\(h) h ago" }
        let d = Int((Double(h) / 24).rounded())
        return d < 7 ? "\(d) d ago" : date.formatted(.dateTime.month(.abbreviated).day())
    }

    /// "45s", "3h 12m", "1d 0h".
    static func duration(_ interval: TimeInterval) -> String {
        let s = max(0, Int(interval.rounded()))
        if s < 60 { return "\(s)s" }
        let m = s / 60
        if m < 60 { return "\(m)m \(String(format: "%02d", s % 60))s" }
        let h = m / 60
        if h < 24 { return "\(h)h \(String(format: "%02d", m % 60))m" }
        return "\(h / 24)d \(h % 24)h"
    }

    static func when(_ date: Date) -> String {
        date.formatted(.dateTime.month(.abbreviated).day().hour().minute())
    }

    /// "812 ms" / "3.2 s".
    static func ms(_ v: Double?) -> String {
        guard let v else { return "—" }
        return v < 1000 ? "\(Int(v.rounded())) ms" : String(format: "%.1f s", v / 1000)
    }

    /// The dashboard's bucket names: "minute", "5 minutes", "hour", "day"…
    static func bucketName(_ ms: Int) -> String {
        switch ms {
        case ..<60_000: "\(ms / 1000)s"
        case 60_000: "minute"
        case ..<3_600_000: "\(ms / 60_000) minutes"
        case 3_600_000: "hour"
        case ..<86_400_000: "\(ms / 3_600_000) hours"
        default: "day"
        }
    }

    /// `claude-opus-5-20260115` → `opus-5-20260115`.
    static func shortModel(_ m: String?) -> String {
        guard let m, !m.isEmpty else { return "—" }
        return m.hasPrefix("claude-") ? String(m.dropFirst(7)) : m
    }

    static func hm(_ date: Date) -> String {
        date.formatted(.dateTime.hour().minute())
    }

    static func date(ms: Int?) -> Date? {
        ms.map { Date(timeIntervalSince1970: TimeInterval($0) / 1000) }
    }

    // MARK: costs, as the console sums them (ui/src/model/format.ts, `costSum`)

    /// Why a sum leaves requests out.
    static let unpricedWhy = "no rate for their model in the pricing table, so the sum leaves them out"
    /// Why a request's tokens are an estimate.
    static let estimatedWhy = "tokens estimated rather than read from the stored response, for Kimi Code requests from before cache reads were counted once whose responses were no longer kept"

    /// A cost as written, and which marks it carries.
    struct CostFigure: Equatable, Sendable {
        /// "$3.10", "$3.10 + 4 unpriced", "≈$3.10", "not priced", "$0", "—".
        let text: String
        let unpriced: Bool
        let estimated: Bool
    }

    /// A sum of costs, one rule wherever a cost is summed: the priced part, marked "+ N
    /// unpriced" when it leaves requests out, "not priced" when nothing in it was priced —
    /// never "$0", which stands only for a zero the router measured — and "≈" before it when
    /// the dollars rest on estimated tokens. A cost the router did not give is "—".
    static func costSum(_ cost: Double?, unpriced: Double? = nil, estimated: Double? = nil) -> CostFigure {
        let left = whole(unpriced), guessed = whole(estimated)
        let priced = (cost ?? 0) > 0
        let dollars = left > 0 ? (priced ? "\(usd(cost)) + \(count(Double(left))) unpriced" : "not priced") : usd(cost)
        return CostFigure(text: guessed > 0 && priced ? "≈" + dollars : dollars, unpriced: left > 0, estimated: guessed > 0 && priced)
    }

    /// One request's cost: "not priced" for a request with usage and no cost, never "$0"; "—"
    /// for one with no usage to price.
    static func requestCost(_ cost: Double?, promptTokens: Double?, outputTokens: Double?) -> CostFigure {
        if cost == nil, promptTokens != nil || outputTokens != nil { return CostFigure(text: "not priced", unpriced: true, estimated: false) }
        return costSum(cost)
    }

    /// What the marks on a list's costs mean, said once under it. Nil when none is marked.
    static func costNote(_ figures: [CostFigure]) -> String? {
        let said = [figures.contains(where: \.unpriced) ? "Unpriced: \(unpricedWhy)." : nil,
                    figures.contains(where: \.estimated) ? "≈ estimated: \(estimatedWhy)." : nil].compactMap { $0 }
        return said.isEmpty ? nil : said.joined(separator: " ")
    }

    private static func whole(_ v: Double?) -> Int { v.map { $0 > 0 && $0.isFinite ? Int($0.rounded()) : 0 } ?? 0 }
}
