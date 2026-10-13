// Settings → Clients, as the console has it (ui/src/model/client-setup.ts,
// ui/src/screens/Settings/Clients.tsx): Claude Code, Codex and OpenCode set up to reach the
// router with an exact model and a router credential — previewed, checked, applied on a local
// router, restored, or shared as a file to install on the client's machine.

import Foundation
import Observation

/// `GET /api/clients`.
struct ClientSetupReply: Decodable, Sendable, Equatable {
    /// `local` or `server`.
    let mode: String
    let url: String
    let clients: [SetupClient]
    let models: [SetupModel]
}

struct SetupClient: Decodable, Sendable, Equatable, Identifiable {
    /// `claude-code`, `codex`, `opencode`.
    let id: String
    let name: String
    let `protocol`: String
    let installed: Bool
    let version: String?
    let configured: Bool
    /// The router's own configuration is in place.
    let managed: Bool
    let configPath: String?
    let canApply: Bool
    enum CodingKeys: String, CodingKey {
        case id, name, installed, version, configured, managed, `protocol`
        case configPath = "config_path", canApply = "can_apply"
    }
}

struct SetupModel: Decodable, Sendable, Equatable, Identifiable {
    let id: String
    let provider: String
    let displayName: String
    let accountId: String?
    /// A credential of its provider can sign it now.
    let available: Bool
    let serviceId: String?
    let nativeProtocol: String?
    let capabilities: ProviderRegistry.Capabilities?
    enum CodingKeys: String, CodingKey {
        case id, provider, available, capabilities
        case displayName = "display_name", accountId = "account_id", serviceId = "service_id", nativeProtocol = "native_protocol"
    }
}

/// What preview, check and apply take.
struct ClientInput: Encodable, Sendable, Equatable {
    let model: String
    let url: String
    var token: String? = nil
    var project: String? = nil
    var revision: String? = nil
}

struct ClientCheck: Decodable, Sendable, Equatable, Identifiable {
    let id: String
    let label: String
    /// `pass`, `warning`, `fail`.
    let status: String
    let detail: String
}

/// What preview answers, and check without the content.
struct ClientPreview: Decodable, Sendable, Equatable {
    let client: String
    let model: String
    let provider: String
    let protocolName: String
    let translated: Bool
    let canApply: Bool
    let path: String?
    /// `json` or `toml`.
    let format: String
    let content: String?
    let warnings: [String]
    let checks: [ClientCheck]
    let revision: String
    let catalogueContent: String?
    let cataloguePath: String?
    enum CodingKeys: String, CodingKey {
        case client, model, provider, translated, path, format, content, warnings, checks, revision
        case protocolName = "protocol", canApply = "can_apply"
        case catalogueContent = "catalogue_content", cataloguePath = "catalogue_path"
    }
}

struct ClientApplied: Decodable, Sendable { let path: String; let changed: Bool; let warnings: [String] }
struct ClientRestored: Decodable, Sendable {
    let path: String
    let changed: Bool
    let preservedEdits: Bool
    enum CodingKeys: String, CodingKey { case path, changed, preservedEdits = "preserved_edits" }
}
struct RestoreBody: Encodable, Sendable { var project: String? = nil }

enum ClientSetup {
    static let explain = "Choose the coding client you use and a model from any connected provider. Agent Router translates its requests and keeps provider credentials on the router."
    static let hostedNote = "This is a hosted router. Export the configuration and install it on the machine running your client."
    static let tokenNote = "Create a named Client access token for this client. Provider API keys belong in Accounts. A preview without a token contains a placeholder to replace on your client’s machine."
    static let checkNote = "The connection check verifies the model route, provider credential and router token. It does not run a model request."
    static let noCredential = "Add a provider credential in Accounts before configuring a client."

    /// A client's tag: whether the router's configuration is in place.
    static func status(_ c: SetupClient) -> String { c.managed ? "Routed" : c.configured ? "Configuration found" : "Not configured" }

    /// Where it is installed, as far as this router can tell.
    static func installation(_ c: SetupClient, hosted: Bool) -> String {
        hosted ? "Check on your client’s machine" : c.installed ? (c.version ?? "Installed") : "Not detected on this machine"
    }

