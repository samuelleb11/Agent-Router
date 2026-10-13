// Settings → Forges and Settings → Library on a hosted router, as the console has them
// (ui/src/model/forges.ts, ui/src/data/library.ts, ui/src/screens/Settings/Forges.tsx and
// Library.tsx): the forges the Library imports from, its sources with their imports and webhook
// secrets, the publish tokens a pipeline publishes with, and whether a machine's publish goes live.

import Foundation
import Observation

/// A forge as listed: never its token, only whether one is kept.
struct ForgeRow: Decodable, Sendable, Equatable, Identifiable {
    let key: String
    /// `gitlab` or `github`.
    let kind: String
    let baseURL: String
    let tokenPresent: Bool
    let sources: Int
    let createdAt: Int
    var id: String { key }
    enum CodingKeys: String, CodingKey { case key, kind, sources, baseURL = "base_url", tokenPresent = "token_present", createdAt = "created_at" }
}

struct ForgesReply: Decodable, Sendable { let forges: [ForgeRow] }

/// `PUT /api/library/forges/{key}`: its token absent to keep the kept one, null to remove it, a
/// string to replace it.
struct ForgeBody: Encodable, Sendable, Equatable {
    enum Token: Equatable, Sendable { case keep, remove, set(String) }
    let kind: String
    let baseURL: String
    let token: Token
    enum CodingKeys: String, CodingKey { case kind, token, baseURL = "base_url" }
    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(kind, forKey: .kind)
        try c.encode(baseURL, forKey: .baseURL)
        switch token {
        case .keep: break
        case .remove: try c.encodeNil(forKey: .token)
        case .set(let t): try c.encode(t, forKey: .token)
        }
    }
}

/// What a forge's check answered.
struct ForgeChecked: Decodable, Sendable, Equatable {
    /// `ok`, `unauthorised`, `insufficient_scope`, `not_found`, `unreachable`, `unexpected`.
    let result: String
    let words: String
}

/// One line of an import.
struct ImportLine: Decodable, Sendable, Equatable {
    let kind: String
    let name: String
    let path: String
    /// `new`, `unchanged`, `refused`, `failed`.
    let outcome: String
    let version: Int?
    let state: String?
    let why: String?
}

/// What importing a source did.
struct Imported: Decodable, Sendable, Equatable {
    let source: String
    let commit: String
    let fetched: Bool
    let items: [ImportLine]
    let gone: [String]
}

/// A source's last import, or the forge's refusal.
enum LastImport: Decodable, Sendable, Equatable {
    case imported(Imported)
    case refused(String)

    private enum Keys: String, CodingKey { case error }
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        if let e = try c.decodeIfPresent(String.self, forKey: .error) { self = .refused(e) } else { self = .imported(try Imported(from: decoder)) }
    }
}

/// A source as listed: never its webhook secret, only whether it has one.
struct SourceRow: Decodable, Sendable, Equatable, Identifiable {
    let key: String
    let forge: String
    let repository: String
    let ref: String
    let path: String
    /// Its new versions go live without approval.
    let live: Bool
    let head: String?
    let checkedAt: Int?
    let importedAt: Int?
    let lastResult: LastImport?
    let hook: Bool
    let createdAt: Int
    var id: String { key }
    enum CodingKeys: String, CodingKey {
        case key, forge, repository, ref, path, live, head, hook
        case checkedAt = "checked_at", importedAt = "imported_at", lastResult = "last_result", createdAt = "created_at"
    }
}

struct SourcesReply: Decodable, Sendable { let sources: [SourceRow] }
struct SourceBody: Encodable, Sendable, Equatable { let forge: String; let repository: String; let ref: String; let path: String; let live: Bool }
struct ImportReply: Decodable, Sendable { let imported: Imported }
/// A webhook secret, shown once: the forge is given it and the URL.
struct SecretReply: Decodable, Sendable, Equatable { let secret: String; let path: String; let url: String? }

/// A publish token as listed: never its value.
struct PublishToken: Decodable, Sendable, Equatable, Identifiable {
    let id: String
    let name: String
    let kinds: [String]
    let items: [String]
    let live: Bool
    let createdAt: Int
    let lastUsedAt: Int?
    let revokedAt: Int?
    enum CodingKeys: String, CodingKey { case id, name, kinds, items, live, createdAt = "created_at", lastUsedAt = "last_used_at", revokedAt = "revoked_at" }
}

struct PublishTokensReply: Decodable, Sendable { let tokens: [PublishToken] }
struct PublishTokenBody: Encodable, Sendable, Equatable { let name: String; let kinds: [String]; let items: [String]; let live: Bool }
/// The token made, this once.
struct PublishTokenMade: Decodable, Sendable, Equatable, Identifiable { let id: String; let name: String; let token: String }
struct LibrarySetting: Codable, Sendable, Equatable {
    /// `live` or `approval`.
    let machinePublishes: String
    enum CodingKeys: String, CodingKey { case machinePublishes = "machine_publishes" }
}

