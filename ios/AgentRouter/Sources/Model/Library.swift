// A hosted router's Library, as the console's Library screen has it (ui/src/model/library.ts,
// ui/src/data/library.ts, ui/src/screens/Library): every skill, command and subagent with its
// versions, where each came from and how much it is used, and the decisions taken on them here
// — make a version live, reject one, archive an item, sync every machine. Uploading a version
// stays a desk task (Sam's scope, 2026-10-12).

import Foundation
import Observation

/// Where a version came from (src/library/store.rs, `Provenance`), its origin under `from`.
enum LibraryProvenance: Decodable, Sendable, Hashable {
    case upload
    case forge(source: String, forge: String, repository: String, commit: String, path: String)
    case token(String)
    case machine(machine: String, label: String?, session: String?)
    case router(build: String)
    case unknown

    private enum Keys: String, CodingKey { case from, source, forge, repository, commit, path, token, machine, label, session, build }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        let s = { (k: Keys) in (try? c.decodeIfPresent(String.self, forKey: k)) ?? nil }
        switch s(.from) {
        case "upload": self = .upload
        case "forge": self = .forge(source: s(.source) ?? "", forge: s(.forge) ?? "", repository: s(.repository) ?? "", commit: s(.commit) ?? "", path: s(.path) ?? "")
        case "token": self = .token(s(.token) ?? "")
        case "machine": self = .machine(machine: s(.machine) ?? "", label: s(.label), session: s(.session))
        case "router": self = .router(build: s(.build) ?? "")
        default: self = .unknown
        }
    }
}

/// A version in brief, as a list names its live and newest one.
struct LibraryBrief: Decodable, Sendable, Hashable {
    let number: Int
    let digest: String
    let size: Int
    /// `live`, `pending`, `rejected`, `superseded`.
    let state: String
    let provenance: LibraryProvenance
    let createdAt: Int
    enum CodingKeys: String, CodingKey { case number, digest, size, state, provenance, createdAt = "created_at" }
}

/// One item of `GET /api/library`.
struct LibraryItem: Decodable, Sendable, Hashable, Identifiable {
    /// `skill`, `command`, `subagent`.
    let kind: String
    let name: String
    let description: String
    /// Nil for uploads and publishes, `source:<key>`, or `router`.
    let source: String?
    let createdAt: Int
    let archivedAt: Int?
    let live: LibraryBrief?
    let newest: LibraryBrief?
    let pending: Int
    let versions: Int
    let uses30d: Int?
    let lastUsedAt: Int?
    var id: String { "\(kind)/\(name)" }
    enum CodingKeys: String, CodingKey {
        case kind, name, description, source, live, newest, pending, versions
        case createdAt = "created_at", archivedAt = "archived_at", uses30d = "uses_30d", lastUsedAt = "last_used_at"
    }
}

struct LibraryList: Decodable, Sendable { let items: [LibraryItem] }

/// One version in full, as an item's page lists them.
struct LibraryVersion: Decodable, Sendable, Hashable, Identifiable {
    let number: Int
    let digest: String
    let size: Int
    let state: String
    let provenance: LibraryProvenance
    let createdAt: Int
    let description: String
    let decidedAt: Int?
    var id: Int { number }
    enum CodingKeys: String, CodingKey {
        case number, digest, size, state, provenance, description
        case createdAt = "created_at", decidedAt = "decided_at"
    }
}

/// `GET /api/library/items/{kind}/{name}`: one item, its versions and its newest's files.
struct LibraryItemDetail: Decodable, Sendable, Equatable {
    let kind: String
    let name: String
    let description: String
    let source: String?
    let archivedAt: Int?
    let versions: [LibraryVersion]
    let files: [File]

    struct File: Decodable, Sendable, Hashable, Identifiable {
        let path: String
        let size: Int
        let executable: Bool
        var id: String { path }
    }
    enum CodingKeys: String, CodingKey { case kind, name, description, source, versions, files, archivedAt = "archived_at" }
}

/// `GET /api/library/usage`: every item's use over 30 days, in this phone's days, and what the
/// Library's skills add to every turn.
struct LibraryUsage: Decodable, Sendable, Equatable {
    let days: [Int]
    let listing: Listing?
    let items: [ItemUsage]

    struct Listing: Decodable, Sendable, Equatable {
        let at: Int
        let chars: Int
        let tokens: Int
    }

