// Which client a live session is, and what its controls can do from here, as the console
// decides it (ui/src/model/session.ts, `drives`, `rowClient`, `controlsOf`; ui/src/model/clients.ts):
// a Claude Code session under the wrapper takes everything; a Kimi Code session only while the
// router hosts it, and never an effort, a side question or a question's answer; a Codex session
// is watched from its transcript. Each refusal is in the router's own words.

import Foundation

/// What an open session's controls can do, and why not where they cannot.
struct SessionControls: Equatable, Sendable {
    /// Why the session takes no control at all, else nil: the composer, Stop, the permission
    /// card, the question form and the model and mode switchers are off, and this is said.
    let held: String?
    /// Why the effort switcher is off, else nil.
    let effort: String?
    /// Why the side question is off, else nil.
    let aside: String?
    /// Why a question's form is off, else nil.
    let question: String?
    /// The modes the switcher offers in place of the router's list: Kimi Code's for its sessions.
    let modes: [LiveMode]?
    /// The mode asked about first, since it stops the session asking about anything.
    let bypass: String

    static let every = SessionControls(held: nil, effort: nil, aside: nil, question: nil, modes: nil, bypass: "bypassPermissions")
}

enum SessionClient {
    static let kimiCode = "kimi-code"
    static let codex = "codex"

    static let kimiWatched = "This Kimi Code session runs in its own terminal or in VS Code, which takes no outside control. Start a hosted session to drive one."
    static let kimiNoEffort = "A Kimi Code session has no effort to set from here."
    static let kimiNoAside = "A Kimi Code session has no side question to ask from here."
    static let kimiQuestion = "A Kimi Code question is answered in its own window for now."
    static let codexWatched = "This Codex session is watched from its local transcript. Send messages and answer approvals in the terminal or IDE that owns it."

    /// Kimi Code's permission modes, offered in place of Claude Code's.
    static let kimiModes = [LiveMode(value: "manual", label: "Manual"), LiveMode(value: "auto", label: "Auto"), LiveMode(value: "yolo", label: "YOLO")]
    static let kimiYolo = "yolo"

    /// A product a machine's agent reports, by its name; one the console does not name, as sent.
    static func productName(_ id: String) -> String {
        switch id {
        case "claude-code": "Claude Code"
        case kimiCode: "Kimi Code"
        case codex: "Codex"
        default: id
        }
    }

    /// Kimi Code's: its row says so, or, before a list has listed it, its id, `session_<uuid>`
    /// where Claude Code's is a bare UUID.
    static func isKimi(_ id: String, client: String?) -> Bool { client == kimiCode || id.hasPrefix("session_") }
    static func isCodex(_ id: String, client: String?) -> Bool { client == codex || id.hasPrefix("codex:") }
}

extension LiveSession {
    var isKimi: Bool { SessionClient.isKimi(sessionId, client: client) }
    var isCodex: Bool { SessionClient.isCodex(sessionId, client: client) }

    /// Whether it takes a typed message and a Stop from here: a Kimi Code session when the router
    /// hosts it, a Claude Code one under the wrapper, a Codex one never (it is watched).
    var canControl: Bool {
        if isCodex { return false }
        return isKimi ? hosted == true : wrapped
    }

    /// The client its row names: "Kimi Code web" for a session the router hosts under Kimi Code's
    /// web server, "Kimi Code" for one it watches, "Codex"; none for Claude Code's.
    var clientName: String? {
        if isCodex { return SessionClient.productName(SessionClient.codex) }
        guard isKimi else { return nil }
        return hosted == true ? "Kimi Code web" : SessionClient.productName(SessionClient.kimiCode)
    }

    /// The provider its row can name: Kimi Code's, known by its client. A Claude Code row says
    /// none, since its requests may go to either, and nothing is marked rather than a guess.
    var markProvider: String? { isKimi ? "kimi" : nil }

    var controls: SessionControls {
        if isCodex {
            return SessionControls(held: SessionClient.codexWatched, effort: SessionClient.codexWatched, aside: SessionClient.codexWatched,
                                   question: SessionClient.codexWatched, modes: nil, bypass: "bypassPermissions")
        }
        guard isKimi else { return .every }
        let held = hosted == true ? nil : SessionClient.kimiWatched
        return SessionControls(held: held, effort: SessionClient.kimiNoEffort, aside: SessionClient.kimiNoAside,
                               question: held ?? SessionClient.kimiQuestion, modes: SessionClient.kimiModes, bypass: SessionClient.kimiYolo)
    }

    /// Why it takes no control from here, in the router's words; nil when it takes them.
    var watchedReason: String? { controls.held }
    var canSend: Bool { controls.held == nil }
    var canAskAside: Bool { controls.aside == nil }
}
