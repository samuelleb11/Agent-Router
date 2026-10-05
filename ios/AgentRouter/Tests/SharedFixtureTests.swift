import XCTest
@testable import AgentRouter

/// The fixtures the web view and both phones share, fixtures/live/markdown-blocks.json and
/// fixtures/live/session-groups.json, read from the repository and held against this app's own
/// Markdown blocks and session list. Their `expected` is generated from the web's model, the
/// reference for both phones (docs/session-liveness.md, "Client presentation policy"); each file's
/// `about` says what the shape means. A case marked `pending` is one where matching the web would
/// lose something here, and it is skipped, by name, until it is decided.
final class SharedFixtureTests: XCTestCase {
    func testMarkdownBlocksMatchTheSharedFixtures() throws {
        let fixture = try load(Fixture<MarkdownCase>.self, "markdown-blocks.json")
        XCTAssertFalse(fixture.cases.isEmpty)
        for c in fixture.cases where c.pending == nil {
            XCTAssertEqual(Shape.blocks(Markdown.blocks(c.input)), c.expected, c.name)
        }
    }

    func testSessionGroupsMatchTheSharedFixtures() throws {
        let fixture = try load(Fixture<GroupsCase>.self, "session-groups.json")
        XCTAssertFalse(fixture.cases.isEmpty)
        for c in fixture.cases where c.pending == nil {
            // What the list screen draws: one part under no machine's heading, for a local router
            // and for a hosted router's single machine.
            let parts = SessionGroups.byMachine(c.sessions, now: Date(timeIntervalSince1970: TimeInterval(c.now) / 1000))
            XCTAssertEqual(parts.map(\.name), [nil], c.name)
            guard parts.count == 1, let groups = parts.first?.groups else { continue }
            XCTAssertEqual(Shape.groups(groups), c.expected, c.name)
        }
    }

    /// Named here so the run says which cases wait on a decision, and what each client does.
    func testPendingCasesAreSkippedByName() throws {
        let markdown = try load(Fixture<MarkdownCase>.self, "markdown-blocks.json").cases.compactMap { c in
            c.pending.map { "markdown-blocks.json: \(c.name) (\($0))" }
        }
        let groups = try load(Fixture<GroupsCase>.self, "session-groups.json").cases.compactMap { c in
            c.pending.map { "session-groups.json: \(c.name) (\($0))" }
        }
        let pending = markdown + groups
        try XCTSkipIf(!pending.isEmpty, "\(pending.count) pending, skipped: \(pending.joined(separator: "; "))")
    }

    // ---------- the files ----------

    private struct Fixture<Case: Decodable>: Decodable {
        let cases: [Case]
    }

    private struct MarkdownCase: Decodable {
        let name: String
        let input: String
        let pending: String?
        let expected: [Shape.Block]
    }

    private struct GroupsCase: Decodable {
        let name: String
        let now: Int
        let sessions: [LiveSession]
        let pending: String?
        let expected: Shape.Groups
    }

    /// The repository's own copy, found from this source file: …/ios/AgentRouter/Tests/ is three
    /// folders below the root.
    private func load<T: Decodable>(_ type: T.Type, _ name: String) throws -> T {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let url = root.appendingPathComponent("fixtures/live").appendingPathComponent(name)
        return try JSONDecoder().decode(T.self, from: Data(contentsOf: url))
    }
}

/// The client-neutral shapes the fixtures are written in, and this app's values mapped to them.
private enum Shape {
    struct Run: Decodable, Equatable, CustomStringConvertible {
        var text: String
        var bold = false
        var italic = false
        var code = false
        var href: String?

        init(_ run: Markdown.Run) {
            (text, bold, italic, code, href) = (run.text, run.bold, run.italic, run.code, run.href)
        }

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: Keys.self)
            text = try c.decode(String.self, forKey: .text)
            bold = try c.decodeIfPresent(Bool.self, forKey: .bold) ?? false
            italic = try c.decodeIfPresent(Bool.self, forKey: .italic) ?? false
            code = try c.decodeIfPresent(Bool.self, forKey: .code) ?? false
            href = try c.decodeIfPresent(String.self, forKey: .href)
        }

        private enum Keys: String, CodingKey { case text, bold, italic, code, href }

        var description: String {
            let styles = [bold ? "bold" : nil, italic ? "italic" : nil, code ? "code" : nil, href.map { "href \($0)" }].compactMap { $0 }
            return styles.isEmpty ? "\"\(text)\"" : "\"\(text)\" (\(styles.joined(separator: ", ")))"
        }
    }

    struct Block: Decodable, Equatable {
        var type: String
        var level: Int?
        var ordered: Bool?
        var runs: [Run]?
        var items: [[Run]]?
        var lang: String?
        var text: String?
        var head: [[Run]]?
        var rows: [[[Run]]]?
    }

    struct Group: Decodable, Equatable {
        var project: String
        var tone: String?
        var halo: Bool
        var sessions: [String]
    }

    struct Groups: Decodable, Equatable {
        var recent: [Group]
        var older: [Group]
        var archived: [String]
    }

    /// A line's runs, as the screen draws them: adjacent runs in the same styles are one.
    static func runs(_ s: String) -> [Run] {
        var out: [Run] = []
        for run in Markdown.runs(s).map(Run.init) where !run.text.isEmpty {
            if let last = out.last, last.bold == run.bold, last.italic == run.italic, last.code == run.code, last.href == run.href {
                out[out.count - 1].text += run.text
            } else {
                out.append(run)
            }
        }
        return out
    }

    static func blocks(_ blocks: [Markdown.Block]) -> [Block] {
        blocks.map { block in
            switch block {
            case .paragraph(let s): Block(type: "paragraph", runs: runs(s))
            case .heading(let level, let s): Block(type: "heading", level: level, runs: runs(s))
            case .bullets(let items): Block(type: "list", ordered: false, items: items.map(runs))
            case .numbered(let items): Block(type: "list", ordered: true, items: items.map(runs))
            case .quote(let s): Block(type: "quote", runs: runs(s))
            // The language tag is dropped, as on the web (SE-C93).
            case .code(let s): Block(type: "code", lang: nil, text: s)
            case .table(let rows): Block(type: "table", head: (rows.first ?? []).map(runs), rows: rows.dropFirst().map { $0.map(runs) })
            }
        }
    }

    static func groups(_ g: SessionGroups) -> Groups {
        let project = { (p: SessionGroups.Project) in
            Group(project: p.name, tone: p.tone.map(name), halo: p.look.halo, sessions: p.sessions.map(\.sessionId))
        }
        return Groups(recent: g.recent.map(project), older: g.older.map(project), archived: g.archived.map(\.sessionId))
    }

    private static func name(_ tone: SessionTone) -> String {
        switch tone {
        case .busy: "busy"
        case .attention: "attention"
        case .warn: "warn"
        case .error: "error"
        case .idle: "idle"
        case .closed: "closed"
        case .unknown(let raw): raw
        }
    }
}