    struct ItemUsage: Decodable, Sendable, Equatable {
        let kind: String
        let name: String
        let uses30d: Int
        let lastUsedAt: Int?
        let daily: [Int]
        let machines: [ByMachine]
        let projects: [ByProject]
        enum CodingKeys: String, CodingKey { case kind, name, daily, machines, projects, uses30d = "uses_30d", lastUsedAt = "last_used_at" }
    }

    struct ByMachine: Decodable, Sendable, Equatable {
        let id: String?
        let label: String?
        let uses: Int
    }

    struct ByProject: Decodable, Sendable, Equatable {
        let project: String?
        let uses: Int
    }
}

enum Library {
    static let kinds = ["skill", "command", "subagent"]

    /// A local router keeps no Library: it is a hosted router's.
    static let notHere = "The Library is a hosted router’s: it keeps the skills, commands and subagents of the machines connected to it. This router runs on this machine, so it keeps none."
    /// An empty Library, and where one is added: the console, at a desk.
    static let empty = "The Library is empty. Add a skill, a command or a subagent from the console, and every connected machine gets it."

    static func kindWord(_ kind: String) -> String { ["skill": "Skill", "command": "Command", "subagent": "Subagent"][kind] ?? kind }

    static func stateWord(_ state: String) -> String {
        ["live": "live", "pending": "waits for approval", "rejected": "rejected", "superseded": "superseded"][state] ?? state
    }

    /// Where an item takes its versions from.
    static func sourceWords(_ source: String?) -> String {
        if source == "router" { return "comes with the router’s build" }
        if let s = source, s.hasPrefix("source:") { return "imported by the source \(s.dropFirst("source:".count))" }
        return "uploaded and published here"
    }

    /// Where a version came from, in words.
    static func provenanceWords(_ p: LibraryProvenance) -> String {
        switch p {
        case .upload: "uploaded from the console"
        case let .forge(_, _, repository, commit, path): "imported from \(repository) at \(commit.prefix(8))" + (path.isEmpty ? "" : ", \(path)")
        case .token(let t): "published with the token \(t)"
        case let .machine(machine, label, session): "published from \(label.flatMap { $0.isEmpty ? nil : $0 } ?? String(machine.prefix(8)))" + (session == nil ? "" : " by a Claude Code session")
        case .router: "built into the router"
        case .unknown: "from somewhere the console does not know"
        }
    }

    /// The session a version was published by, which opens in the live view.
    static func sessionOf(_ p: LibraryProvenance) -> String? {
        if case let .machine(_, _, session?) = p, !session.isEmpty { return session }
        return nil
    }

    /// "v3 live", or that none is.
    static func liveWords(_ item: LibraryItem) -> String { item.live.map { "v\($0.number) live" } ?? "none live" }

    /// The file Claude Code reads of an item, shown first.
    static func mainFile(kind: String, name: String) -> String { kind == "skill" ? "SKILL.md" : "\(name).md" }

    /// What can be done to a version: approve or reject one that waits, make an older one live
    /// again. A live one and a rejected one take nothing.
    enum Act: String, Sendable { case approve, reject, restore }
    static func acts(_ state: String) -> [Act] {
        switch state {
        case "pending": [.approve, .reject]
        case "superseded": [.restore]
        default: []
        }
    }
    static func actWord(_ act: Act) -> String {
        switch act {
        case .approve: "Approve"
        case .reject: "Reject"
        case .restore: "Make live again"
        }
    }
    /// The route an act posts to: approving and making live again are both making it live.
    static func actPath(kind: String, name: String, version: Int, _ act: Act) -> String {
        itemPath(kind: kind, name: name) + "/versions/\(version)/" + (act == .reject ? "reject" : "live")
    }

    static func itemPath(kind: String, name: String) -> String {
        "/api/library/items/\(RouterClient.segment(kind))/\(RouterClient.segment(name))"
    }

    static func filePath(kind: String, name: String, version: Int, path: String) -> String {
        itemPath(kind: kind, name: name) + "/versions/\(version)/files/" + path.split(separator: "/").map { RouterClient.segment(String($0)) }.joined(separator: "/")
    }

