// Settings → Access tokens, as the console has it (ui/src/model/access-tokens.ts,
// ui/src/screens/Settings/AccessTokens.tsx): router credentials for clients and machines, each
// named, with its permissions and expiry, shown in full once and revoked without touching the
// others. Provider keys stay on the Accounts page.

import Foundation
import Observation

/// One token of `GET /api/auth/tokens`: what is public of it, never the token itself.
struct AccessToken: Decodable, Sendable, Identifiable, Hashable {
    let id: String
    let name: String
    /// The token's first characters, to tell it by.
    let prefix: String
    /// `client`, `machine` or `admin`.
    let scope: String
    /// Epoch milliseconds, each.
    let createdAt: Int
    let lastUsedAt: Int?
    let expiresAt: Int?
    let revokedAt: Int?
    enum CodingKeys: String, CodingKey {
        case id, name, prefix, scope
        case createdAt = "created_at", lastUsedAt = "last_used_at", expiresAt = "expires_at", revokedAt = "revoked_at"
    }
}

struct AccessTokensReply: Decodable, Sendable { let tokens: [AccessToken] }

/// `POST /api/auth/tokens`: a token to make, its expiry null for one that never expires.
struct CreateTokenBody: Encodable, Sendable, Equatable {
    let name: String
    let scope: String
    let expiresAt: Int?
    enum CodingKeys: String, CodingKey { case name, scope, expiresAt = "expires_at" }
    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(name, forKey: .name)
        try c.encode(scope, forKey: .scope)
        try c.encode(expiresAt, forKey: .expiresAt)
    }
}

/// What a create answers: the token's public fields, and the token itself, this once.
struct CreatedToken: Decodable, Sendable, Identifiable, Equatable {
    let id: String
    let name: String
    let scope: String
    let expiresAt: Int?
    let token: String
    enum CodingKeys: String, CodingKey { case id, name, scope, token, expiresAt = "expires_at" }
}

enum AccessTokens {
    struct Scope: Identifiable, Sendable {
        let value: String
        let label: String
        let description: String
        var id: String { value }
    }

    static let scopes = [
        Scope(value: "client", label: "Client", description: "Send model requests and read the model catalogue. Use for Claude Code, Codex or OpenCode."),
        Scope(value: "machine", label: "Machine", description: "Connect a machine agent, report its sessions and send model requests."),
        Scope(value: "admin", label: "Administrator", description: "Access the dashboard and change router settings, accounts and access tokens."),
    ]

    /// How long a new token lasts, in days; nil never expires.
    static let lifetimes: [(days: Int?, label: String)] = [(30, "30 days"), (90, "90 days"), (365, "1 year"), (nil, "Never")]

    static let explain = "Give each client or machine its own named credential for this router. Choose its permissions and expiry; revoke it without changing other clients. Provider API keys stay on the Accounts page."
    static let howSent = "Clients send the token as Authorization: Bearer <token> or x-api-key. Existing router passwords continue to work."
    static let shownOnce = "This is the only time the full token is shown. Copy it to your client or machine configuration before closing."
    static let revokeWhat = "This token will stop authenticating new requests immediately. Existing machine connections close within five seconds. Give any client or machine using it a replacement token to reconnect."

    static func scopeLabel(_ scope: String) -> String { scopes.first { $0.value == scope }?.label ?? scope }

    /// When a token made now with this lifetime expires, epoch milliseconds; nil for never.
    static func expiry(days: Int?, now: Date = .now) -> Int? {
        days.map { Int(now.timeIntervalSince1970 * 1000) + $0 * 86_400_000 }
    }

    /// Active, Expired or Revoked.
    static func status(_ t: AccessToken, now: Date = .now) -> String {
        if t.revokedAt != nil { return "Revoked" }
        if let at = t.expiresAt, at <= Int(now.timeIntervalSince1970 * 1000) { return "Expired" }
        return "Active"
    }

    /// The sentence under a token just made: its permissions and when it expires.
    static func createdWords(_ t: CreatedToken) -> String {
        "Permissions: \(scopeLabel(t.scope)). " + (t.expiresAt.flatMap(Fmt.date(ms:)).map { "Expires \(Fmt.when($0))." } ?? "It does not expire.")
    }
}

@MainActor
@Observable
final class AccessTokensState {
    let router: RouterState
    var tokens: [AccessToken]?
    var failure: String?
    var done: String?

    init(router: RouterState) { self.router = router }

    func load() async {
        guard let client = router.liveClient else { return }
        do { tokens = (try await client.get("/api/auth/tokens") as AccessTokensReply).tokens; failure = nil }
        catch { failure = LiveFeed.describe(error) }
    }

    /// The token made, or the router's words for refusing it.
    func create(name: String, scope: String, days: Int?) async -> Result<CreatedToken, RouterClient.Failure> {
        guard let client = router.liveClient else { return .failure(.unreachable("bad address")) }
        do {
            let made: CreatedToken = try await client.post("/api/auth/tokens",
                CreateTokenBody(name: name.trimmingCharacters(in: .whitespaces), scope: scope, expiresAt: AccessTokens.expiry(days: days)))
            await load()
            return .success(made)
        } catch let e as RouterClient.Failure {
            return .failure(e)
        } catch {
            return .failure(.unreachable(error.localizedDescription))
        }
    }

    func revoke(_ t: AccessToken) async {
        guard let client = router.liveClient else { return }
        do {
            let _: OkReply = try await client.delete("/api/auth/tokens/" + RouterClient.segment(t.id))
            done = "Revoked \(t.name)"
            failure = nil
        } catch {
            failure = LiveFeed.describe(error)
        }
        await load()
    }
}
