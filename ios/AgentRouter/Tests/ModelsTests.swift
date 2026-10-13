import XCTest
@testable import AgentRouter

/// Settings → Models on the phone, as the console has it (AGEN-0179): the catalogue with the
/// account each model's next request goes to, the models a registered route stands for left to
/// the routes, a choice of the model's own provider's accounts that can sign here, and the
/// request that assigns one. The fixtures are the router's replies with the identities replaced.
final class ModelsTests: XCTestCase {
    private func fixture<T: Decodable>(_ name: String, as: T.Type) throws -> T {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: name, withExtension: "json"))
        return try JSONDecoder().decode(T.self, from: Data(contentsOf: url))
    }

    private func catalogue() throws -> [CatalogueModel] { try fixture("kimi-models", as: [CatalogueModel].self) }
    private func accounts() throws -> [Account] { try fixture("sources-accounts", as: [Account].self) }

    func testTheCatalogueDecodesWithEachModelsServingAccount() throws {
        let c = try catalogue()
        XCTAssertEqual(c.map(\.provider), ["anthropic", "kimi", "kimi", "openai"])
        XCTAssertEqual(c[1].name, "Kimi K3")
        XCTAssertEqual(c[2].assigned, "kimi-main")
        XCTAssertEqual(ModelChoices.servedBy(c[1], accounts: try accounts()), "kimi-main, Kimi Code’s active account")
        XCTAssertEqual(ModelChoices.servedBy(c[2], accounts: try accounts()), "kimi-main, assigned to it")
        XCTAssertEqual(ModelChoices.servedBy(c[0], accounts: try accounts()), "work@example.com, Anthropic’s active account")
        let none = CatalogueModel(id: "k3", provider: "kimi", displayName: nil, assigned: nil, serving: nil)
        XCTAssertEqual(ModelChoices.servedBy(none, accounts: []), "nothing: Kimi Code has no active account")
    }

    func testAModelARouteStandsForIsLeftToTheRoute() throws {
        let registry = try fixture("provider-registry", as: ProviderRegistry.self)
        XCTAssertEqual(ModelChoices.listed(try catalogue(), registry: registry).map(\.id), ["claude-opus-5-5", "k3", "kimi-for-coding"])
        XCTAssertEqual(ModelChoices.listed(try catalogue(), registry: nil).count, 4, "with no registry read, every model")
    }

    func testTheChoiceOffersOnlyTheModelsOwnProvidersAccountsThatCanSign() throws {
        let opus = try catalogue()[0]
        let hosted = ModelChoices.choices(opus, accounts: try accounts(), hosted: true)
        XCTAssertEqual(hosted.map(\.label), ["Anthropic’s active account (work@example.com)", "work@example.com"],
                       "a hosted router signs with a pasted key only, and never with another provider's account")
        XCTAssertNil(hosted[0].account, "the first choice is the provider's active account, whichever it is")
        let local = ModelChoices.choices(opus, accounts: try accounts(), hosted: false)
        XCTAssertEqual(local.map(\.account), [nil, "work", "default"], "a local router signs with its keychain's too")
        let stale = CatalogueModel(id: "k3", provider: "kimi", displayName: "Kimi K3", assigned: "gone", serving: "gone")
        XCTAssertEqual(ModelChoices.choices(stale, accounts: try accounts(), hosted: true).map(\.account), [nil, "kimi-main", "gone"],
                       "the account it is assigned now is offered, so the choice shows what the router reports")
    }

    func testAnAssignmentSendsItsAccountAndNullForTheProvidersOwn() throws {
        let to = String(decoding: try JSONEncoder().encode(AssignBody(model: "k3", account: "kimi-main")), as: UTF8.self)
        XCTAssertTrue(to.contains(#""account":"kimi-main""#) && to.contains(#""model":"k3""#), to)
        let back = String(decoding: try JSONEncoder().encode(AssignBody(model: "k3", account: nil)), as: UTF8.self)
        XCTAssertTrue(back.contains(#""account":null"#), "null is sent, not left out: \(back)")
        let k3 = try catalogue()[1]
        XCTAssertEqual(ModelChoices.assignedWords(k3, .init(account: nil, label: "x")), "Kimi K3 goes to Kimi Code’s active account again.")
        XCTAssertEqual(ModelChoices.assignedWords(k3, .init(account: "kimi-main", label: "kimi-main")), "Kimi K3 goes to kimi-main now.")
    }
}
