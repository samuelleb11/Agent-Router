// Provider services and model routes, under Settings → Models as the console has them
// (ui/src/screens/Settings/Registry.tsx, ui/src/model/provider-registry.ts): a service says where
// requests go, the API it speaks and how an account authenticates; a route gives a model a
// client-facing name, an exact upstream model, a service and an account. Built-in services are
// read-only, and the console's rules are checked before anything is sent.

import Foundation
import Observation

/// `POST /api/provider-registry/services`.
struct ServiceInput: Encodable, Sendable, Equatable {
    let id: String
    let label: String
    let provider: String
    let baseURL: String
    let `protocol`: String
    let authProfile: String
    let capabilities: ProviderRegistry.Capabilities
    enum CodingKeys: String, CodingKey {
        case id, label, provider, capabilities, `protocol`
        case baseURL = "base_url", authProfile = "auth_profile"
    }
}

/// `POST /api/provider-registry/models`: a route, its display name and its own capabilities
/// null when it has none of its own.
struct RouteInput: Encodable, Sendable, Equatable {
    let alias: String
    let service: String
    let upstreamModel: String
    let accountId: String
    let displayName: String?
    let capabilities: ProviderRegistry.Capabilities?
    enum CodingKeys: String, CodingKey {
        case alias, service, capabilities
        case upstreamModel = "upstream_model", accountId = "account_id", displayName = "display_name"
    }
    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(alias, forKey: .alias)
        try c.encode(service, forKey: .service)
        try c.encode(upstreamModel, forKey: .upstreamModel)
        try c.encode(accountId, forKey: .accountId)
        try c.encode(displayName, forKey: .displayName)
        try c.encode(capabilities, forKey: .capabilities)
    }
}

enum Registry {
    static let routesExplain = "Give a model a client-facing name, an exact upstream model ID, a provider service and an account. Clients select the alias; requests use the model and account saved here."
    static let servicesExplain = "A service defines where its model requests go, the API it speaks and how an account authenticates. Provider credentials are managed in Accounts."
    static let capabilitiesExplain = "Configured capabilities are your settings, not a provider entitlement check. Unknown context and output limits remain unknown until you configure them."
    static let builtinNote = "Built-in service. Add a separate service for another endpoint."
    static let inUseNote = "Remove this service’s model routes before removing the service."
    static let customAccounts = "Custom services use API-key accounts. Subscription tokens and native client logins cannot be sent to another endpoint."
    static let noEligible = "Add a compatible API-key account in Accounts before saving this route."
    static let urlRule = "Use an HTTPS base URL without credentials, query, fragment or a final /v1."
    static let nameRule = "Use up to 200 letters, digits, underscores, hyphens, dots, slashes or colons."
    static let idRule = "Use lowercase letters, digits, underscores or hyphens."
    static let limitRule = "Use a whole number between 1 and 100,000,000, or leave blank for an unknown limit."

    static func protocolName(_ p: String) -> String { p == "responses" ? "OpenAI Responses" : "Anthropic Messages" }
    static func authName(_ a: String) -> String { a == "api_key" ? "x-api-key" : "Authorization: Bearer" }

