// Turning what was picked into what the tool expects — and into what a person would
// have typed, for when the prompt cannot be answered directly.

import Foundation

enum AskAnswers {
    struct Selection: Equatable, Sendable {
        var chosen: Set<String> = []
        var other: String = ""
    }

    /// `question text -> answer`, plus one line per answered question. Option order is
    /// the tool's own; "Other" is whatever was typed, and typing it is choosing it.
    static func compose(_ questions: [AskQuestion], _ selections: [String: Selection])
        -> (answers: [String: String], parts: [String]) {
        var answers: [String: String] = [:]
        var parts: [String] = []
        for q in questions {
            guard let sel = selections[q.question] else { continue }
            var picked = (q.options ?? []).map(\.label).filter { sel.chosen.contains($0) }
            let other = sel.other.trimmingCharacters(in: .whitespacesAndNewlines)
            if !other.isEmpty { picked.append(other) }
            guard !picked.isEmpty else { continue }
            let a = picked.joined(separator: ", ")
            answers[q.question] = a
            parts.append("\(q.title): \(a)")
        }
        return (answers, parts)
    }
}

/// Assistant prose, split at code fences so code can be set in monospace and left
/// unwrapped while everything else keeps inline markdown.
enum Markdown {
    enum Segment: Equatable {
        case prose(String)
        case code(String)
    }

    static func split(_ src: String) -> [Segment] {
        var out: [Segment] = []
        var inCode = false
        var buf: [String] = []
        func flush() {
            let text = buf.joined(separator: "\n").trimmingCharacters(in: .newlines)
            if !text.isEmpty { out.append(inCode ? .code(text) : .prose(text)) }
            buf = []
        }
        for line in src.components(separatedBy: "\n") {
            if line.trimmingCharacters(in: .whitespaces).hasPrefix("```") {
                flush()
                inCode.toggle()
                continue
            }
            buf.append(line)
        }
        flush()
        return out
    }

    /// The blocks the web view's `md()` draws: fenced code, headings, bullet and
    /// numbered lists, quotes, pipe tables, and paragraphs. Inline styling is applied
    /// per block by `inline`.
    enum Block: Equatable {
        case paragraph(String)
        case heading(Int, String)
        case bullets([String])
        case numbered([String])
        case quote(String)
        case code(String)
        case table([[String]])
    }

    static func blocks(_ src: String) -> [Block] {
        var out: [Block] = []
        var para: [String] = []
        var bullets: [String] = []
        var numbered: [String] = []
        var quote: [String] = []
        var table: [[String]] = []
        var code: [String]? = nil
        func flush() {
            if !para.isEmpty { out.append(.paragraph(para.joined(separator: "\n"))); para = [] }
            if !bullets.isEmpty { out.append(.bullets(bullets)); bullets = [] }
            if !numbered.isEmpty { out.append(.numbered(numbered)); numbered = [] }
            if !quote.isEmpty { out.append(.quote(quote.joined(separator: "\n"))); quote = [] }
            if !table.isEmpty { out.append(.table(table)); table = [] }
        }
        func cells(_ line: String) -> [String] {
            var t = line.trimmingCharacters(in: .whitespaces)
            if t.hasPrefix("|") { t.removeFirst() }
            if t.hasSuffix("|") { t.removeLast() }
            return t.split(separator: "|", omittingEmptySubsequences: false).map { $0.trimmingCharacters(in: .whitespaces) }
        }
        let isRow = { (l: String) -> Bool in
            let t = l.trimmingCharacters(in: .whitespaces)
            return t.hasPrefix("|") && t.hasSuffix("|") && t.count > 1
        }
        let isRule = { (l: String) -> Bool in
            isRow(l) && l.allSatisfy { "|-: ".contains($0) }
        }
        let lines = src.components(separatedBy: "\n")
        var i = 0
        while i < lines.count {
            let raw = lines[i]
            let line = raw.trimmingCharacters(in: .whitespaces)
            defer { i += 1 }
            if let c = code {
                if line.hasPrefix("```") { out.append(.code(c.joined(separator: "\n"))); code = nil }
                else { code!.append(raw) }
                continue
            }
            if line.hasPrefix("```") { flush(); code = []; continue }
            if line.isEmpty { flush(); continue }
            if isRow(line) {
                if table.isEmpty {
                    // A table is a header row followed by a |---| rule; without the rule
                    // a line of pipes is prose and must stay prose.
                    if i + 1 < lines.count, isRule(lines[i + 1]) {
                        flush(); table = [cells(line)]; i += 1; continue
                    }
                } else if !isRule(line) {
                    table.append(cells(line)); continue
                }
            } else if !table.isEmpty {
                flush()
            }
            if let h = line.firstIndex(where: { $0 != "#" }), line.hasPrefix("#"), line[h] == " " {
                flush()
                out.append(.heading(min(line.distance(from: line.startIndex, to: h), 6), String(line[h...]).trimmingCharacters(in: .whitespaces)))
                continue
            }
            if line.hasPrefix("- ") || line.hasPrefix("* ") {
                if !para.isEmpty || !numbered.isEmpty || !quote.isEmpty { flush() }
                bullets.append(String(line.dropFirst(2)))
                continue
            }
            if let dot = line.firstIndex(where: { $0 == "." || $0 == ")" }),
               line[..<dot].allSatisfy(\.isNumber), !line[..<dot].isEmpty,
               line.index(after: dot) < line.endIndex, line[line.index(after: dot)] == " " {
                if !para.isEmpty || !bullets.isEmpty || !quote.isEmpty { flush() }
                numbered.append(String(line[line.index(dot, offsetBy: 2)...]))
                continue
            }
            if line.hasPrefix(">") {
                if !para.isEmpty || !bullets.isEmpty || !numbered.isEmpty { flush() }
                quote.append(String(line.dropFirst()).trimmingCharacters(in: .whitespaces))
                continue
            }
            if !bullets.isEmpty || !numbered.isEmpty || !quote.isEmpty { flush() }
            para.append(line)
        }
        if let c = code { out.append(.code(c.joined(separator: "\n"))) }
        flush()
        return out
    }

    static func inline(_ s: String) -> AttributedString {
        (try? AttributedString(markdown: s,
                               options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)))
            ?? AttributedString(s)
    }
}