enum LibrarySettings {
    static let forgeKinds = [("gitlab", "GitLab"), ("github", "GitHub")]
    static let forgesExplain = "Where the Library imports skills, commands and subagents from: GitLab, or GitHub. A forge’s token is kept on the router and never shown again; a public GitHub repository needs none."
    static let sourcesExplain = "A repository, a ref and a path on a forge. Every skill directory, commands/*.md and agents/*.md under the path is an item, imported when the ref moves — every five minutes, or at once from the forge’s webhook."
    static let tokensExplain = "For a pipeline or a cloud session that publishes to the Library: a token can publish and do nothing else, and never holds the router’s password."
    static let machinesExplain = "What an agent publishes with agent-router library publish, signed with its machine and its session."
    static let secretExplain = "Give the forge both — GitLab’s “Secret token”, or GitHub’s “Secret” with content type application/json. The secret is shown now and never again, and any secret made before it no longer works."

    static func kindLabel(_ kind: String) -> String { forgeKinds.first { $0.0 == kind }?.1 ?? kind }

    /// A key a forge or a source is known by: lowercase letters and digits, single hyphens between.
    static func isKey(_ key: String) -> Bool { key.count <= 64 && key.range(of: #"^[a-z0-9]+(-[a-z0-9]+)*$"#, options: .regularExpression) != nil }

    /// A forge's check, as a heading over the router's own words.
    static func checked(_ c: ForgeChecked) -> String {
        let heading = ["ok": "The forge answered", "unauthorised": "The forge refused the token", "insufficient_scope": "The token may not read this",
                       "not_found": "The forge has no such repository", "unreachable": "The forge could not be reached",
                       "unexpected": "The forge answered something unexpected"][c.result] ?? c.result
        return "\(heading): \(c.words)"
    }

    /// What an import came to, counted: "Imported at 0123abcd: 3 new, 2 unchanged, 1 refused."
    static func importWords(_ i: Imported) -> String {
        if !i.fetched { return "Nothing to fetch: the ref is still at \(i.commit.prefix(8))." }
        let count = { (o: String) in i.items.filter { $0.outcome == o }.count }
        var parts = ["\(count("new")) new", "\(count("unchanged")) unchanged"]
        if count("refused") > 0 { parts.append("\(count("refused")) refused") }
        if count("failed") > 0 { parts.append("\(count("failed")) failed") }
        let gone = i.gone.isEmpty ? "" : "; no longer in the repository: \(i.gone.joined(separator: ", "))"
        return "Imported at \(i.commit.prefix(8)): \(parts.joined(separator: ", "))\(gone)."
    }

    /// What a source's last import came to, or that it has not run, or the forge's refusal.
    static func lastWords(_ s: SourceRow) -> String {
        switch s.lastResult {
        case nil: "Never imported yet."
        case .refused(let e)?: "The forge said no: \(e)"
        case .imported(let i)?: importWords(i)
        }
    }

    /// What a publish token may publish.
    static func scopeWords(_ t: PublishToken) -> String {
        if !t.items.isEmpty { return t.items.joined(separator: ", ") }
        if !t.kinds.isEmpty { return t.kinds.map { "any \($0)" }.joined(separator: ", ") }
        return "anything"
    }

    /// "skill/bench, command/deploy" typed, as the list a token is made with.
    static func itemList(_ typed: String) -> [String] {
        typed.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    }

    static func forgePath(_ key: String, _ tail: String = "") -> String { "/api/library/forges/" + RouterClient.segment(key) + tail }
    static func sourcePath(_ key: String, _ tail: String = "") -> String { "/api/library/sources/" + RouterClient.segment(key) + tail }
}

@MainActor
@Observable
final class LibrarySettingsState {
    let router: RouterState
    var forges: [ForgeRow]?
    var sources: [SourceRow]?
    var tokens: [PublishToken]?
    var setting: LibrarySetting?
    var failure: String?

    init(router: RouterState) { self.router = router }

    func load() async {
        guard let client = router.liveClient else { return }
        async let f: ForgesReply = client.get("/api/library/forges")
        async let s: SourcesReply = client.get("/api/library/sources")
        async let t: PublishTokensReply = client.get("/api/library/tokens")
        async let g: LibrarySetting = client.get("/api/library/settings")
        var problems: [String] = []
        do { forges = try await f.forges } catch { problems.append(LiveFeed.describe(error)) }
        do { sources = try await s.sources } catch { problems.append(LiveFeed.describe(error)) }
        do { tokens = try await t.tokens } catch { problems.append(LiveFeed.describe(error)) }
        do { setting = try await g } catch { problems.append(LiveFeed.describe(error)) }
        failure = problems.first
    }

    /// The reply, or the router's words for refusing.
    func call<T: Decodable & Sendable>(_ op: (RouterClient) async throws -> T) async -> Result<T, RouterClient.Failure> {
        guard let client = router.liveClient else { return .failure(.unreachable("bad address")) }
        do {
            let r = try await op(client)
            await load()
            return .success(r)
        } catch {
            return .failure((error as? RouterClient.Failure) ?? .unreachable(error.localizedDescription))
        }
    }
}
