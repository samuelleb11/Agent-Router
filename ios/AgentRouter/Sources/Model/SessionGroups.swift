// The session list, arranged the way the web view arranges it — so a phone and a
// laptop looking at the same router agree about what is where.

import Foundation

struct SessionGroups: Equatable {
    struct Project: Identifiable, Equatable {
        let name: String
        /// Newest first: within one project, recency is what you are choosing between.
        let sessions: [LiveSession]
        var id: String { name }
        var live: Bool { sessions.contains(where: \.isLive) }
        var newest: Date { sessions.map(\.lastActivity).max() ?? .distantPast }
        /// The header's dot: the most pressing thing any of its sessions is doing — waiting
        /// on you, then an error, then a stall, then work, then idle — and the empty ring
        /// only once every one of them has ended. One ranking on the web view and both
        /// phones, so a project reads the same wherever it is looked at: a stalled or failed
        /// session is not hidden behind the work beside it. The halo is whether anything in
        /// it is live, as on the web view. From an older router, whose rows name no
        /// activity, simply whether anything is live, as it always was.
        var look: StateLook {
            guard sessions.contains(where: { $0.activity != nil }) else {
                return live ? StateLook(color: StateColor.of(tone: .busy), hollow: false, halo: true, label: "working")
                            : StateLook(color: StateColor.of(tone: .idle), hollow: false, halo: false, label: "idle")
            }
            let tones = sessions.map { SessionTone.effective($0.tone, $0.activity) }
            let ranked: [(SessionTone, String)] = [
                (.attention, "waiting for you"), (.error, "failed"), (.warn, "stalled"), (.busy, "working"),
            ]
            for (tone, label) in ranked where tones.contains(tone) {
                return StateLook(color: StateColor.of(tone: tone), hollow: false, halo: live, label: label)
            }
            if tones.allSatisfy({ $0 == .closed }) {
                return StateLook(color: StateColor.of(tone: .closed), hollow: true, halo: false, label: "ended")
            }
            // Idle: some session is still open with its turn over — or says something this
            // build does not know, which is neutral, and no reason to call the project ended.
            return StateLook(color: StateColor.of(tone: .idle), hollow: false, halo: live, label: "idle")
        }
    }

    /// Moved within the window, or doing something now.
    let recent: [Project]
    let older: [Project]
    /// Put away by the reader, and not showing life since.
    let archived: [LiveSession]

    static let window: TimeInterval = 12 * 3600

    /// A session belongs on screen if it has moved recently or is doing something now.
    /// Used for ordering the visible list, not for deciding what is on the shelf.
    static func isCurrent(_ r: LiveSession, now: Date) -> Bool {
        r.isLive || r.state == "working" || r.state == "ask"
            || now.timeIntervalSince(r.lastActivity) <= window
    }