    static func serviceIdValid(_ id: String) -> Bool { id.range(of: #"^[a-z0-9_-]{1,64}$"#, options: .regularExpression) != nil }
    static func nameValid(_ v: String) -> Bool { v.range(of: #"^[a-zA-Z0-9_.:/-]{1,200}$"#, options: .regularExpression) != nil }

    /// An HTTPS origin or API prefix, with no credentials, query, fragment or final `/v1`.
    static func serviceURLProblem(_ value: String) -> String? {
        guard let url = URL(string: value), url.scheme == "https", let host = url.host, !host.isEmpty,
              url.user == nil, url.password == nil, !value.contains("?"), !value.contains("#") else { return urlRule }
        var path = url.path
        while path.hasSuffix("/") { path.removeLast() }
        return path.hasSuffix("/v1") ? urlRule : nil
    }

    /// A token limit typed: blank for unknown, else a whole number in range.
    static func tokenLimitProblem(_ value: String) -> String? {
        let v = value.trimmingCharacters(in: .whitespaces)
        if v.isEmpty { return nil }
        guard v.allSatisfy(\.isNumber), let n = Int(v), (1...100_000_000).contains(n) else { return limitRule }
        return nil
    }

    /// The accounts a route on this service may sign with: of the service's provider, and pasted
    /// keys only, but for a built-in service on a local router, which signs with its native logins.
    static func eligible(_ accounts: [Account], service: ProviderRegistry.Service?, hosted: Bool) -> [Account] {
        guard let service else { return [] }
        return accounts.filter { ($0.provider ?? "anthropic") == service.provider && ((!hosted && service.builtin == true) || $0.source == "token") }
    }

    /// A service may be edited and removed when it is not built in; removed once no route uses it.
    static func editable(_ s: ProviderRegistry.Service) -> Bool { s.builtin != true }
    static func removable(_ s: ProviderRegistry.Service, in registry: ProviderRegistry) -> Bool {
        editable(s) && !registry.models.contains { $0.service == s.id }
    }

    /// Whether a service form may be sent. Compaction is a native Responses route's alone.
    static func serviceProblem(_ input: ServiceInput, existing: ProviderRegistry?, editing: Bool) -> String? {
        if !serviceIdValid(input.id) { return idRule }
        if !editing, existing?.services.contains(where: { $0.id == input.id }) == true { return "This service already exists. Edit it from its card." }
        if input.label.trimmingCharacters(in: .whitespaces).isEmpty { return "Name the service." }
        if let p = serviceURLProblem(input.baseURL) { return p }
        if input.capabilities.compaction && input.protocol != "responses" { return "Compaction requires a native Responses route." }
        return nil
    }

    /// Whether a route form may be sent.
    static func routeProblem(_ input: RouteInput, registry: ProviderRegistry, accounts: [Account], hosted: Bool, editing: Bool) -> String? {
        if !nameValid(input.alias) || !nameValid(input.upstreamModel) { return nameRule }
        if !editing, registry.models.contains(where: { $0.alias == input.alias }) { return "This route already exists. Edit it from its card." }
        let service = registry.services.first { $0.id == input.service }
        guard let service else { return "Choose a provider service." }
        let ok = eligible(accounts, service: service, hosted: hosted)
        if ok.isEmpty { return noEligible }
        if !ok.contains(where: { $0.id == input.accountId }) { return "Choose an account of \(Provider.name(service.provider)) that can sign for this service." }
        if input.capabilities?.compaction == true && service.protocol != "responses" { return "Compaction requires a native Responses route." }
        return nil
    }

    /// A capability set in words.
    static func capabilityWords(_ c: ProviderRegistry.Capabilities?) -> String {
        guard let c else { return "Inherits the service’s configured capabilities." }
        let on = { (b: Bool) in b ? "allowed" : "disabled" }
        return "Context \(c.contextTokens.map { $0.formatted() } ?? "unknown") tokens; output \(c.outputTokens.map { $0.formatted() } ?? "unknown") tokens. Tools \(on(c.tools)), images \(on(c.images)), reasoning \(on(c.reasoning)), compaction \(on(c.compaction))."
    }
}

@MainActor
@Observable
final class RegistryState {
    let router: RouterState
    var registry: ProviderRegistry?
    var accounts: [Account] = []
    var failure: String?
    var done: String?

    init(router: RouterState) { self.router = router }
    var hosted: Bool { router.auth?.isHosted == true }

    func load() async {
        guard let client = router.liveClient else { return }
        async let r: ProviderRegistry = client.get("/api/provider-registry")
        async let a: [Account] = client.get("/api/accounts")
        do { registry = try await r; failure = nil } catch { failure = LiveFeed.describe(error) }
        if let read = try? await a { accounts = read }
    }

    /// Nil when the router took it, else its words for refusing.
    func save(_ input: ServiceInput) async -> String? { await change { try await $0.post("/api/provider-registry/services", input) } }
    func save(_ input: RouteInput) async -> String? { await change { try await $0.post("/api/provider-registry/models", input) } }
    func remove(service id: String) async -> String? { await change { try await $0.delete("/api/provider-registry/services/" + RouterClient.segment(id)) } }
    func remove(route alias: String) async -> String? { await change { try await $0.delete("/api/provider-registry/models/" + RouterClient.segment(alias)) } }

    private func change(_ op: (RouterClient) async throws -> OkReply) async -> String? {
        guard let client = router.liveClient else { return "bad address" }
        do {
            let reply = try await op(client)
            await load()
            return reply.problem
        } catch {
            return LiveFeed.describe(error)
        }
    }
}
