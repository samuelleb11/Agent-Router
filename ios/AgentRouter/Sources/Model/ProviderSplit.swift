// The Usage page across providers, as the console's Overview reads it: each total split by
// provider (ui/src/screens/Overview/tiles.ts, `providerSplit`) and the provider a scope can be
// narrowed to (ui/src/screens/Overview/scopeOptions.ts, `providerChoices`).

import Foundation

/// One provider's part of a total.
struct ProviderPart: Identifiable, Equatable, Sendable {
    let provider: String
    let value: Double
    /// The part as written: "1.20M", "$3.10 + 4 unpriced", "—".
    let figure: String
    /// Its share of the whole, 0–1; nil for a spend with nothing priced.
    let share: Double?
    var id: String { provider }
}

enum ProviderSplit {
    /// The totals that split. Latency and errors do not: the router gives no figure per provider.
    enum Total: Sendable { case prompt, spend, requests, output, cache, writes }

    /// The providers the Overview's scope can name, as the console has them (ui/src/model/scope.ts,
    /// `PROVIDERS`): OpenAI is not among them, on the console or here (AGEN-0190).
    static let scopable = ["anthropic", "kimi"]

    /// A total's parts, in the router's order, Anthropic's first. Nil when fewer than two
    /// providers served in scope: a router with one provider, or a scope narrowed to one, shows
    /// no split.
    static func parts(_ rows: [ProviderTotals], _ total: Total) -> [ProviderPart]? {
        guard rows.count >= 2 else { return nil }
        func of(_ r: ProviderTotals) -> Double {
            let v: Double? = switch total {
            case .prompt: r.promptTokens
            case .spend: r.cost
            case .requests: r.requests
            case .output: r.outputTokens
            case .cache: r.cacheReadTokens
            case .writes: r.cacheWriteTokens
            }
            return v.flatMap { $0.isFinite ? $0 : nil } ?? 0
        }
        let whole = rows.reduce(0) { $0 + of($1) }
        let share = { (v: Double) in whole > 0 ? v / whole : nil }
        return rows.map { r in
            let v = of(r)
            switch total {
            case .spend:
                // A cost sum: "not priced" beside requests the router could not price, "—" when
                // nothing was priced at all, never "$0".
                let priced = v > 0
                let sum = Fmt.costSum(r.cost, unpriced: r.unpriced, estimated: r.estimated)
                return ProviderPart(provider: r.provider, value: priced ? v : 0,
                                    figure: priced || (r.unpriced ?? 0) > 0 ? sum.text : "—", share: priced ? share(v) : nil)
            case .requests:
                return ProviderPart(provider: r.provider, value: v, figure: Fmt.count(v), share: share(v))
            default:
                return ProviderPart(provider: r.provider, value: v, figure: Fmt.compact(v), share: share(v))
            }
        }
    }

    /// The provider choice: every provider (nil) first, then each provider with an account the
    /// scope can name, Anthropic's first — and the one chosen, kept once it has no account, so it
    /// can be let go. Nil, offering nothing, while at most one provider has an account and none
    /// is chosen.
    static func choices(withAccount: [String], chosen: String?) -> [String?]? {
        var known: [String] = []
        for p in withAccount where scopable.contains(p) && !known.contains(p) { known.append(p) }
        known.sort { Provider.rank($0) < Provider.rank($1) }
        if let chosen, !known.contains(chosen) { known.append(chosen) }
        if known.count < 2 && chosen == nil { return nil }
        return [nil] + known.map(Optional.some)
    }
}

extension RequestRow {
    /// The account that signed it, as the console's request detail names it: "email · id",
    /// else its label and id, else the id alone when the router no longer holds it.
    func accountWords(_ accounts: [Account]) -> String? {
        guard let id = accountId else { return nil }
        guard let a = accounts.first(where: { $0.id == id }) else { return id }
        let who = a.email ?? a.label
        return who == id ? id : "\(who) · \(id)"
    }
}