    static func build(_ rows: [LiveSession], now: Date = .now) -> SessionGroups {
        // `archived` arrives from the router already meaning "put away, and quiet ever
        // since" — it clears itself the moment the session does something new. So the
        // shelf is exactly what the router says it is. Re-testing it here against a
        // recency window, as this once did, un-hid everything: the session you just
        // finished with had by definition moved a moment before you put it away.
        let shelved = rows.filter(\.archived)
        let rest = rows.filter { !$0.archived }
        // A session at work or waiting on you is never folded away, however long ago it
        // last wrote: a question can wait for hours, and a busy session can go minutes
        // without writing a line — the transcript is silent while the model thinks.
        let isRecent = { (r: LiveSession) in r.isLive || now.timeIntervalSince(r.lastActivity) <= window }
        let recent = rest.filter(isRecent)
        let older = rest.filter { !isRecent($0) }
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

extension SessionGroups {
    /// One machine's part of the list: its sessions, arranged exactly as a single machine's
    /// always were.
    struct Machine: Identifiable, Equatable {
        let id: String
        /// The machine's heading. Nil when every session comes from one machine or none — a
        /// local router names none — and the list has no machine headings at all.
        let name: String?
        let groups: SessionGroups
    }

    /// The heading for sessions the router listed without naming their machine, beside
    /// others it did name.
    static let unnamedMachine = "Unnamed machine"

    /// The list machine by machine, when a hosted router merges more than one machine's
    /// sessions: each machine under its own heading, alphabetically, and under it what
    /// `build` makes of that machine's sessions alone — so the same project on two machines
    /// stays two projects, as on the web view. With one machine or none, a single part with
    /// no heading that is exactly `build` of every row, so that list looks as it always did.
    static func byMachine(_ rows: [LiveSession], now: Date = .now) -> [Machine] {
        let names = Set(rows.compactMap(\.machineName))
        guard names.count > 1 else { return [Machine(id: "", name: nil, groups: build(rows, now: now))] }
        var parts = names.sorted(by: alphabetically).map { name in
            Machine(id: "machine:" + name, name: name, groups: build(rows.filter { $0.machineName == name }, now: now))
        }
        let unnamed = rows.filter { $0.machineName == nil }
        if !unnamed.isEmpty {
            parts.append(Machine(id: "unnamed", name: unnamedMachine, groups: build(unnamed, now: now)))
        }
        return parts
    }

    /// Case-insensitive, and the same order every time for names that differ only in case.
    private static func alphabetically(_ a: String, _ b: String) -> Bool {
        let order = a.localizedCaseInsensitiveCompare(b)
        return order == .orderedSame ? a < b : order == .orderedAscending
    }
}

enum StateColor {
    /// A legacy `state`, as this app has always coloured it — what an older router gets.
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

    /// The router's tone in this app's palette, which is the palette the legacy states
    /// already used: a phone reading a newer router colours a state the way it colours
    /// the same state from an older one. A tone this build does not know is neutral gray.
    static func of(tone: SessionTone) -> ColorName {
        switch tone {
        case .busy: .green
        case .attention: .orange
        case .warn: .yellow
        case .error: .red
        case .idle, .closed, .unknown: .gray
        }
    }

    enum ColorName: Sendable { case green, orange, yellow, red, purple, gray }
}

/// How a state dot is drawn — decided here, away from the view, so it can be tested.
struct StateLook: Equatable, Sendable {
    let color: StateColor.ColorName
    /// A ring with nothing inside: the session has ended, which is not the same as idle.
    let hollow: Bool
    /// The halo around a session that is doing something or waiting on you.
    let halo: Bool
    /// What VoiceOver says.
    let label: String

    /// The router's tone when it sent one this build knows, else the tone its activity
    /// implies — an activity this build does not know is neutral gray, not a guess —
    /// and, from a router that sends neither, the legacy state exactly as before.
    static func of(tone: SessionTone?, activity: SessionActivity?, state: String?, halo: Bool) -> StateLook {
        let t = SessionTone.effective(tone, activity)
        return StateLook(color: t.map(StateColor.of(tone:)) ?? StateColor.of(state),
                         hollow: t == .closed,
                         halo: halo,
                         label: activity?.spoken ?? state ?? "idle")
    }
}

extension LiveSession {
    /// The row's dot. The halo is the router's `active`, which a current router derives
    /// from the activity and an older one from recency — the halo rows always had.
    var look: StateLook { .of(tone: tone, activity: activity, state: state, halo: active) }
}

extension LiveStatus {
    /// The open view's header dot.
    var look: StateLook { .of(tone: tone, activity: activity, state: state, halo: isActive) }
}

/// What a session is doing, in words — the router's `detail` and how long it has held — worked
/// out here, away from the views, so the row and the header say it one way and it can be
/// tested. The web view's `doing()` is the reference; both phones follow it.
enum StatusText {
    /// `detail`, and for how long once that is more than a few seconds: under that the count
    /// would only flicker. The web view's threshold, so the two read alike.
    static func held(_ detail: String, for seconds: TimeInterval?) -> String {
        guard let seconds, seconds > 4 else { return detail }
        return "\(detail) · \(Fmt.duration(seconds))"
    }

    /// "Session ended · 3 h ago": both ends measured on the router's clock, so a phone whose
    /// clock is off says what the laptop says. Without both, the router's words alone.
    static func ended(_ detail: String, endedMs: Int?, asOfMs: Int?) -> String {
        guard let ended = Fmt.date(ms: endedMs), let asOf = Fmt.date(ms: asOfMs) else { return detail }
        return "\(detail) · \(Fmt.ago(ended, now: asOf))"
    }
}

extension LiveSession {
    /// The row's second line, under every session a current router lists — idle and ended
    /// included, as the web view shows them: "Waiting for you · 1h 00m", "Waiting for you ·
    /// background shell running · 10h 00m", "API error: 529 Overloaded · 1m 00s", "Session
    /// ended · 3 h ago". An idle session says how long it has waited, which is the question a
    /// glance at the list is asking. Nil from an older router, which names no activity: its
    /// rows stay exactly as they were.
    var doing: String? {
        guard let activity, let detail, !detail.isEmpty else { return nil }
        if activity == .closed { return StatusText.ended(detail, endedMs: endedMs, asOfMs: asOfMs) }
        return StatusText.held(detail, for: elapsed)
    }
}

extension LiveStatus {
    /// The open view's header line. `quiet_ms` is measured on the router, so the count is
    /// right whatever this phone's clock says, and it ticks because the router re-sends the
    /// status every second. An ended session says how long ago instead, on the router's
    /// clock too — the same words its row uses.
    var line: String {
        var line = activity == .closed
            ? StatusText.ended(detail, endedMs: endedMs, asOfMs: asOfMs)
            : StatusText.held(detail, for: TimeInterval(quietMs) / 1000)
        if let q = queued, q > 0 { line += " · \(q) queued" }
        return line
    }
}
