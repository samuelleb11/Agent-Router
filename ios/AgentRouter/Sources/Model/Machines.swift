// A hosted router's machines, with no view in it: how a machine connects — the installer
// the router serves, one command per platform — and every machine the router has seen, as
// the Machines screen lists them. The web console's `ui/src/model/machine.ts` is the
// reference, and its sentences are this screen's wherever they fit a phone
// (docs/web-console-contracts.md, MA-C2 to MA-C5).

import Foundation

/// One row of `GET /api/machines` (src/machines.rs, `Seen`): a machine the router has seen
/// since it last started, online or not. The reply is a list, and anything else is a failed
/// read, never an empty one.
struct MachineRow: Decodable, Sendable, Identifiable, Hashable {
    let id: String
    /// What its agent was given with `--label`, else its host name.
    let label: String
    /// Its operating system as Rust names it: `macos`, `linux`, `windows`.
    let platform: String
    let version: String?
    /// The source tree its agent was built from, `tree:<sha>`, as `/_router/health` names
    /// the router's own.
    let build: String?
    let online: Bool
    /// When it connected, while it is online, and when it went away once it is not: epoch
    /// milliseconds on the router's clock.
    let sinceMs: Int?
    /// The clients its agent reported, Claude Code first; empty when it has not said
    /// (src/clients.rs, `Client`).
    let clients: [MachineClient]
    /// Where its Library stands; nil from a router that keeps none.
    let library: LibraryStanding?

    enum CodingKeys: String, CodingKey {
        case id, label, platform, version, build, online, clients, library, sinceMs = "since_ms"
    }
}

/// Where a machine's Library stands (src/library/push.rs, `Standing`). Its own fields are Claude
/// Code's; each other client's standing is under its name in `clients`.
struct LibraryStanding: Decodable, Sendable, Hashable {
    /// `in_step`, `behind`, `failed`, `replaced`, `predates`, `opted_out`, `not_yet`.
    let state: String
    let behind: Int?
    let failed: Int?
    let replaced: [String]?
    let atMs: Int?
    let clients: [String: LibraryStanding]?
    enum CodingKeys: String, CodingKey { case state, behind, failed, replaced, clients, atMs = "at_ms" }
}

/// What one machine said when told to sync (src/library/push.rs, `Told`).
struct LibraryTold: Decodable, Sendable, Hashable {
    let machine: String
    let label: String
    /// `synced`, `predates`, `opted_out`, `offline`, `failed`.
    let outcome: String
    let report: Report?
    let why: String?

    struct Report: Decodable, Sendable, Hashable {
        let items: [Item]
        struct Item: Decodable, Sendable, Hashable { let outcome: String }
    }
}

/// `POST /api/library/sync`: each machine told, and what it said.
struct LibrarySyncReply: Decodable, Sendable {
    let machines: [LibraryTold]
}

/// Which machine to tell; none tells every one.
struct LibrarySyncBody: Encodable, Sendable, Equatable {
    var machine: String? = nil
}

/// One client a machine's agent reported: its command line, its extension in each editor that
/// has it, and its home folder.
struct MachineClient: Decodable, Sendable, Hashable {
    /// `claude-code`, `kimi-code`, `codex`, `opencode`.
    let client: String
    let cli: CLI?
    let extensions: [Extension]?
    let home: String?

    struct CLI: Decodable, Sendable, Hashable {
        let path: String?
        /// Nil when it did not answer in time.
        let version: String?
    }

    struct Extension: Decodable, Sendable, Hashable {
        /// The editor's folder name: `vscode`, `cursor`…
        let editor: String?
        let version: String?
    }
}