    /// Items by kind, then name.
    static func ordered(_ items: [LibraryItem]) -> [LibraryItem] {
        items.sorted { a, b in
            let ka = kinds.firstIndex(of: a.kind) ?? kinds.count, kb = kinds.firstIndex(of: b.kind) ?? kinds.count
            return ka != kb ? ka < kb : a.name.localizedStandardCompare(b.name) == .orderedAscending
        }
    }

    /// How often an item was used.
    static func usedWords(_ uses: Int) -> String {
        if uses <= 0 { return "not used in 30 days" }
        return uses == 1 ? "used once in 30 days" : "used \(uses.formatted()) times in 30 days"
    }

    /// What the Library's skills add to every turn, or that it is not measured yet.
    static func listingWords(_ listing: LibraryUsage.Listing?) -> String {
        guard let listing else { return "What the Library’s skills add to every turn is not measured yet: no turn the router has carried lists one of them." }
        return "The Library’s skills add about \(listing.tokens.formatted()) tokens to every turn."
    }

    static func machineWords(_ m: LibraryUsage.ByMachine) -> String { m.label ?? m.id ?? "a machine not known" }

    /// A Markdown file without its frontmatter, which the page shows apart as the description.
    static func body(_ text: String) -> String {
        var lines = text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        if let first = lines.first, first.hasPrefix("\u{FEFF}") { lines[0] = String(first.dropFirst()) }
        guard lines.first?.trimmingCharacters(in: .whitespaces) == "---",
              let end = lines.dropFirst().firstIndex(where: { $0.trimmingCharacters(in: .whitespaces) == "---" }) else { return text }
        return lines[(end + 1)...].joined(separator: "\n").replacingOccurrences(of: #"^\n+"#, with: "", options: .regularExpression)
    }
}

@MainActor
@Observable
final class LibraryState {
    let router: RouterState
    var items: [LibraryItem]?
    var usage: LibraryUsage?
    var failure: String?
    var done: String?
    var busy = false

    init(router: RouterState) { self.router = router }

    func load() async {
        guard let client = router.liveClient else { return }
        async let list: LibraryList = client.get("/api/library")
        async let use: LibraryUsage = client.get("/api/library/usage", query: ["tz": String(TimeZone.current.secondsFromGMT() / 60)])
        do { items = Library.ordered(try await list.items); failure = nil } catch { failure = LiveFeed.describe(error) }
        if let u = try? await use { usage = u }
    }

    func usage(of item: LibraryItem) -> LibraryUsage.ItemUsage? {
        usage?.items.first { $0.kind == item.kind && $0.name == item.name }
    }

    /// Tells every connected machine to sync its Library now, and says what each answered.
    func syncAll() async {
        guard let client = router.liveClient else { return }
        busy = true
        defer { busy = false }
        do {
            let reply: LibrarySyncReply = try await client.post("/api/library/sync", LibrarySyncBody())
            done = reply.machines.isEmpty ? "No machine is connected to tell." : reply.machines.map(Machines.told).joined(separator: "\n")
        } catch {
            failure = LiveFeed.describe(error)
        }
    }

    func detail(kind: String, name: String) async -> Result<LibraryItemDetail, RouterClient.Failure> {
        guard let client = router.liveClient else { return .failure(.unreachable("bad address")) }
        do { return .success(try await client.get(Library.itemPath(kind: kind, name: name))) }
        catch { return .failure((error as? RouterClient.Failure) ?? .unreachable(error.localizedDescription)) }
    }

    func file(kind: String, name: String, version: Int, path: String) async -> String? {
        try? await router.liveClient?.text(Library.filePath(kind: kind, name: name, version: version, path: path))
    }

    /// An act on a version: nil when the router took it, else its words for refusing.
    func act(_ act: Library.Act, kind: String, name: String, version: Int) async -> String? {
        guard let client = router.liveClient else { return "bad address" }
        do {
            let _: OkReply = try await client.post(Library.actPath(kind: kind, name: name, version: version, act), [String: String]())
            await load()
            return nil
        } catch {
            return LiveFeed.describe(error)
        }
    }

    func archive(kind: String, name: String) async -> String? {
        guard let client = router.liveClient else { return "bad address" }
        do {
            let _: OkReply = try await client.post(Library.itemPath(kind: kind, name: name) + "/archive", [String: String]())
            await load()
            return nil
        } catch {
            return LiveFeed.describe(error)
        }
    }
}
