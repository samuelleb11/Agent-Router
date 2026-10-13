// Settings → Models, as the console has it (ui/src/screens/Settings/Models.tsx, rules.ts
// `modelChoices`, `servedBy`, `assignedWords`): every model in the router's catalogue with the
// account its next request goes to, and the choice of another account of its own provider. A
// client picks an account by picking a model, so this is where that is decided.

import Foundation
import Observation

/// `GET /api/provider-registry`: the services the router can send to and the model routes
/// registered on them (src/provider_registry.rs).
struct ProviderRegistry: Decodable, Sendable, Equatable {
    let services: [Service]
    let models: [Route]

    struct Service: Codable, Sendable, Equatable, Identifiable {
        let id: String
        let label: String
        let provider: String
        let baseURL: String
        /// `messages` or `responses`.
        let `protocol`: String
        /// `bearer` or `api_key`.
        let authProfile: String
        let capabilities: Capabilities?
        /// Built into the router: neither edited nor removed.
        let builtin: Bool?
        enum CodingKeys: String, CodingKey {
            case id, label, provider, capabilities, builtin, `protocol`
            case baseURL = "base_url", authProfile = "auth_profile"
        }
    }

    struct Route: Codable, Sendable, Equatable, Identifiable {
        /// The id a client asks for.
        let alias: String
        let service: String
        let upstreamModel: String
        let accountId: String?
        let displayName: String?
        let capabilities: Capabilities?
        var id: String { alias }
        enum CodingKeys: String, CodingKey {
            case alias, service, capabilities
            case upstreamModel = "upstream_model", accountId = "account_id", displayName = "display_name"
        }
    }

    struct Capabilities: Codable, Sendable, Equatable {
        var tools: Bool
        var images: Bool
        var reasoning: Bool
        var compaction: Bool
        var contextTokens: Int?
        var outputTokens: Int?
        enum CodingKeys: String, CodingKey {
            case tools, images, reasoning, compaction
            case contextTokens = "context_tokens", outputTokens = "output_tokens"
        }
    }
}

/// `POST /api/models/assign`: the account a model goes to, null for its provider's active one.
struct AssignBody: Encodable, Sendable {
    let model: String
    let account: String?
    enum CodingKeys: String, CodingKey { case model, account }
    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(model, forKey: .model)
        // Null is a choice of its own: back to the provider's active account.
        try c.encode(account, forKey: .account)
    }
}

enum ModelChoices {
    struct Choice: Identifiable, Equatable, Sendable {
        /// The account, nil for the provider's active one.
        let account: String?
        let label: String
        var id: String { account ?? "" }
    }

    /// An account as the page names it: its email, else its label, else its id.
    static func accountName(_ a: Account) -> String { a.email ?? (a.label.isEmpty ? a.id : a.label) }

    /// The models the page lists: every model of the catalogue but those a registered route
    /// stands for, which the routes below them list instead.
    static func listed(_ models: [CatalogueModel], registry: ProviderRegistry?) -> [CatalogueModel] {
        let aliases = Set(registry?.models.map(\.alias) ?? [])
        return models.filter { !aliases.contains($0.id) }
    }

    /// What a model's choice offers: its provider's active account first, naming which one that
    /// is now; then each account of the model's own provider that can sign on this router — the
    /// router refuses another provider's, and a hosted router signs with a pasted key only — and
    /// whatever account it is assigned now, so the choice always shows what the router reports.
    static func choices(_ model: CatalogueModel, accounts: [Account], hosted: Bool) -> [Choice] {
        let own = accounts.filter { ($0.provider ?? "anthropic") == model.provider }
        let active = own.first(where: \.isActive)
        let first = Choice(account: nil, label: "\(Provider.name(model.provider))’s active account" + (active.map { " (\(accountName($0)))" } ?? ""))
        var rest = own.filter { !hosted || $0.source == "token" || $0.id == model.assigned }.map { Choice(account: $0.id, label: accountName($0)) }
        if let assigned = model.assigned, !rest.contains(where: { $0.account == assigned }) { rest.append(Choice(account: assigned, label: assigned)) }
        return [first] + rest
    }

    /// Who a model's next request goes to: the account it was assigned, else its provider's
    /// active account, by name; or that its provider has none.
    static func servedBy(_ model: CatalogueModel, accounts: [Account]) -> String {
        let provider = Provider.name(model.provider)
        guard let serving = model.serving else { return "nothing: \(provider) has no active account" }
        let name = accounts.first { $0.id == serving }.map(accountName) ?? serving
        return model.assigned != nil ? "\(name), assigned to it" : "\(name), \(provider)’s active account"
    }

    /// What an assignment did, once the router took it.
    static func assignedWords(_ model: CatalogueModel, _ choice: Choice) -> String {
        choice.account == nil ? "\(model.name) goes to \(Provider.name(model.provider))’s active account again." : "\(model.name) goes to \(choice.label) now."
    }
}

@MainActor
@Observable
final class ModelsState {
    let router: RouterState
    var models: [CatalogueModel]?
    var accounts: [Account] = []
    var registry: ProviderRegistry?
    var failure: String?
    /// What the last assignment did, or why the router refused it, by model.
    var notice: [String: (text: String, ok: Bool)] = [:]
    var busy: String?

    init(router: RouterState) { self.router = router }

    var hosted: Bool { router.auth?.isHosted == true }
    var listed: [CatalogueModel] { ModelChoices.listed(models ?? [], registry: registry) }

    func load() async {
        guard let client = router.liveClient else { return }
        async let m: [CatalogueModel] = client.get("/api/models")
        async let a: [Account] = client.get("/api/accounts")
        async let r: ProviderRegistry = client.get("/api/provider-registry")
        do { models = try await m; failure = nil } catch { failure = LiveFeed.describe(error) }
        do { accounts = try await a } catch { failure = failure ?? LiveFeed.describe(error) }
        registry = try? await r
    }

    func assign(_ model: CatalogueModel, to choice: ModelChoices.Choice) async {
        guard let client = router.liveClient else { return }
        busy = model.id
        defer { busy = nil }
        do {
            let reply: OkReply = try await client.post("/api/models/assign", AssignBody(model: model.id, account: choice.account))
            if let p = reply.problem { notice[model.id] = (p, false) } else { notice[model.id] = (ModelChoices.assignedWords(model, choice), true) }
        } catch {
            notice[model.id] = (LiveFeed.describe(error), false)
        }
        await load()
    }
}
