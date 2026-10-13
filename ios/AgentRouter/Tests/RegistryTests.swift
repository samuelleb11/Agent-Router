import XCTest
@testable import AgentRouter

/// Provider services and model routes on the phone, as the console has them (AGEN-0184): the
/// registry decoded, built-in services read-only, a service and a route that break the console's
/// rules refused before sending, and the save and delete requests. The fixtures are the router's
/// replies with the identities replaced.
final class RegistryTests: XCTestCase {
    private func fixture<T: Decodable>(_ name: String, as: T.Type) throws -> T {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: name, withExtension: "json"))
        return try JSONDecoder().decode(T.self, from: Data(contentsOf: url))
    }
    private func registry() throws -> ProviderRegistry { try fixture("provider-registry", as: ProviderRegistry.self) }
    private func accounts() throws -> [Account] { try fixture("sources-accounts", as: [Account].self) }
    private let caps = ProviderRegistry.Capabilities(tools: true, images: true, reasoning: true, compaction: false, contextTokens: nil, outputTokens: nil)

    func testTheRegistryDecodesWithItsBuiltInServicesReadOnly() throws {
        let r = try registry()
        XCTAssertEqual(r.services.map(\.id), ["anthropic", "openai", "kimi"])
        XCTAssertTrue(r.services.allSatisfy { $0.builtin == true })
        XCTAssertFalse(r.services.contains(where: Registry.editable), "a built-in service is neither edited nor removed")
        XCTAssertEqual(r.services.map { Registry.protocolName($0.protocol) }, ["Anthropic Messages", "OpenAI Responses", "Anthropic Messages"])
        XCTAssertEqual(Registry.authName(r.services[2].authProfile), "x-api-key")
        XCTAssertEqual(r.models.map(\.alias), ["gpt-5.1-codex", "house-k3"])
        XCTAssertEqual(r.models[1].capabilities?.contextTokens, 262144)
        let custom = ProviderRegistry.Service(id: "proxy", label: "Proxy", provider: "openai", baseURL: "https://proxy.example.com", protocol: "responses", authProfile: "bearer", capabilities: caps, builtin: false)
        XCTAssertTrue(Registry.removable(custom, in: r))
        let used = ProviderRegistry(services: r.services + [custom], models: r.models + [.init(alias: "p", service: "proxy", upstreamModel: "x", accountId: "openai-proj", displayName: nil, capabilities: nil)])
        XCTAssertFalse(Registry.removable(custom, in: used), "a service its routes use stays until they go")
    }

    func testTheConsolesRulesForAServiceAreCheckedBeforeSending() throws {
        XCTAssertNil(Registry.serviceURLProblem("https://proxy.example.com"))
        XCTAssertNil(Registry.serviceURLProblem("https://proxy.example.com/api/"))
        for bad in ["http://proxy.example.com", "https://user:pw@proxy.example.com", "https://proxy.example.com/v1", "https://proxy.example.com/v1/", "https://proxy.example.com?x=1", "https://proxy.example.com#a", "proxy.example.com"] {
            XCTAssertEqual(Registry.serviceURLProblem(bad), Registry.urlRule, bad)
        }
        XCTAssertTrue(Registry.serviceIdValid("my-proxy_2"))
        XCTAssertFalse(Registry.serviceIdValid("My Proxy"))
        let r = try registry()
        let good = ServiceInput(id: "proxy", label: "Proxy", provider: "openai", baseURL: "https://proxy.example.com", protocol: "responses", authProfile: "bearer", capabilities: caps)
        XCTAssertNil(Registry.serviceProblem(good, existing: r, editing: false))
        let taken = ServiceInput(id: "openai", label: "x", provider: "openai", baseURL: "https://proxy.example.com", protocol: "responses", authProfile: "bearer", capabilities: caps)
        XCTAssertNotNil(Registry.serviceProblem(taken, existing: r, editing: false), "an id that exists is edited from its card")
        var compacting = caps; compacting.compaction = true
        let messages = ServiceInput(id: "m", label: "M", provider: "anthropic", baseURL: "https://m.example.com", protocol: "messages", authProfile: "api_key", capabilities: compacting)
        XCTAssertEqual(Registry.serviceProblem(messages, existing: r, editing: false), "Compaction requires a native Responses route.")
        XCTAssertNil(Registry.tokenLimitProblem(""))
        XCTAssertNil(Registry.tokenLimitProblem("200000"))
        XCTAssertEqual(Registry.tokenLimitProblem("0"), Registry.limitRule)
        XCTAssertEqual(Registry.tokenLimitProblem("12k"), Registry.limitRule)
    }

    func testARoutesAccountMustBeItsServicesProviderAndAPastedKey() throws {
        let r = try registry(), a = try accounts()
        let anthropic = r.services[0]
        XCTAssertEqual(Registry.eligible(a, service: anthropic, hosted: true).map(\.id), ["work"], "a hosted router signs with pasted keys only")
        XCTAssertEqual(Registry.eligible(a, service: anthropic, hosted: false).map(\.id), ["work", "default"], "a local built-in service signs with its native logins too")
        let custom = ProviderRegistry.Service(id: "proxy", label: "Proxy", provider: "anthropic", baseURL: "https://proxy.example.com", protocol: "messages", authProfile: "api_key", capabilities: caps, builtin: false)
        XCTAssertEqual(Registry.eligible(a, service: custom, hosted: false).map(\.id), ["work"], "a custom service never gets a native login")
        let route = RouteInput(alias: "house-opus", service: "anthropic", upstreamModel: "claude-opus-5-5", accountId: "kimi-main", displayName: nil, capabilities: nil)
        XCTAssertNotNil(Registry.routeProblem(route, registry: r, accounts: a, hosted: true, editing: false), "another provider's account")
        let ok = RouteInput(alias: "house-opus", service: "anthropic", upstreamModel: "claude-opus-5-5", accountId: "work", displayName: nil, capabilities: nil)
        XCTAssertNil(Registry.routeProblem(ok, registry: r, accounts: a, hosted: true, editing: false))
        let dup = RouteInput(alias: "house-k3", service: "kimi", upstreamModel: "k3", accountId: "kimi-main", displayName: nil, capabilities: nil)
        XCTAssertNotNil(Registry.routeProblem(dup, registry: r, accounts: a, hosted: true, editing: false))
        XCTAssertNil(Registry.routeProblem(dup, registry: r, accounts: a, hosted: true, editing: true), "editing keeps its alias")
        XCTAssertFalse(Registry.nameValid("has space"))
        XCTAssertTrue(Registry.nameValid("org/model:v1.2"))
    }

    func testSavesAndDeletesAreBuiltAsTheConsoleSendsThem() throws {
        let route = RouteInput(alias: "house-k3", service: "kimi", upstreamModel: "k3", accountId: "kimi-main", displayName: nil, capabilities: nil)
        let json = String(decoding: try JSONEncoder().encode(route), as: UTF8.self)
        XCTAssertTrue(json.contains(#""display_name":null"#) && json.contains(#""capabilities":null"#), "null is sent, not left out: \(json)")
        XCTAssertTrue(json.contains(#""upstream_model":"k3""#) && json.contains(#""account_id":"kimi-main""#))
        let service = ServiceInput(id: "proxy", label: "Proxy", provider: "openai", baseURL: "https://proxy.example.com", protocol: "responses", authProfile: "bearer", capabilities: caps)
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(service)) as? [String: Any])
        XCTAssertEqual(fields["base_url"] as? String, "https://proxy.example.com")
        XCTAssertEqual(fields["auth_profile"] as? String, "bearer")
        XCTAssertNotNil(fields["capabilities"] as? [String: Any])
        XCTAssertEqual("/api/provider-registry/models/" + RouterClient.segment("org/model:v1"), "/api/provider-registry/models/org%2Fmodel%3Av1", "an alias with a slash stays one segment")
        XCTAssertEqual(Registry.capabilityWords(nil), "Inherits the service’s configured capabilities.")
    }
}