extension MachineRow {
    /// The id is what names the machine, and is required, as a session row's id is. The rest
    /// is read leniently: one agent that says less than this build expects must not fail the
    /// whole list.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        label = c.lenient(String.self, .label) ?? ""
        platform = c.lenient(String.self, .platform) ?? ""
        version = c.lenient(String.self, .version)
        build = c.lenient(String.self, .build)
        online = c.lenient(Bool.self, .online) ?? false
        sinceMs = c.lenient(Int.self, .sinceMs)
        clients = c.lenient([MachineClient].self, .clients) ?? []
        library = c.lenient(LibraryStanding.self, .library)
    }
}

enum Machines {
    // MARK: connecting a machine

    /// The screen's first section, and the link to it from a hosted router's empty session
    /// list (MA-C2, SE-C187).
    static let connect = "Connect a machine"

    /// The router serves the installer without a password; it installs the agent and asks
    /// there for the router's password (MA-C2).
    static let connectHow = "Run this on a machine to connect it — paste a Machine access token from this router into its password field. The existing router password also works."

    /// What else the installer does where VS Code is (MA-C2). The console's sentence, without
    /// its "and from the phone", which on the phone is "here".
    static let withVSCode = "With VS Code’s code command on the machine, the script also adds the Agent Router extension, so its sessions can be typed into from here; without it, they can only be followed."

    /// A hosted router that no machine has connected to: said under the commands here, and
    /// by a hosted router's empty session list (MA-C5, SE-C187).
    static let agentless = "No machine has connected yet."

    struct InstallCommand: Identifiable, Equatable, Sendable {
        /// The platforms it runs on, which label it.
        let label: String
        let command: String
        var id: String { label }
    }

    /// Where a machine's agent connects: the address the router gives for itself when it
    /// gives one, else the one this phone reaches it at. The installer the router serves
    /// writes the same address into the agent (src/downloads.rs, `address`).
    static func origin(publicURL: String?, router: Router) -> String {
        var url = publicURL?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        while url.hasSuffix("/") { url.removeLast() }
        return url.isEmpty ? router.origin : url
    }

    /// The commands that connect a machine to the router at `origin`, one for each platform:
    /// a shell's on macOS and Linux, PowerShell's on Windows (MA-C2).
    static func installCommands(origin: String) -> [InstallCommand] {
        [
            InstallCommand(label: "macOS and Linux", command: "curl -fsSL \(origin)/agent/install.sh | sh"),
            InstallCommand(label: "Windows (PowerShell)", command: "irm \(origin)/agent/install.ps1 | iex"),
        ]
    }

    // MARK: the machines

    /// The list's heading: the router lists what it has seen since it last started, online
    /// or not (MA-C3).
    static let seen = "Seen since this router started"

    /// A machine's build compared with the router's: quietly the same, or another, with what
    /// updates it (MA-C4).
    static let sameBuild = "Same build as this router"
    static let otherBuild = "Runs a different build from this router — run the installer on it again to update it."

    /// One machine as the screen lists it.
    struct Item: Identifiable, Equatable, Sendable {
        let id: String
        /// Its label, trimmed; a blank one names it by the start of its id, as a session's
        /// machine is named.
        let label: String
        /// Its platform in words (MA-C3).
        let platform: String
        let online: Bool
        /// "online for 3h 12m", "offline for 2d 4h" (MA-C3).
        let presence: String
        /// Whether it runs the router's own build, and the words for it; nil until the
        /// router's build is known, or when the machine did not say its own (MA-C4).
        let build: Build?
        /// Its clients, one line each, in the order its agent reported them (MA-C8).
        let clients: [ClientItem]
        /// Where its Library stands, and whether that is something to act on; nil where its
        /// Claude Code line says it instead (MA-C9), and on a router that keeps none.
        let library: Standing?
        /// Whether it offers a sync of its Library now: online, on a router that keeps one.
        let syncs: Bool

        /// Online is drawn as a session at work is, offline as one that has ended.
        var look: StateLook {
            online
                ? StateLook(color: StateColor.of(tone: .busy), hollow: false, halo: true, label: "online")
                : StateLook(color: StateColor.of(tone: .closed), hollow: true, halo: false, label: "offline")
        }
    }

