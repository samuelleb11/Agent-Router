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

    enum CodingKeys: String, CodingKey {
        case id, label, platform, version, build, online, sinceMs = "since_ms"
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
    }
}

enum Machines {
    // MARK: connecting a machine

    /// The screen's first section, and the link to it from a hosted router's empty session
    /// list (MA-C2, SE-C187).
    static let connect = "Connect a machine"

    /// The router serves the installer without a password; it installs the agent and asks
    /// there for the router's password (MA-C2).
    static let connectHow = "Run this on a machine to connect it — it asks for this router’s password."

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
            return Item(id: m.id,
                        label: !label.isEmpty ? label : !id.isEmpty ? String(id.prefix(8)) : SessionGroups.unnamedMachine,
                        platform: platform(m.platform),
                        online: m.online,
                        presence: presence(online: m.online, sinceMs: m.sinceMs, now: now),
                        build: build(m.build, router: routerBuild))
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
