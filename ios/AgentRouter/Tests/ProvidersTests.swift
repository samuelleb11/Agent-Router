import UIKit
import XCTest
@testable import AgentRouter

/// Providers across the app, as the console has them (AGEN-0143): each provider's mark, the
/// accounts by provider each with its own serving account, one quota block per provider with
/// its own headroom, totals split by provider, a provider in the scope, the provider named on
/// every row, and sums that never say "$0" for what could not be priced. The fixtures are the
/// router's replies with the identities replaced.
final class ProvidersTests: XCTestCase {
    /// The fixtures' moment: 2026-10-12, before every window in them resets.
    private let now = Date(timeIntervalSince1970: 1_791_860_000)

    private func fixture(_ name: String) throws -> Data {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: name, withExtension: "json"))
        return try Data(contentsOf: url)
    }

    private func summary() throws -> Summary { try JSONDecoder().decode(Summary.self, from: fixture("providers-summary")) }
    private func forecast() throws -> Forecast { try JSONDecoder().decode(Forecast.self, from: fixture("sources-forecast")) }

    // MARK: marks

    func testEachProviderWithAMarkHasItInTheApp() {
        let app = Bundle(for: RouterState.self)
        XCTAssertNotNil(UIImage(named: "ProviderAnthropic", in: app, with: nil), "Anthropic's mark is in the asset catalog")
        XCTAssertNotNil(UIImage(named: "ProviderKimi", in: app, with: nil), "Kimi Code's mark is in the asset catalog")
        XCTAssertEqual(ProviderMark.asset("anthropic"), "ProviderAnthropic")
        XCTAssertEqual(ProviderMark.asset(nil), "ProviderAnthropic", "an account from before providers is Anthropic's")
        XCTAssertEqual(ProviderMark.asset("kimi"), "ProviderKimi")
        XCTAssertNil(ProviderMark.asset("openai"), "OpenAI has no mark, on the console or here: its name in words")
        XCTAssertEqual(Provider.name("openai"), "OpenAI")
    }

    // MARK: accounts

    func testAccountsGroupByProviderEachWithItsOwnServingAccount() throws {
        let accounts = try JSONDecoder().decode([Account].self, from: fixture("sources-accounts"))
        let groups = ProviderAccounts.group(accounts)
        XCTAssertEqual(groups.map(\.provider), ["anthropic", "kimi", "openai"], "Anthropic's first, then the console's order")
        XCTAssertEqual(groups.map(\.name), ["Anthropic", "Kimi Code", "OpenAI"])
        XCTAssertEqual(groups[0].accounts.map(\.id).sorted(), ["default", "work"], "an account with no provider is Anthropic's")
        XCTAssertEqual(groups.map { $0.active?.id }, ["work", "kimi-main", "openai-proj"], "each provider's requests go to its own account")
    }

    // MARK: quota blocks

    func testEachProviderHasABlockWithItsWindowsByTheirOwnNames() throws {
        let s = try summary()
        XCTAssertEqual(s.providers.map(\.provider), ["anthropic", "kimi"])
        let kimi = QuotaBlock.make(s.providers[1], live: nil, liveAt: nil, forecast: try forecast(), now: now)
        XCTAssertEqual(kimi.name, "Kimi Code")
        XCTAssertEqual(kimi.serving, "kimi-main")
        XCTAssertEqual(kimi.meters?.map(\.label), ["5-hour window", "Month window"])
        XCTAssertEqual(kimi.meters?.last?.used ?? -1, 0.0553, accuracy: 1e-9)
        XCTAssertEqual(kimi.status, QuotaStatus(word: "allowed", tone: .ok))
        XCTAssertEqual(kimi.source, "checked 3 min ago", "a usage API's reading is checked, not taken from a request")
        XCTAssertEqual(kimi.forecast?.id, "kimi-main", "the burn-downs are the serving account's")
        let anthropic = QuotaBlock.make(s.providers[0], live: nil, liveAt: nil, forecast: try forecast(), now: now)
        XCTAssertEqual(anthropic.serving, "work@example.com · Max", "the serving account by its email, with its plan")
        XCTAssertEqual(anthropic.meters?.map(\.label), ["5-hour window", "7-day window"])
        XCTAssertEqual(anthropic.source, "from the last request 2 min ago", "a header reading is only as fresh as the last request")
    }

    func testAnthropicsBlockPrefersTheLiveReadingOnlyForItsOwnAccount() throws {
        let block = try summary().providers[0]
        let live = #"{"five_hour":{"utilization":60,"resets_at":"@1791876600"},"seven_day":{"utilization":35,"resets_at":"@1792357200"},"severity":"warning","source":"usage-api","account_id":"work","age_ms":120000}"#
        let q = try JSONDecoder().decode(Quota.self, from: Data(live.utf8))
        let b = QuotaBlock.make(block, live: q, liveAt: now, forecast: nil, now: now)
        XCTAssertEqual(b.meters?.first?.used ?? -1, 0.6, accuracy: 1e-9, "the live reading, 0–100, as a fraction")
        XCTAssertEqual(b.status?.tone, .warn)
        XCTAssertEqual(b.source, "checked 2 min ago", "counted back by the reading's own age")
        let other = try JSONDecoder().decode(Quota.self, from: Data(live.replacingOccurrences(of: #""account_id":"work""#, with: #""account_id":"personal""#).utf8))
        let headers = QuotaBlock.make(block, live: other, liveAt: now, forecast: nil, now: now)
        XCTAssertEqual(headers.meters?.first?.used ?? -1, 0.42, accuracy: 1e-9, "a reading of another account never stands for this one")
    }

    func testABlockWithNoReadingAndARolledOverWindowSayWhatTheyAre() throws {
        let empty = #"{"provider":"kimi","account":{"id":"k","label":"k"},"windows":{"five_hour":null,"month":null},"status":null,"at":null,"history":[]}"#
        let none = QuotaBlock.make(try JSONDecoder().decode(ProviderQuota.self, from: Data(empty.utf8)), live: nil, liveAt: nil, forecast: nil, now: now)
        XCTAssertNil(none.meters, "no reading is said as none, never 0%")
        let rolled = #"{"provider":"kimi","account":{"id":"k","label":"k"},"windows":{"five_hour":{"utilization":0.9,"resets_at":1791856000},"month":{"utilization":0.2,"resets_at":1794355200}},"status":"allowed","at":1791855000000,"history":[]}"#
        let b = QuotaBlock.make(try JSONDecoder().decode(ProviderQuota.self, from: Data(rolled.utf8)), live: nil, liveAt: nil, forecast: nil, now: now)
        XCTAssertNil(b.meters?.first?.used, "a window that rolled over shows no figure")
        XCTAssertEqual(b.meters?.first?.expired, "window rolled over 1 h ago · nothing seen since")
        XCTAssertEqual(QuotaStatus.of("rejected")?.tone, .bad)
        XCTAssertEqual(QuotaStatus.of("allowed_warning")?.tone, .warn)
        XCTAssertNil(QuotaStatus.of(nil))
    }

    func testEachProviderHasItsOwnHeadroom() throws {
        let f = try forecast()
        let anthropic = try XCTUnwrap(ProviderRoom.of(f, provider: "anthropic", now: now))
        XCTAssertEqual(anthropic.value, "23 h")
        XCTAssertEqual(anthropic.sub, "2 quota pools, none projected to run out")
        XCTAssertNil(anthropic.first)
        let kimi = try XCTUnwrap(ProviderRoom.of(f, provider: "kimi", now: now))
        XCTAssertEqual(kimi.value, "—", "no hours measured is no figure")
        XCTAssertNil(ProviderRoom.of(f, provider: "openai", now: now), "a provider with no account has no headroom")

        var json = try XCTUnwrap(JSONSerialization.jsonObject(with: fixture("sources-forecast")) as? [String: Any])
        var global = try XCTUnwrap(json["global"] as? [String: Any])
        var providers = try XCTUnwrap(global["providers"] as? [String: Any])
        providers["anthropic"] = ["active_hours_left": 2.5, "pools": 1, "first_to_exhaust": ["account_id": "work", "window": "seven_day", "at": 1_791_867_200_000, "in_ms": 7_200_000]]
        global["providers"] = providers
        json["global"] = global
        let crossing = try JSONDecoder().decode(Forecast.self, from: JSONSerialization.data(withJSONObject: json))
        let room = try XCTUnwrap(ProviderRoom.of(crossing, provider: "anthropic", now: now))
        XCTAssertEqual(room.value, "2.5 h")
        XCTAssertEqual(room.sub, "work's 7-day window is projected to reset first", "the window's own verdict decides the words")
        XCTAssertTrue(room.first?.hasPrefix("First to run out: work's 7-day window, ") == true)
    }

    // MARK: figures

    func testASumNeverSaysZeroForWhatCouldNotBePriced() {
        XCTAssertEqual(Fmt.costSum(3.1).text, "$3.10")
        XCTAssertEqual(Fmt.costSum(3.1, unpriced: 4).text, "$3.10 + 4 unpriced")
        XCTAssertEqual(Fmt.costSum(0, unpriced: 4).text, "not priced")
        XCTAssertEqual(Fmt.costSum(3.1, estimated: 2).text, "≈$3.10")
        XCTAssertEqual(Fmt.costSum(0, estimated: 2).text, "$0", "an estimate moves only dollars there are")
        XCTAssertEqual(Fmt.costSum(0).text, "$0", "a zero the router measured")
        XCTAssertEqual(Fmt.costSum(nil).text, "—")
        XCTAssertEqual(Fmt.requestCost(nil, promptTokens: 1200, outputTokens: 40).text, "not priced")
        XCTAssertEqual(Fmt.requestCost(nil, promptTokens: nil, outputTokens: nil).text, "—")
        XCTAssertEqual(Fmt.costNote([Fmt.costSum(1, unpriced: 2)]), "Unpriced: \(Fmt.unpricedWhy).")
        XCTAssertNil(Fmt.costNote([Fmt.costSum(1)]))
    }

    func testTotalsSplitByProviderOnlyWhenTwoServed() throws {
        let t = try summary().totals
        XCTAssertEqual(t.unpriced, 4)
        let prompt = try XCTUnwrap(ProviderSplit.parts(t.byProvider, .prompt))
        XCTAssertEqual(prompt.map(\.provider), ["anthropic", "kimi"])
        XCTAssertEqual(prompt.map(\.figure), ["2.40M", "600k"])
        XCTAssertEqual(prompt[0].share ?? 0, 0.8, accuracy: 1e-9)
        let spend = try XCTUnwrap(ProviderSplit.parts(t.byProvider, .spend))
        XCTAssertEqual(spend.map(\.figure), ["$12.50", "not priced"])
        XCTAssertNil(spend[1].share, "nothing priced has no share of the dollars")
        XCTAssertNil(ProviderSplit.parts(Array(t.byProvider.prefix(1)), .prompt), "one provider shows no split")
    }

    func testTheScopeOffersAProviderOnlyWithTwoToChooseBetween() {
        XCTAssertNil(ProviderSplit.choices(withAccount: ["anthropic", "anthropic"], chosen: nil))
        XCTAssertEqual(ProviderSplit.choices(withAccount: ["openai", "kimi", "anthropic"], chosen: nil), [nil, "anthropic", "kimi"],
                       "every provider, then each the scope can name, Anthropic's first; OpenAI as on the console")
        XCTAssertEqual(ProviderSplit.choices(withAccount: ["anthropic"], chosen: "kimi"), [nil, "anthropic", "kimi"], "the one chosen stays, to be let go")
        XCTAssertEqual(UsageScope(provider: "kimi").title, "Kimi Code")
    }

    func testEveryRowNamesItsProvider() throws {
        let s = try summary()
        XCTAssertEqual(s.models.map(\.provider), ["anthropic", "kimi"])
        XCTAssertEqual(s.byAccount.map(\.provider), ["anthropic", "kimi"])
        XCTAssertEqual(s.byAccount[1].unpriced, 4)
        let session = #"{"id":"s1","project":"agent-router","started_at":1,"last_seen_at":2,"requests":3,"prompt_tokens":4,"cost":0,"output_tokens":5,"cache_read_tokens":6,"subagent_tokens":0,"errors":0,"subagents":0,"providers":["anthropic","kimi"],"unpriced":2,"estimated":0,"client":"codex-managed"}"#
        let row = try JSONDecoder().decode(ProxySessionRow.self, from: Data(session.utf8))
        XCTAssertEqual(row.providers, ["anthropic", "kimi"], "a mark for each provider its requests went to")
        XCTAssertEqual(row.client, "Codex managed")
        XCTAssertEqual(Fmt.costSum(row.cost, unpriced: row.unpriced).text, "not priced")
        let request = #"{"id":7,"model":"k3","provider":"kimi","account_id":"kimi-main","started_at":1,"prompt_tokens":100,"output_tokens":5,"cost":null}"#
        let r = try JSONDecoder().decode(RequestRow.self, from: Data(request.utf8))
        XCTAssertEqual(r.provider, "kimi")
        let accounts = try JSONDecoder().decode([Account].self, from: fixture("sources-accounts"))
        XCTAssertEqual(r.accountWords(accounts), "kimi-main", "an account named by its id alone says it once")
        let work = try JSONDecoder().decode(RequestRow.self, from: Data(request.replacingOccurrences(of: "kimi-main", with: "work").utf8))
        XCTAssertEqual(work.accountWords(accounts), "work@example.com · work")
        let gone = try JSONDecoder().decode(RequestRow.self, from: Data(request.replacingOccurrences(of: "kimi-main", with: "removed").utf8))
        XCTAssertEqual(gone.accountWords(accounts), "removed")
    }
}
