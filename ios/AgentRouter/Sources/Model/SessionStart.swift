// Starting a session from the phone, as the console's Start a session does
// (ui/src/model/session.ts, `startMachines`, `startModels`, `startBody`;
// ui/src/screens/Sessions/StartSession.tsx): the router hosts it, on the machine named on a
// hosted router or on itself on a local one, so it takes messages, answers and a Stop from here.

import Foundation

/// One model of `GET /api/models`: the router's catalogue, each model with its provider, the
/// account assigned to it, if any, and the account serving it now.
struct CatalogueModel: Decodable, Sendable, Hashable, Identifiable {
    let id: String
    let provider: String
    let displayName: String?
    let assigned: String?
    let serving: String?
    var name: String { displayName.flatMap { $0.isEmpty ? nil : $0 } ?? id }
    enum CodingKeys: String, CodingKey { case id, provider, assigned, serving, displayName = "display_name" }
}

/// What `POST /api/live/start` takes.
struct StartBody: Encodable, Sendable, Equatable {
    /// The machine to start it on, on a hosted router, which relays it there.
    let machine: String?
    let cwd: String
    let model: String
    /// The client, sent for Codex; Kimi Code is the router's default.
    let client: String?
}

/// What it answers: the session it started.
struct StartReply: Decodable, Sendable, Verdict {
    let ok: Bool
    let session: String?
}

enum SessionStart {
    static let title = "Start a session"
    static let why = "The router hosts the session there, so it takes messages, answers and a Stop from here."

    /// The clients a session can be started with, by their names.
    static let clients = [SessionClient.kimiCode]

    struct Machine: Identifiable, Equatable, Sendable {
        let id: String
        let label: String
    }

    /// The machines a session can be started on: online, with the client among the ones their
    /// agent reported, by label in the order the list heads its machines.
    static func machines(_ rows: [MachineRow], client: String) -> [Machine] {
        rows.filter { $0.online && $0.clients.contains { $0.client == client } }
            .map { m in
                let label = m.label.trimmingCharacters(in: .whitespacesAndNewlines)
                return Machine(id: m.id, label: label.isEmpty ? String(m.id.prefix(8)) : label)
            }
            .sorted { SessionGroups.alphabetically($0.label, $1.label) }
    }

    /// The models a session can be started with: a Kimi Code session's are the catalogue's Kimi
    /// Code models, in its order, each by its display name.
    static func models(_ rows: [CatalogueModel], client: String) -> [CatalogueModel] {
        rows.filter { client == SessionClient.codex || $0.provider == "kimi" }
    }

    /// What a start sends: the machine on a hosted router, none on a local one, which starts the
    /// session on itself; the client only for Codex.
    static func body(hosted: Bool, machine: String, cwd: String, model: String, client: String) -> StartBody {
        StartBody(machine: hosted ? machine : nil, cwd: cwd.trimmingCharacters(in: .whitespacesAndNewlines), model: model,
                  client: client == SessionClient.codex ? client : nil)
    }
}
