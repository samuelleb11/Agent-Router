// The session list, arranged the way the web view arranges it — so a phone and a
// laptop looking at the same router agree about what is where.

import Foundation

struct SessionGroups: Equatable {
    struct Project: Identifiable, Equatable {
        let name: String
        /// Newest first: within one project, recency is what you are choosing between.
        let sessions: [LiveSession]
        var id: String { name }
        var live: Bool { sessions.contains(where: \.active) }
        var newest: Date { sessions.map(\.lastActivity).max() ?? .distantPast }
    }

    /// Moved within the window, or doing something now.
    let recent: [Project]
    let older: [Project]
    /// Put away by the reader, and not showing life since.
    let archived: [LiveSession]

    static let window: TimeInterval = 12 * 3600

    /// A session belongs on screen if it has moved recently or is doing something now.
    /// Archiving hides it — until it shows life again, because putting something away
    /// says "I am done with this", not "never show me this".
    static func isCurrent(_ r: LiveSession, now: Date) -> Bool {
        r.active || r.state == "working" || r.state == "ask"
            || now.timeIntervalSince(r.lastActivity) <= window
    }

    static func build(_ rows: [LiveSession], now: Date = .now) -> SessionGroups {
        let shelved = rows.filter { $0.archived && !isCurrent($0, now: now) }
        let rest = rows.filter { !($0.archived && !isCurrent($0, now: now)) }
        let recent = rest.filter { now.timeIntervalSince($0.lastActivity) <= window }
        let older = rest.filter { now.timeIntervalSince($0.lastActivity) > window }
        return SessionGroups(recent: group(recent), older: group(older),
                             archived: shelved.sorted { $0.lastActivityMs > $1.lastActivityMs })
    }

    /// Projects alphabetically, so a project keeps its place instead of jumping to the
    /// top whenever it moves.
    private static func group(_ rows: [LiveSession]) -> [Project] {
        let by = Dictionary(grouping: rows, by: \.projectName)
        return by.keys.sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }.map { name in
            Project(name: name, sessions: by[name]!.sorted { $0.lastActivityMs > $1.lastActivityMs })
        }
    }
}

enum StateColor {
    static func of(_ state: String?) -> ColorName {
        switch state {
        case "working", "tool": .green
        case "ask": .orange
        case "stalled": .yellow
        case "failed": .red
        case "thinking": .purple
        default: .gray
        }
    }
    enum ColorName { case green, orange, yellow, red, purple, gray }
}