    struct Build: Equatable, Sendable {
        let same: Bool
        let text: String
    }

    /// One client of a machine, as the screen lists it (MA-C8).
    struct ClientItem: Equatable, Sendable, Identifiable {
        /// Its id as the agent sent it: `claude-code`, `kimi-code`.
        let client: String
        /// "Claude Code", "Kimi Code".
        let name: String
        /// Its command line's version, each editor's extension, and whether its home folder is there.
        let parts: [String]
        /// Where its own copy of the Library stands; nil when it has none of its own (MA-C9).
        let library: Standing?
        var id: String { client }
        /// "Claude Code: CLI 2.1.267 · VS Code 2.1.296 · home folder, Library in step".
        var line: String { "\(name): " + parts.joined(separator: " · ") + (library.map { ", \($0.text)" } ?? "") }
    }

    /// A Library standing in words, and whether it is something to act on (LB-C11).
    struct Standing: Equatable, Sendable {
        let text: String
        let needs: Bool
    }

    static func standing(_ s: LibraryStanding) -> Standing {
        let n = { (v: Int?) in v ?? 0 }
        switch s.state {
        case "in_step": return Standing(text: "Library in step", needs: false)
        case "behind": return Standing(text: "Library behind on \(n(s.behind)) item\(n(s.behind) == 1 ? "" : "s")", needs: true)
        case "failed": return Standing(text: "Library: \(n(s.failed)) item\(n(s.failed) == 1 ? "" : "s") could not be placed", needs: true)
        case "replaced": return Standing(text: "Library in step; it put back \((s.replaced ?? []).joined(separator: ", ")) over a copy changed there", needs: false)
        case "predates": return Standing(text: "Its agent predates the Library — run the installer on it again", needs: true)
        case "opted_out": return Standing(text: "Takes no part in the Library", needs: false)
        default: return Standing(text: "Nothing heard of its Library yet", needs: false)
        }
    }

    /// A machine's answer to "sync now", in words (LB-C10, LB-C12).
    static func told(_ t: LibraryTold) -> String {
        switch t.outcome {
        case "synced":
            let changed = (t.report?.items ?? []).filter { $0.outcome != "in_step" }.count
            return changed > 0 ? "\(t.label): synced, \(changed) changed" : "\(t.label): synced, already in step"
        case "predates": return "\(t.label): its agent predates the Library — run the installer on it again"
        case "opted_out": return "\(t.label): takes no part in the Library"
        case "offline": return "\(t.label): offline — it syncs when it connects again"
        default: return "\(t.label): failed" + (t.why.map { " — \($0)" } ?? "")
        }
    }

    /// What a machine that reported no client says (MA-C10).
    static let noClients = "Its agent has reported no Claude Code, Kimi Code or Codex."

    /// An editor by its name, from the folder name the agent sends (src/clients.rs, `EDITORS`);
    /// one the console does not know, as sent (MA-C8).
    static func editorName(_ editor: String?) -> String {
        let e = editor?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return ["vscode": "VS Code", "vscode-insiders": "VS Code Insiders", "vscode-server": "VS Code Server",
                "cursor": "Cursor", "windsurf": "Windsurf", "vscodium": "VSCodium"][e] ?? e
    }

    /// A client's parts in words: "CLI 2.1.267", "CLI, version not read" or "no CLI"; each
    /// editor's extension, "VS Code 2.1.296", or "no editor extension"; "home folder" or
    /// "no home folder" (MA-C8).
    static func clientParts(_ c: MachineClient) -> [String] {
        let cli = c.cli.map { $0.version.map { "CLI \($0)" } ?? "CLI, version not read" } ?? "no CLI"
        let extensions = (c.extensions ?? []).map { x in [editorName(x.editor), x.version ?? ""].filter { !$0.isEmpty }.joined(separator: " ") }
        return [cli] + (extensions.isEmpty ? ["no editor extension"] : extensions) + [c.home != nil ? "home folder" : "no home folder"]
    }

