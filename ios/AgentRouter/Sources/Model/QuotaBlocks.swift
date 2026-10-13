// One quota block per provider with an account, as the console's Quota panel draws them
// (ui/src/screens/Overview/quotaView.ts, Quota.tsx, ui/src/model/forecast.ts
// `providerHeadroom`). Each block describes its provider's serving account: its windows by
// their own names, where the reading came from, and its own headroom, never set against another
// provider's windows. A provider with no account has no block.

import Foundation

/// One window's meter in a block.
struct QuotaMeter: Identifiable, Sendable {
    let key: WindowKey
    /// Fraction used, 0–1, or nil with no reading — never "0% used".
    let used: Double?
    let resetsAt: Date?
    /// The window rolled over: its sentence, and no figure.
    let expired: String?
    /// The forecast for the same window of the same account, for its projection line.
    let forecast: Forecast.Window?
    var id: WindowKey { key }
    var label: String { key.label }
}

/// The tag beside a block's meters: its word and colour from one field.
struct QuotaStatus: Equatable, Sendable {
    enum Tone: Sendable { case ok, warn, bad, neutral }
    let word: String
    let tone: Tone

    /// Red only for critical or rejected; amber for a warning; green for normal or allowed.
    static func of(_ field: String?) -> QuotaStatus? {
        guard let field, !field.isEmpty else { return nil }
        let f = field.lowercased()
        if f == "rejected" || f.contains("critical") { return QuotaStatus(word: field, tone: .bad) }
        if f.contains("warning") { return QuotaStatus(word: field, tone: .warn) }
        if f == "normal" || f == "allowed" { return QuotaStatus(word: field, tone: .ok) }
        return QuotaStatus(word: field, tone: .neutral)
    }
}

/// A provider's own headroom: hours of work left over its own windows, and the first of them
/// to run out.
struct ProviderRoom: Sendable {
    enum Tone: Sendable { case plain, warn, bad }
    /// "6.2 h".
    let value: String
    /// What the tile says under the value.
    let sub: String
    let tone: Tone
    /// "First to run out: work's 7-day window, Oct 14, 09:00." — nil when nothing is projected to.
    let first: String?

    static func of(_ forecast: Forecast?, provider: String, now: Date = .now) -> ProviderRoom? {
        guard let room = forecast?.global.providers[provider] else { return nil }
        guard let first = room.firstToExhaust else {
            let pools = "\(Fmt.count(Double(room.pools))) \(room.pools == 1 ? "quota pool" : "quota pools")"
            return ProviderRoom(value: Fmt.hours(room.activeHoursLeft), sub: "\(pools), none projected to run out", tone: .plain, first: nil)
        }
        let account = forecast?.accounts.first { $0.id == first.accountId }
        let key = WindowKey(rawValue: first.window) ?? .fiveHour
        let w = account.flatMap { a in a.windows.first { $0.key == key }?.window } ?? nil
        let verdict = w?.call ?? "crosses"
        let who = account.map { $0.label.isEmpty ? $0.id : $0.label } ?? first.accountId
        let at = first.inMs > 0 ? Fmt.relative(now.addingTimeInterval(TimeInterval(first.inMs) / 1000), now: now) : "now"
        let sub = switch verdict {
        case "tight": "too close to call: \(who)'s \(key.name) window could run out \(at)"
        case "crosses": "first to go: \(who)'s \(key.name) window, \(at)"
        default: "\(who)'s \(key.name) window is projected to reset first"
        }
        let when = Fmt.when(Date(timeIntervalSince1970: TimeInterval(first.at) / 1000))
        return ProviderRoom(value: Fmt.hours(room.activeHoursLeft), sub: sub,
                            tone: verdict == "crosses" ? .bad : verdict == "tight" ? .warn : .plain,
                            first: "First to run out: \(who)'s \(key.name) window, \(when)\(verdict == "tight" ? " — too close to call" : "").")
    }
}

