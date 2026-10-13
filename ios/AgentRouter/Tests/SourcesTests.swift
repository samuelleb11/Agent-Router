import XCTest
@testable import AgentRouter

/// A router's sources reach the phone: a Kimi Code account with its provider and its address,
/// each provider's rule for a key, the session picker by provider, Kimi Code's second window by
/// its own name, and Kimi Code's models as their own family (AGEN-0093). The fixtures are the
/// router's replies with the identities replaced.
final class SourcesTests: XCTestCase {
    private func fixture(_ name: String) throws -> Data {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: name, withExtension: "json"))
        return try Data(contentsOf: url)
    }

    func testAKimiCodeAccountDecodesWithItsProviderAndAddress() throws {
        let accounts = try JSONDecoder().decode([Account].self, from: fixture("sources-accounts"))
        let byId = Dictionary(uniqueKeysWithValues: accounts.map { ($0.id, $0) })
        let kimi = try XCTUnwrap(byId["kimi-main"])
        XCTAssertEqual(kimi.provider, "kimi")
        XCTAssertEqual(kimi.providerName, "Kimi Code")
        XCTAssertEqual(kimi.baseURL, "https://api.kimi.ai/coding")
        XCTAssertFalse(kimi.takesUsageKey, "a claude.ai session key reads Anthropic's usage alone")
        XCTAssertEqual(byId["openai-proj"]?.providerName, "OpenAI")
        XCTAssertEqual(byId["openai-proj"]?.endpoint, "https://api.openai.com", "no address of its own is the provider's")
        XCTAssertNil(byId["work"]?.endpoint, "an Anthropic account names no address")
        XCTAssertEqual(byId["default"]?.providerName, "Anthropic", "an account from before providers is Anthropic's")
    }

    func testAddingAKimiCodeAccountSendsItsProviderKeyAndAddress() throws {
        XCTAssertEqual(Provider.addable, ["anthropic", "kimi", "openai"], "the add form offers Kimi Code, in the console's order")
        XCTAssertEqual(Provider.addable.map(Provider.name), ["Anthropic", "Kimi Code", "OpenAI"])
        let body = AddAccountBody(label: "kimi-main", source: "token", token: "kimi-key", provider: "kimi", baseURL: "https://kimi.example.com/coding")
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(body)) as? [String: String])
        XCTAssertEqual(fields, ["label": "kimi-main", "source": "token", "token": "kimi-key", "provider": "kimi", "base_url": "https://kimi.example.com/coding"])
        let own = AddAccountBody(label: "kimi-main", source: "token", token: "kimi-key", provider: "kimi")
        let ownFields = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(own)) as? [String: String])
        XCTAssertNil(ownFields["base_url"], "the provider's own address is not sent")
    }

    func testEachProviderKeepsItsOwnRuleForAKey() {
        XCTAssertNil(Provider.keyProblem("kimi", "sk-kimi-abc"))
        XCTAssertNotNil(Provider.keyProblem("kimi", "sk-ant-oat01-abc"), "an Anthropic token on a Kimi Code account")
        XCTAssertNil(Provider.keyProblem("openai", "sk-proj-abc"))
        XCTAssertNotNil(Provider.keyProblem("openai", "sk-ant-oat01-abc"))
        XCTAssertNotNil(Provider.keyProblem("openai", "not-a-key"))
        XCTAssertNil(Provider.keyProblem("anthropic", "sk-ant-oat01-abc"))
        XCTAssertNotNil(Provider.keyProblem("anthropic", "sk-ant-sid01-abc"), "a session key cannot sign")
        XCTAssertNotNil(Provider.keyProblem("anthropic", "sk-proj-abc"))
        XCTAssertNil(Provider.keyProblem("openai", "  "), "blank is the form's to ask for, not a rule's")

        XCTAssertNil(Provider.baseURLProblem(""))
        XCTAssertNil(Provider.baseURLProblem("https://kimi.example.com/coding/"))
        XCTAssertEqual(Provider.trimmedBaseURL(" https://kimi.example.com/coding// "), "https://kimi.example.com/coding")
        XCTAssertEqual(Provider.baseURLProblem("http://kimi.example.com"), "\"http://kimi.example.com\" is not an https:// address")
        XCTAssertNotNil(Provider.baseURLProblem("kimi.example.com"))
    }

    func testTheSessionPickerIsGroupedByProvider() throws {
        let o = try JSONDecoder().decode(LiveOptions.self, from: fixture("sources-options"))
        XCTAssertEqual(o.byProvider.map(\.name), ["Anthropic", "Kimi Code"])
        let kimi = try XCTUnwrap(o.byProvider.last)
        XCTAssertEqual(kimi.provider, "kimi")
        XCTAssertEqual(kimi.models.first?.id, "k3")
        XCTAssertEqual(kimi.models.first?.displayName, "Kimi K3", "each model by the name its provider gives it")
        XCTAssertEqual(Set(o.byProvider.flatMap { $0.models.map(\.id) }), Set(o.models), "the groups hold every model on offer")
    }

    func testAKimiCodeAccountsSecondWindowIsNamedAsItsOwn() throws {
        let f = try JSONDecoder().decode(Forecast.self, from: fixture("sources-forecast"))
        let kimi = try XCTUnwrap(f.accounts.first { $0.provider == "kimi" })
        XCTAssertEqual(kimi.secondKey, .month)
        XCTAssertEqual(kimi.windows.map(\.key.label), ["5-hour window", "Month window"])
        XCTAssertEqual(kimi.secondKey.short, "mo")
        XCTAssertNotNil(kimi.second?.used, "a Kimi Code reading is a reading, never 'no rate-limit reading yet'")
        XCTAssertEqual(kimi.second?.spent, true)
        let work = try XCTUnwrap(f.accounts.first { $0.id == "work" })
        XCTAssertEqual(work.providerId, "anthropic")
        XCTAssertEqual(work.windows.map(\.key.label), ["5-hour window", "7-day window"])
        XCTAssertEqual(WindowKey.month.name, "month")
    }

    func testKimiCodeModelsAreAFamilyOfTheirOwn() {
        for m in ["k3", "k3-256k", "kimi-for-coding", "kimi-for-coding-highspeed"] {
            XCTAssertEqual(Bucket.family(m), "kimi", m)
        }
        XCTAssertEqual(Bucket.family("claude-opus-5-5"), "opus")
        XCTAssertEqual(Bucket.family("claude-haiku-5-5"), "haiku")
        XCTAssertEqual(Bucket.family("gpt-5.1-codex"), "other", "OpenAI's models have no family, as on the console")
        XCTAssertEqual(Bucket.family("k30"), "other")
        XCTAssertEqual(FamilyColor.order, ["opus", "sonnet", "haiku", "kimi", "other"])
    }
}