    /// A machine's clients, each ending with where its own copy of the Library stands: Claude
    /// Code's is the machine's standing, every other client's is under its name (MA-C8, MA-C9).
    static func clientItems(_ m: MachineRow) -> [ClientItem] {
        m.clients.map { c in
            let own = c.client == "claude-code" ? m.library : m.library?.clients?[c.client]
            return ClientItem(client: c.client, name: SessionClient.productName(c.client), parts: clientParts(c), library: own.map(standing))
        }
    }

    /// A platform in the reader's words — macOS, Linux, Windows — and one this build does
    /// not know as the agent sent it (MA-C3).
    static func platform(_ platform: String) -> String {
        let p = platform.trimmingCharacters(in: .whitespacesAndNewlines)
        switch p.lowercased() {
        case "macos": return "macOS"
        case "linux": return "Linux"
        case "windows": return "Windows"
        default: return p
        }
    }

    /// Whether a machine is online, and for how long it has been online or offline, measured
    /// from `since_ms` on this phone's clock: the list carries no `as_of_ms` of the router's,
    /// as a session row does, and the seconds two clocks differ by do not move an hour or a
    /// day. With no `since_ms`, only which (MA-C3).
    static func presence(online: Bool, sinceMs: Int?, now: Date) -> String {
        let word = online ? "online" : "offline"
        guard let sinceMs, sinceMs > 0, let since = Fmt.date(ms: sinceMs) else { return word }
        return "\(word) for \(Fmt.duration(now.timeIntervalSince(since)))"
    }

    /// A machine's build against the router's: nothing until the router's health has named
    /// its build, and nothing for a machine that named none (MA-C4).
    static func build(_ machine: String?, router: String?) -> Build? {
        guard let router, !router.isEmpty, let machine, !machine.isEmpty else { return nil }
        return machine == router ? Build(same: true, text: sameBuild) : Build(same: false, text: otherBuild)
    }

    /// Every machine the router has seen, as the screen lists them: online first, then by
    /// label in the order the session list heads its machines (MA-C3, SE-C220). The router
    /// sorts them too, but by bytes, which puts "Zed" before "alpha".
    static func items(_ rows: [MachineRow], routerBuild: String?, now: Date) -> [Item] {
        rows.map { m in
            let label = m.label.trimmingCharacters(in: .whitespacesAndNewlines)
            let id = m.id.trimmingCharacters(in: .whitespacesAndNewlines)
            let clients = clientItems(m)
            return Item(id: m.id,
                        label: !label.isEmpty ? label : !id.isEmpty ? String(id.prefix(8)) : SessionGroups.unnamedMachine,
                        platform: platform(m.platform),
                        online: m.online,
                        presence: presence(online: m.online, sinceMs: m.sinceMs, now: now),
                        build: build(m.build, router: routerBuild),
                        clients: clients,
                        // Claude Code's line says the machine's own standing, so it is not said twice.
                        library: m.library != nil && !clients.contains { $0.client == "claude-code" && $0.library != nil } ? m.library.map(standing) : nil,
                        syncs: m.online && m.library != nil)
        }
        .sorted { a, b in
            if a.online != b.online { return a.online }
            if a.label != b.label { return SessionGroups.alphabetically(a.label, b.label) }
            return a.id < b.id
        }
    }

    /// How many are online and how many not, at a glance over the list: "2 online · 1
    /// offline" (MA-C3).
    static func tally(_ items: [Item]) -> String {
        let online = items.filter(\.online).count
        let offline = items.count - online
        return [online > 0 ? "\(online) online" : nil, offline > 0 ? "\(offline) offline" : nil]
            .compactMap { $0 }
            .joined(separator: " · ")
    }
}
