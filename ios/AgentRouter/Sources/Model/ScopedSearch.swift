// Search and a project's places as the Usage page's scope has them, as the console's Overview
// does (ui/src/model/search.ts; ui/src/screens/Overview/ProjectPlaces.tsx, panels.ts
// `projectPlaces`): a search from a scoped page searches that scope — its provider, account,
// project, session and the start of its range, never an end — and a scoped project says where
// it ran.

import Foundation

enum ScopedSearch {
    /// The query a search sends: the words and the facet, and the scope it was opened from.
    static func query(_ q: String, kind: String?, scope: UsageScope?, since: String?, limit: Int = 25) -> [String: String?] {
        ["q": q, "kind": kind, "limit": String(limit),
         "provider": scope?.provider, "account": scope?.account, "project": scope?.project, "session": scope?.session, "since": since]
    }

    /// What the page says it searches: everything, or the scope it was opened from.
    static func caption(scope: UsageScope?) -> String? {
        guard let scope, !scope.isEmpty else { return nil }
        return "Searching \(scope.title) only, over the range the page was on."
    }
}

/// Where a scoped project's sessions ran: its repository, and each directory by the machine it
/// is on. Nil with nothing to say — an unresolved project has no directory.
struct ProjectPlaces: Equatable, Sendable {
    struct Place: Equatable, Sendable {
        let machine: String?
        let path: String
        /// "studio-mac · /Users/you/code/app", or the path alone.
        var label: String { machine.map { "\($0) · \(path)" } ?? path }
    }

    let repository: String?
    let directories: [Place]

    static func of(_ key: String?, in rows: [ProjectRow]) -> ProjectPlaces? {
        guard let key, let row = rows.first(where: { $0.id == key }) else { return nil }
        let places = (row.directories ?? []).map { d in
            Place(machine: d.machine?.label.flatMap { $0.trimmingCharacters(in: .whitespaces).isEmpty ? nil : $0.trimmingCharacters(in: .whitespaces) }, path: d.cwd)
        }
        return row.repository != nil || !places.isEmpty ? ProjectPlaces(repository: row.repository, directories: places) : nil
    }
}
