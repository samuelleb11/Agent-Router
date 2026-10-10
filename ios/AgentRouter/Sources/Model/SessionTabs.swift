// The session list a machine at a time: when a hosted router lists sessions from more than
// one machine, a row of tabs over the list — All, then each machine — each saying what is
// at work there and what waits on you. Worked out here, away from the view, so it can be
// tested. The web console tabs its list the same way (`machineTabs`, SE-C226 to SE-C229).

import Foundation

enum SessionTabs {
    /// The first tab, which shows every machine under its heading, as the list always did.
    static let all = "All"

    /// Said once over the list when the machine whose tab was chosen has no sessions listed
    /// any more — it went offline, say — and the list shows every machine instead (SE-C229).
    static let unlisted = "That machine has no sessions listed now, so this is all of them."

    /// One tab over a list of several machines.
    struct Tab: Identifiable, Equatable, Sendable {
        /// The part it shows, by `SessionGroups.Machine.id`; nil for All, which shows them all.
        let machine: String?
        /// "All", the machine's name, or "Unnamed machine".
        let label: String
        /// What is at work there and what waits on you, in the words the routers list uses,
        /// "2 working · 1 waiting", or from an older router "3 active"; empty when nothing is.
        let counts: String
        /// Something there waits on the person: the tab carries the attention dot, so a
        /// prompt on a machine that is not on screen is not missed.
        let waiting: Bool

        var id: String { machine ?? "all" }

        /// What VoiceOver says, as the console names its tab: its label, then its counts, or
        /// "idle" — "Laptop, 1 working · 1 waiting" — so it says whether something waits.
        var spoken: String { "\(label), \(counts.isEmpty ? "idle" : counts)" }
    }

    /// The tabs over the list `SessionGroups.byMachine` made: All first, then one per machine
    /// in the order the list heads them, the unnamed part last. None when the list is not
    /// headed per machine — one machine's sessions, or a local router's — so it looks
    /// exactly as it always did.
    static func tabs(_ parts: [SessionGroups.Machine]) -> [Tab] {
        guard parts.count > 1 else { return [] }
        func tab(_ machine: String?, _ label: String, _ rows: [LiveSession]) -> Tab {
            Tab(machine: machine, label: label, counts: rows.liveSummary ?? "",
                waiting: (rows.activityCounts?.waiting ?? 0) > 0)
        }
        return [tab(nil, all, parts.flatMap(\.groups.rows))]
            + parts.map { tab($0.id, $0.name ?? SessionGroups.unnamedMachine, $0.groups.rows) }
    }

    /// What the list draws on the chosen tab: every part, each under its heading, on All —
    /// and for a selection no part has — and a machine's part alone, without its heading,
    /// on that machine's tab. A part keeps its id, so its older and archived folds are the
    /// same folds on both.
    static func parts(_ parts: [SessionGroups.Machine], showing selection: String?) -> [SessionGroups.Machine] {
        guard parts.count > 1, let selection, let part = parts.first(where: { $0.id == selection }) else { return parts }
        return [SessionGroups.Machine(id: part.id, name: nil, groups: part.groups)]
    }

    /// Whether the chosen machine has gone from the list, so the list falls back to All and
    /// says so. A machine still listed is not gone, even alone, when there are no tabs.
    static func isUnlisted(_ selection: String?, rows: [LiveSession]) -> Bool {
        guard let selection else { return false }
        return !rows.contains { SessionGroups.partId(machine: $0.machineName) == selection }
    }
}

extension SessionGroups {
    /// Every session in this part of the list: the recent and older projects' and the shelf.
    var rows: [LiveSession] { recent.flatMap(\.sessions) + older.flatMap(\.sessions) + archived }
}

extension Collection where Element == LiveSession {
    /// Sessions at work and sessions waiting on you, when the router names each session's
    /// activity. Nil from an older router, whose only word for either is `active`.
    var activityCounts: (working: Int, waiting: Int)? {
        guard contains(where: { $0.activity != nil }) else { return nil }
        return (filter(\.isWorking).count, filter(\.needsYou).count)
    }

    /// "2 working · 1 waiting", or from an older router "3 active"; nil when nothing is.
    /// An ended session is neither, so it is never counted. The routers list, a router's
    /// page and a machine's tab all say it this way.
    var liveSummary: String? {
        guard let c = activityCounts else {
            let active = filter(\.active).count
            return active > 0 ? "\(active) active" : nil
        }
        let parts = [c.working > 0 ? "\(c.working) working" : nil, c.waiting > 0 ? "\(c.waiting) waiting" : nil]
        let said = parts.compactMap { $0 }
        return said.isEmpty ? nil : said.joined(separator: " · ")
    }
}
