// Settings → Clients, as the console has it (ui/src/model/client-setup.ts,
// ui/src/screens/Settings/Clients.tsx): Claude Code, Codex and OpenCode set up to reach the
// router with an exact model and a router credential — previewed, checked, applied on a local
// router, restored, or shared as a file to install on the client's machine. A hosted router also
// sets up a connected machine's Claude Code and Codex through its agent (AGEN-0197, AGEN-0200).

import Foundation
import Observation

/// `GET /api/clients`.
struct ClientSetupReply: Decodable, Sendable, Equatable {
    /// `local` or `server`.
    let mode: String
    let url: String
    let clients: [SetupClient]
    let models: [SetupModel]
    /// The machine whose clients these are, when a hosted router read them through its agent.
    var machine: String? = nil
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
    static let hostedNote = "This is a hosted router. Choose a connected machine to set up its Claude Code and Codex from here, or export the configuration and install it on the machine running your client."
    static let machineNote = "Its agent reads, applies and restores Claude Code’s and Codex’s settings there. OpenCode is exported."
    static let tokenNote = "Create a named Client access token for this client. Provider API keys belong in Accounts. A preview without a token contains a placeholder to replace on your client’s machine."
    static let checkNote = "The connection check verifies the model route, provider credential and router token. It does not run a model request."
    static let noCredential = "Add a provider credential in Accounts before configuring a client."

    /// A client's tag: whether the router's configuration is in place.
    static func status(_ c: SetupClient) -> String { c.managed ? "Routed" : c.configured ? "Configuration found" : "Not configured" }

    /// Where it is installed, as far as this router can tell: on a hosted router, only what a
    /// chosen machine's agent read of it.
    static func installation(_ c: SetupClient, hosted: Bool, machine: String? = nil) -> String {
        if hosted && !(machine != nil && c.canApply) { return "Check on your client’s machine" }
        return c.installed ? (c.version ?? "Installed") : "Not detected on \(machine ?? "this machine")"
    }

    /// The machines a hosted router's Clients page can be about: the connected ones.
    static func machineChoices(_ rows: [MachineRow]) -> [MachineRow] { rows.filter(\.online) }

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
    /// Apply writes on a local router's machine, or a hosted router's chosen one (`writes`), for a
    /// client it can write and a preview it can apply.
    static func offersApply(writes: Bool, client: SetupClient, preview: ClientPreview?) -> Bool { writes && client.canApply && preview?.canApply == true }
    /// Restore, where Apply writes, once its configuration is in place or a project is named.
    static func offersRestore(writes: Bool, client: SetupClient, project: String) -> Bool {
        writes && client.canApply && (client.managed || !project.trimmingCharacters(in: .whitespaces).isEmpty)
    }

    static func appliedWords(_ a: ClientApplied, name: String, machine: String? = nil) -> String {
        let at = machine.map { " on \($0)" } ?? ""
        return a.changed ? "Saved \(name) configuration\(at). Start a new client session to use it." : "\(name)\(at) already has this configuration."
    }
    static func restoredWords(_ r: ClientRestored) -> String {
        r.preservedEdits ? "Restored unchanged settings. Settings you edited remain in place; review the configuration for any remaining router URL or token."
            : r.changed ? "Restored the previous configuration." : "No router-managed configuration needed restoring."
    }

    static func input(model: String, url: String, token: String, project: String, revision: String? = nil) -> ClientInput {
        let t = token.trimmingCharacters(in: .whitespaces), p = project.trimmingCharacters(in: .whitespaces)
        return ClientInput(model: model, url: url.trimmingCharacters(in: .whitespaces), token: t.isEmpty ? nil : t, project: p.isEmpty ? nil : p, revision: revision)
    }

    static func path(_ client: String, _ act: String, machine: String? = nil) -> String {
        onMachine("/api/clients/\(RouterClient.segment(client))/\(act)", machine)
    }

    /// A path of the Clients API, about one machine when one is chosen, as the console's
    /// `clientsOnMachine` sends it.
    static func onMachine(_ path: String, _ machine: String?) -> String {
        machine.map { "\(path)?machine=\(RouterClient.queryValue($0))" } ?? path
    }
}

@MainActor
@Observable
final class ClientSetupState {
    let router: RouterState
    /// The router's own reading.
    var setup: ClientSetupReply?
    var failure: String?
    /// On a hosted router: the connected machines, the one chosen, and its reading.
    var machines: [MachineRow] = []
    var machine: String?
    var there: ClientSetupReply?
    var thereFailure: String?

    init(router: RouterState) { self.router = router }

    func load() async {
        guard let client = router.liveClient else { return }
        do { setup = try await client.get("/api/clients"); failure = nil } catch { failure = LiveFeed.describe(error) }
        if setup?.mode == "server" {
            let rows: [MachineRow]? = try? await client.get("/api/machines")
            machines = ClientSetup.machineChoices(rows ?? machines)
        }
        await loadMachine()
    }

    /// Point the page at a machine, or with nil back at an export.
    func choose(_ id: String?) async {
        machine = id
        there = nil
        thereFailure = nil
        await loadMachine()
    }

    private func loadMachine() async {
        guard let id = machine, let client = router.liveClient else { return }
        do {
            let read: ClientSetupReply = try await client.get(ClientSetup.onMachine("/api/clients", id))
            // A reading answers for the machine still chosen, or for none.
            if machine == id { there = read; thereFailure = nil }
        } catch {
            if machine == id { thereFailure = LiveFeed.describe(error) }
        }
    }

    /// A machine as the page names it: its label, else its id.
    func label(_ id: String) -> String { machines.first { $0.id == id }?.label ?? id }

    /// Preview, or check without the content.
    func run(_ act: String, client: String, input: ClientInput, machine: String? = nil) async -> Result<ClientPreview, RouterClient.Failure> {
        await call { try await $0.post(ClientSetup.path(client, act, machine: machine), input) }
    }

    func apply(client: String, input: ClientInput, machine: String? = nil) async -> Result<ClientApplied, RouterClient.Failure> {
        await call { try await $0.post(ClientSetup.path(client, "apply", machine: machine), input) }
    }

    func restore(client: String, project: String, machine: String? = nil) async -> Result<ClientRestored, RouterClient.Failure> {
        let p = project.trimmingCharacters(in: .whitespaces)
        return await call { try await $0.post(ClientSetup.path(client, "restore", machine: machine), RestoreBody(project: p.isEmpty ? nil : p)) }
    }

    private func call<T: Sendable>(_ op: (RouterClient) async throws -> T) async -> Result<T, RouterClient.Failure> {
        guard let client = router.liveClient else { return .failure(.unreachable("bad address")) }
        do { return .success(try await op(client)) }
        catch { return .failure((error as? RouterClient.Failure) ?? .unreachable(error.localizedDescription)) }
    }
}
