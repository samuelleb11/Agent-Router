import Foundation

/// What `/api/search` answers with.
///
/// In its own file rather than appended to `Models.swift`: search is a self-contained
/// slice of the API, and that file is large and busy.
///
/// `expression` is the query the router actually ran. It differs from `query` when FTS5
/// refused what was typed — an apostrophe, a bare `AND`, an unclosed bracket — and the
/// router retried it as a plain AND of quoted words. Worth showing when they differ, so
/// a search that quietly changed shape does not look like a search that found nothing.
struct SearchResults: Decodable, Sendable {
    let query: String
    let expression: String?
    let sessions: [SearchSessionRow]
    let hits: [SearchHit]

    static let empty = SearchResults(query: "", expression: nil, sessions: [], hits: [])
}

/// One matching block, at one place it appeared.
struct SearchHit: Decodable, Sendable, Identifiable, Hashable {
    let blockId: Int
    let kind: String
    let chars: Int
    let sessionId: String
    let role: String
    let ordinal: Int
    let at: Int
    let project: String?
    let sessionLabel: String?
    /// Matches wrapped in \u{2039}…\u{203A}. See `SearchView.highlighted`.
    let snippet: String
    /// bm25: negative, and lower is a better match. The rows arrive already ordered.
    let rank: Double

    /// The same block can appear at several places, so the block alone is not an
    /// identity — a `List` given duplicate ids drops rows.
    var id: String { "\(sessionId)-\(blockId)-\(ordinal)" }

    var shortSession: String { String(sessionId.prefix(8)) }
    var projectName: String { project?.split(separator: "/").last.map(String.init) ?? "—" }

    /// What kind of thing matched, in words rather than in the wire's vocabulary.
    var kindName: String {
        switch kind {
        case "text": "message"
        case "tool_use": "tool call"
        case "tool_result": "tool result"
        case "thinking": "thinking"
        case "system": "system prompt"
        default: kind
        }
    }

    enum CodingKeys: String, CodingKey {
        case kind, chars, role, ordinal, at, project, snippet, rank
        case blockId = "block_id", sessionId = "session_id", sessionLabel = "session_label"
    }
}

/// A session that mentions the query, with how often.
struct SearchSessionRow: Decodable, Sendable, Identifiable, Hashable {
    let sessionId: String
    let project: String?
    let sessionLabel: String?
    let gitBranch: String?
    /// Distinct messages that matched, and places they matched. Counted by SQLite, but
    /// Double for the same reason every other count here is.
    let blocks: Double
    let hits: Double
    let firstAt: Int?
    let lastAt: Int?

    var id: String { sessionId }
    var shortId: String { String(sessionId.prefix(8)) }
    var projectName: String { project?.split(separator: "/").last.map(String.init) ?? "—" }

    enum CodingKeys: String, CodingKey {
        case project, blocks, hits
        case sessionId = "session_id", sessionLabel = "session_label"
        case gitBranch = "git_branch", firstAt = "first_at", lastAt = "last_at"
    }
}