    /// A model as the form names it: "display · Provider · id".
    static func modelLabel(_ m: SetupModel) -> String { "\(m.displayName) · \(Provider.name(m.provider)) · \(m.id)" }

    /// What a client cannot do on a model's route, said before it is set up.
    static func limits(client: String, provider: String?, nativeProtocol: String?, capabilities: ProviderRegistry.Capabilities?) -> [String] {
        let responses = nativeProtocol.map { $0 == "responses" } ?? (provider == "openai")
        if client == "codex" {
            return ((provider != nil && !responses) || capabilities?.compaction == false
                    ? ["Codex conversation compaction is unavailable on this route. Start a new session when the conversation gets long."] : [])
                + ["Codex web search is disabled in the generated configuration.",
                   "Start a new session when switching providers. Saved private reasoning and provider-specific conversation history may not translate."]
        }
        return responses ? ["This route uses Responses, which does not provide the Messages token-counting endpoint used by some client features. Messages stop sequences are also unavailable on this route."] : []
    }

    /// The file a configuration is shared as.
    static func filename(client: String, format: String) -> String {
        client == "claude-code" ? "settings.local.json" : client == "codex" ? "config.toml" : "opencode.\(format)"
    }
    static let catalogueFilename = "agent-router-models.json"

    /// A project directory is asked for on a local router, and never for Codex.
    static func asksProject(local: Bool, client: String) -> Bool { local && client != "codex" }
    /// Apply is a local router's, for a client it can write and a preview it can apply.
    static func offersApply(local: Bool, client: SetupClient, preview: ClientPreview?) -> Bool { local && client.canApply && preview?.canApply == true }
    /// Restore is a local router's, once its configuration is in place or a project is named.
    static func offersRestore(local: Bool, client: SetupClient, project: String) -> Bool {
        local && client.canApply && (client.managed || !project.trimmingCharacters(in: .whitespaces).isEmpty)
    }

    static func appliedWords(_ a: ClientApplied, name: String) -> String {
        a.changed ? "Saved \(name) configuration. Start a new client session to use it." : "\(name) already has this configuration."
    }
    static func restoredWords(_ r: ClientRestored) -> String {
        r.preservedEdits ? "Restored unchanged settings. Settings you edited remain in place; review the configuration for any remaining router URL or token."
            : r.changed ? "Restored the previous configuration." : "No router-managed configuration needed restoring."
    }

    static func input(model: String, url: String, token: String, project: String, revision: String? = nil) -> ClientInput {
        let t = token.trimmingCharacters(in: .whitespaces), p = project.trimmingCharacters(in: .whitespaces)
        return ClientInput(model: model, url: url.trimmingCharacters(in: .whitespaces), token: t.isEmpty ? nil : t, project: p.isEmpty ? nil : p, revision: revision)
    }

    static func path(_ client: String, _ act: String) -> String { "/api/clients/\(RouterClient.segment(client))/\(act)" }
}

@MainActor
@Observable
final class ClientSetupState {
    let router: RouterState
    var setup: ClientSetupReply?
    var failure: String?

    init(router: RouterState) { self.router = router }

    func load() async {
        guard let client = router.liveClient else { return }
        do { setup = try await client.get("/api/clients"); failure = nil } catch { failure = LiveFeed.describe(error) }
    }

    /// Preview, or check without the content.
    func run(_ act: String, client: String, input: ClientInput) async -> Result<ClientPreview, RouterClient.Failure> {
        await call { try await $0.post(ClientSetup.path(client, act), input) }
    }

    func apply(client: String, input: ClientInput) async -> Result<ClientApplied, RouterClient.Failure> {
        await call { try await $0.post(ClientSetup.path(client, "apply"), input) }
    }

    func restore(client: String, project: String) async -> Result<ClientRestored, RouterClient.Failure> {
        let p = project.trimmingCharacters(in: .whitespaces)
        return await call { try await $0.post(ClientSetup.path(client, "restore"), RestoreBody(project: p.isEmpty ? nil : p)) }
    }

    private func call<T: Sendable>(_ op: (RouterClient) async throws -> T) async -> Result<T, RouterClient.Failure> {
        guard let client = router.liveClient else { return .failure(.unreachable("bad address")) }
        do { return .success(try await op(client)) }
        catch { return .failure((error as? RouterClient.Failure) ?? .unreachable(error.localizedDescription)) }
    }
}