/// Everything one provider's block shows.
struct QuotaBlock: Identifiable, Sendable {
    let provider: String
    let accountId: String
    /// "work@example.com · Max": the serving account by its email, else its label.
    let serving: String
    /// The 5-hour window and the second by its own name; nil with no reading yet.
    let meters: [QuotaMeter]?
    let status: QuotaStatus?
    /// Where the numbers came from and how old they are: "checked 2 min ago".
    let source: String?
    let room: ProviderRoom?
    /// The serving account's forecast, for its burn-downs.
    let forecast: Forecast.ForecastAccount?
    let history: [ProviderQuota.HistoryRow]
    var id: String { provider }
    var name: String { Provider.name(provider) }
    var secondKey: WindowKey { meters?.last?.key ?? forecast?.secondKey ?? .sevenDay }

    /// One block. Anthropic's prefers the live usage reading (`/api/quota`, read at `liveAt`),
    /// which stays current on an idle machine, when it was read for the serving account; else
    /// the last response's rate-limit headers. Any other provider's windows are its usage API's
    /// reading, its second by its own name.
    static func make(_ block: ProviderQuota, live: Quota?, liveAt: Date?, forecast: Forecast?, now: Date = .now) -> QuotaBlock {
        let fc = forecast?.accounts.first { $0.id == block.account.id }
        let anthropic = block.provider == "anthropic"
        func meter(_ key: WindowKey, _ used: Double?, _ reset: Date?) -> QuotaMeter {
            let rolled = reset.map { $0 <= now } ?? false
            return QuotaMeter(key: key, used: rolled ? nil : used, resetsAt: rolled ? nil : reset,
                              expired: rolled ? "window rolled over \(Fmt.ago(reset!, now: now)) · nothing seen since" : nil,
                              forecast: fc.flatMap { a in a.windows.first { $0.key == key }?.window } ?? (key == .sevenDay ? fc?.sevenDay : nil))
        }
        var meters: [QuotaMeter]?
        var status: QuotaStatus?
        var source: String?
        if anthropic, let q = live, q.source != "unavailable", q.fiveHour != nil, q.accountId == nil || q.accountId == block.account.id {
            let headers = q.source == "response-headers"
            let seen = headers ? (Fmt.date(ms: q.seenAt) ?? liveAt) : liveAt.map { $0.addingTimeInterval(-TimeInterval(q.ageMs ?? 0) / 1000) }
            meters = [meter(.fiveHour, q.fiveHour?.fraction, q.fiveHour?.resetDate), meter(.sevenDay, q.sevenDay?.fraction, q.sevenDay?.resetDate)]
            status = .of(q.severity)
            source = "\(headers ? "from the last request" : "checked") \(seen.map { Fmt.ago($0, now: now) } ?? "—")"
        } else if block.windows.values.contains(where: { $0 != nil }) {
            let second: WindowKey = block.windows.keys.contains("month") ? .month
                : block.windows.keys.contains("seven_day") ? .sevenDay : (fc?.month != nil ? .month : .sevenDay)
            func of(_ key: WindowKey) -> QuotaMeter {
                let w = block.window(key)
                return meter(key, w?.utilization, w?.resetsAt.map { Date(timeIntervalSince1970: $0) })
            }
            meters = [of(.fiveHour), of(second)]
            status = .of(block.status)
            let read = anthropic ? "from the last request" : "checked"
            source = Fmt.date(ms: block.at).map { "\(read) \(Fmt.ago($0, now: now))" } ?? (anthropic ? read : "usage not read yet")
        }
        let who = block.account.email ?? block.account.label ?? block.account.id
        return QuotaBlock(provider: block.provider, accountId: block.account.id,
                          serving: block.account.plan.map { "\(who) · \($0)" } ?? who,
                          meters: meters, status: status, source: source,
                          room: ProviderRoom.of(forecast, provider: block.provider, now: now), forecast: fc, history: block.history)
    }
}

/// A router's accounts by provider: one section each, Anthropic's first, each with the account
/// its requests go to.
struct ProviderAccounts: Identifiable, Sendable {
    let provider: String
    let accounts: [Account]
    var id: String { provider }
    var name: String { Provider.name(provider) }
    /// Where this provider's requests go: its active account.
    var active: Account? { accounts.first(where: \.isActive) }

    static func group(_ accounts: [Account]) -> [ProviderAccounts] {
        Dictionary(grouping: accounts) { $0.provider ?? "anthropic" }
            .map { ProviderAccounts(provider: $0.key, accounts: $0.value) }
            .sorted { (Provider.rank($0.provider), $0.provider) < (Provider.rank($1.provider), $1.provider) }
    }
}
