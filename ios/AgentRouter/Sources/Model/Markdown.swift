// Assistant prose, read exactly as the web console reads it: `md()` and `inline()` in
// ui/src/model/markdown.ts, which is the reference for both phones (decision F in
// docs/web-console-carbon.md; SE-C90 to SE-C104 in docs/web-console-contracts.md), save that its
// lists follow the phones' rule, which the web now shares. The fixtures the three clients share,
// fixtures/live/markdown-blocks.json, hold all three to it (SharedFixtureTests).
//
// The web reads Markdown with regular expressions. This reads the same patterns character by
// character, so the two cannot disagree about what a space is (JavaScript's `\s`), where a line
// ends, or where a fence may open and close: anywhere, not only at the start of a line. Lists were
// decided on 2026-10-05 (decision F, SE-C99) as the phones always drew them: a numbered list is
// drawn numbered, and a run that mixes bullets and numbers splits into a list of each. The web
// console has drawn them the same way since.

import Foundation

enum Markdown {
    enum Block: Equatable {
        /// One line of prose, the whitespace that starts it kept (SE-C103).
        case paragraph(String)
        /// The level the web draws: `#` is the third, `##` the fourth, `###` the fifth and anything
        /// deeper the sixth, so a heading sits under the screen's own (SE-C98).
        case heading(Int, String)
        case bullets([String])
        /// Drawn 1., 2., …, as the web draws it too (decision F, SE-C99).
        case numbered([String])
        case quote(String)
        /// Verbatim (SE-C93).
        case code(String)
        /// The head row first, then the body (SE-C101).
        case table([[String]])
    }

    /// A stretch of a line's text and its styles; `href` makes it a link (SE-C91).
    struct Run: Equatable, Sendable {
        var text: String
        var bold = false
        var italic = false
        var code = false
        var href: String?
    }

    // ---------- blocks ----------

    /// The blocks the web's `md()` draws.
    static func blocks(_ src: String) -> [Block] {
        let text = Array(src.unicodeScalars)
        var out: [Block] = []
        var at = 0
        // A fence runs from three backticks to the next three, wherever on a line they fall. An
        // unterminated one is not code: its backticks and everything after them stay prose
        // (SE-C94).
        while let open = fence(text, from: at), let close = fence(text, from: open + 3) {
            prose(text[at..<open], into: &out)
            out.append(.code(fenced(text[(open + 3)..<close])))
            at = close + 3
        }
        prose(text[at...], into: &out)
        return out
    }

    private static func fence(_ s: [Unicode.Scalar], from: Int) -> Int? {
        var i = from
        while i + 2 < s.count {
            if s[i] == "`", s[i + 1] == "`", s[i + 2] == "`" { return i }
            i += 1
        }
        return nil
    }

    /// A fence's code: its first line dropped when it is a language tag — one word, or nothing — and
    /// a newline before the closing fence dropped (SE-C93). Opened and closed on one line, the
    /// whole of it is code (SE-C104).
    private static func fenced(_ inner: ArraySlice<Unicode.Scalar>) -> String {
        var code = inner
        if let nl = inner.firstIndex(of: "\n"), isTag(inner[inner.startIndex..<nl]) { code = inner[(nl + 1)...] }
        if code.last == "\n" { code = code.dropLast() }
        return string(code)
    }

    /// `^[ \t]*[^\s`]*[ \t]*$`
    private static func isTag(_ line: ArraySlice<Unicode.Scalar>) -> Bool {
        var i = line.startIndex
        while i < line.endIndex, line[i] == " " || line[i] == "\t" { i += 1 }
        while i < line.endIndex, !isSpace(line[i]), line[i] != "`" { i += 1 }
        while i < line.endIndex, line[i] == " " || line[i] == "\t" { i += 1 }
        return i == line.endIndex
    }

    /// A stretch of prose, line by line: a list, a table, a heading, a quote, or else a paragraph
    /// of its own; consecutive lines are not joined, and blank lines add nothing (SE-C103).
    private static func prose(_ src: ArraySlice<Unicode.Scalar>, into out: inout [Block]) {
        if src.isEmpty { return }
        let lines = src.split(separator: "\n", omittingEmptySubsequences: false).map { trimEnd(Array($0)) }
        var list: [(numbered: Bool, text: String)] = []
        var table: (head: [String], rows: [[String]])?
        func flushList() {
            // Bullets and numbers are never one list: a run that mixes them is a list of each, on
            // the web as here (decision F, SE-C99).
            var i = 0
            while i < list.count {
                var j = i
                while j < list.count, list[j].numbered == list[i].numbered { j += 1 }
                let items = list[i..<j].map(\.text)
                out.append(list[i].numbered ? .numbered(items) : .bullets(items))
                i = j
            }
            list = []
        }
        func flushTable() {
            if let t = table { out.append(.table([t.head] + t.rows)) }
            table = nil
        }
        var i = 0
        while i < lines.count {
            let line = lines[i]
            let row = isRow(line)
            // A table is a row of pipes immediately followed by a separator row (SE-C101).
            if row, table == nil, i + 1 < lines.count, isRule(lines[i + 1]) {
                flushList()
                table = (cells(line), [])
                i += 2
                continue
            }
            i += 1
            if table != nil {
                if row, !isRule(line) {
                    table?.rows.append(cells(line))
                    continue
                }
                flushTable()
            }
            if let item = bullet(line) {
                list.append((false, item))
                continue
            }
            if let item = numbered(line) {
                list.append((true, item))
                continue
            }
            flushList()
            if let h = heading(line) {
                out.append(.heading(h.level, h.text))
            } else if let q = quote(line) {
                out.append(.quote(q))
            } else if line.contains(where: { !isSpace($0) }) {
                out.append(.paragraph(string(line)))
            }
        }
        flushList()
        flushTable()
    }

    /// `^\s*[-*]\s+(.*)$`: indented or not, so nothing nests (SE-C99); a `* ` marker is never
    /// emphasis, because a line's block is decided before its inline formatting (SE-C97).
    private static func bullet(_ l: [Unicode.Scalar]) -> String? {
        var i = 0
        while i < l.count, isSpace(l[i]) { i += 1 }
        guard i < l.count, l[i] == "-" || l[i] == "*" else { return nil }
        return rest(l, after: i + 1)
    }

    /// `^\s*\d+[.)]\s+(.*)$`
    private static func numbered(_ l: [Unicode.Scalar]) -> String? {
        var i = 0
        while i < l.count, isSpace(l[i]) { i += 1 }
        let digits = i
        while i < l.count, ("0"..."9").contains(l[i]) { i += 1 }
        guard i > digits, i < l.count, l[i] == "." || l[i] == ")" else { return nil }
        return rest(l, after: i + 1)
    }

    /// `^(#{1,6})\s+(.*)$`, at the level the web draws it; seven `#`, or a `#` with no space after
    /// it, is prose (SE-C98).
    private static func heading(_ l: [Unicode.Scalar]) -> (level: Int, text: String)? {
        var h = 0
        while h < l.count, l[h] == "#" { h += 1 }
        guard (1...6).contains(h), let text = rest(l, after: h) else { return nil }
        return (min(h + 2, 6), text)
    }

    /// `^>\s?(.*)$`: each quoted line a quote of its own (SE-C100).
    private static func quote(_ l: [Unicode.Scalar]) -> String? {
        guard l.first == ">" else { return nil }
        let from = l.count > 1 && isSpace(l[1]) ? 2 : 1
        guard !l[from...].contains(where: endsLine) else { return nil }
        return string(l[from...])
    }

    /// `\s+(.*)$` from `at`: at least one space, then the rest of the line, all of which `.` reads.
    private static func rest(_ l: [Unicode.Scalar], after at: Int) -> String? {
        guard at < l.count, isSpace(l[at]) else { return nil }
        var i = at
        while i < l.count, isSpace(l[i]) { i += 1 }
        guard !l[i...].contains(where: endsLine) else { return nil }
        return string(l[i...])
    }

    /// `^\s*\|.*\|\s*$`, of a line with nothing left to trim at its end.
    private static func isRow(_ l: [Unicode.Scalar]) -> Bool {
        var i = 0
        while i < l.count, isSpace(l[i]) { i += 1 }
        guard i < l.count, l[i] == "|", l.count - 1 > i, l[l.count - 1] == "|" else { return false }
        return !l[(i + 1)..<(l.count - 1)].contains(where: endsLine)
    }

    /// `^\s*\|[\s:|-]+\|\s*$`, likewise: without it, a line of pipes is prose (SE-C101).
    private static func isRule(_ l: [Unicode.Scalar]) -> Bool {
        var i = 0
        while i < l.count, isSpace(l[i]) { i += 1 }
        guard i < l.count, l[i] == "|", l.count - 1 > i + 1, l[l.count - 1] == "|" else { return false }
        return l[(i + 1)..<(l.count - 1)].allSatisfy { isSpace($0) || $0 == ":" || $0 == "|" || $0 == "-" }
    }

    /// A row's cells, trimmed, their inline formatting read later.
    private static func cells(_ l: [Unicode.Scalar]) -> [String] {
        var s = l[...]
        var i = s.startIndex
        while i < s.endIndex, isSpace(s[i]) { i += 1 }
        if i < s.endIndex, s[i] == "|" { s = s[(i + 1)...] }
        if let last = s.lastIndex(of: "|"), s[(last + 1)...].allSatisfy(isSpace) { s = s[..<last] }
        return s.split(separator: "|", omittingEmptySubsequences: false).map { string(trim($0)) }
    }

    // ---------- inline ----------
    //
    // The web reads a line's inline formatting in four passes — inline code, bold, italic, links —
    // each over the output of the one before, in which whatever an earlier pass built reads as one
    // character no pattern names. So bold can hold inline code, italic can hold bold, and a link's
    // text can hold all three, while nothing is ever read inside inline code (SE-C95).

    private enum Unit {
        case char(Unicode.Scalar)
        /// What an earlier pass built: its runs, and the text it was read from.
        case node([Run], source: [Unicode.Scalar])
    }

    /// The character a node stands for in what a later pass reads, one no pattern names.
    private static let opaque = Unicode.Scalar(UInt32(0xE000))!

    /// A line's inline formatting, as runs.
    static func runs(_ s: String) -> [Run] {
        let text = Array(s.unicodeScalars)
        // Inline code first, from one backtick to the next on the same line: verbatim (SE-C95).
        var units: [Unit] = []
        var i = 0
        while i < text.count {
            if text[i] == "`", let end = stop(text, from: i + 1, at: "`") {
                units.append(.node([Run(text: string(text[(i + 1)..<end]), code: true)], source: Array(text[i...end])))
                i = end + 1
            } else {
                units.append(.char(text[i]))
                i += 1
            }
        }
        // `**bold**`, on one line with no asterisk inside (SE-C96).
        units = pass(units) { s, i, units in
            guard s[i] == "*", i + 1 < s.count, s[i + 1] == "*", let end = stop(s, from: i + 2, at: "*"),
                  end + 1 < s.count, s[end + 1] == "*" else { return nil }
            return (end + 2, 0, styled(units[(i + 2)..<end]) { $0.bold = true })
        }
        // `*italic*`, only when its opening asterisk follows the start of the line, a space or "(",
        // so `d*no*e`, `*.rs` and `2 ** 8` stay as typed; underscores never mean emphasis (SE-C96).
        units = pass(units) { s, i, units in
            let lead: Int
            if i == 0, s[0] == "*" {
                lead = 0
            } else if isSpace(s[i]) || s[i] == "(", i + 1 < s.count, s[i + 1] == "*" {
                lead = 1
            } else {
                return nil
            }
            guard let end = stop(s, from: i + lead + 1, at: "*") else { return nil }
            return (end + 1, lead, styled(units[(i + lead + 1)..<end]) { $0.italic = true })
        }
        // `[text](https://…)`: only an http or https address is a link (SE-C91), and a bare one is
        // text (SE-C92). The address is read as it was typed.
        units = pass(units) { s, i, units in
            guard s[i] == "[", let close = stop(s, from: i + 1, at: "]"), close + 1 < s.count, s[close + 1] == "(" else { return nil }
            var k = close + 2
            guard k + 4 <= s.count, string(s[k..<(k + 4)]) == "http" else { return nil }
            k += 4
            if k + 1 < s.count, s[k] == "s", s[k + 1] == ":" {
                k += 2
            } else if k < s.count, s[k] == ":" {
                k += 1
            } else {
                return nil
            }
            var end = k
            while end < s.count, s[end] != ")", !isSpace(s[end]) { end += 1 }
            guard end > k, end < s.count, s[end] == ")" else { return nil }
            let href = string(source(units[(close + 2)..<end]))
            return (end + 1, 0, styled(units[(i + 1)..<close]) { $0.href = href })
        }
        return merged(inlines(units[...]))
    }

    /// The line drawn: each run in its styles, a link tappable.
    static func inline(_ s: String) -> AttributedString {
        var out = AttributedString()
        for run in runs(s) {
            var piece = AttributedString(run.text)
            var intent: InlinePresentationIntent = []
            if run.bold { intent.insert(.stronglyEmphasized) }
            if run.italic { intent.insert(.emphasized) }
            if run.code { intent.insert(.code) }
            if !intent.isEmpty { piece.inlinePresentationIntent = intent }
            if let href = run.href, let url = URL(string: href) { piece.link = url }
            out.append(piece)
        }
        return out
    }

    /// The end of a pattern's middle, `[^x\n]+x` read from `from`: the index of the closing `x`,
    /// with at least one character before it and no newline.
    private static func stop(_ s: [Unicode.Scalar], from: Int, at x: Unicode.Scalar) -> Int? {
        var j = from
        while j < s.count, s[j] != x, s[j] != "\n" { j += 1 }
        return j > from && j < s.count && s[j] == x ? j : nil
    }

    /// One pass: every match, leftmost first and never overlapping, becomes a node. `match` reads
    /// the pattern at one position of the line as the pass sees it, and says where the match ends,
    /// from which of its characters the node starts, and the node's runs.
    private static func pass(
        _ units: [Unit],
        _ match: ([Unicode.Scalar], Int, [Unit]) -> (end: Int, from: Int, runs: [Run])?
    ) -> [Unit] {
        let flat = units.map { unit -> Unicode.Scalar in
            if case .char(let c) = unit { return c }
            return opaque
        }
        var out: [Unit] = []
        var at = 0
        var i = 0
        while i < flat.count {
            guard let m = match(flat, i, units) else {
                i += 1
                continue
            }
            out.append(contentsOf: units[at..<(i + m.from)])
            out.append(.node(m.runs, source: source(units[(i + m.from)..<m.end])))
            at = m.end
            i = m.end
        }
        out.append(contentsOf: units[at...])
        return out
    }

    private static func inlines(_ units: ArraySlice<Unit>) -> [Run] {
        var out: [Run] = []
        var text: [Unicode.Scalar] = []
        for unit in units {
            switch unit {
            case .char(let c):
                text.append(c)
            case .node(let runs, _):
                if !text.isEmpty { out.append(Run(text: string(text))) }
                text = []
                out.append(contentsOf: runs)
            }
        }
        if !text.isEmpty { out.append(Run(text: string(text))) }
        return out
    }

    private static func styled(_ units: ArraySlice<Unit>, _ style: (inout Run) -> Void) -> [Run] {
        inlines(units).map { run in
            var run = run
            style(&run)
            return run
        }
    }

    private static func source(_ units: ArraySlice<Unit>) -> [Unicode.Scalar] {
        units.flatMap { unit -> [Unicode.Scalar] in
            switch unit {
            case .char(let c): [c]
            case .node(_, let source): source
            }
        }
    }

    /// Adjacent runs in the same styles as one, and no empty run.
    private static func merged(_ runs: [Run]) -> [Run] {
        var out: [Run] = []
        for run in runs where !run.text.isEmpty {
            if let last = out.last, last.bold == run.bold, last.italic == run.italic, last.code == run.code, last.href == run.href {
                out[out.count - 1].text += run.text
            } else {
                out.append(run)
            }
        }
        return out
    }

    // ---------- characters ----------

    /// JavaScript's `\s`, which is also what its `trim()` removes.
    private static func isSpace(_ c: Unicode.Scalar) -> Bool {
        switch c.value {
        case 0x09...0x0D, 0x20, 0xA0, 0x1680, 0x2000...0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000, 0xFEFF: true
        default: false
        }
    }

    /// What JavaScript's `.` never reads: the line terminators.
    private static func endsLine(_ c: Unicode.Scalar) -> Bool {
        switch c.value {
        case 0x0A, 0x0D, 0x2028, 0x2029: true
        default: false
        }
    }

    private static func trimEnd(_ s: [Unicode.Scalar]) -> [Unicode.Scalar] {
        var end = s.count
        while end > 0, isSpace(s[end - 1]) { end -= 1 }
        return Array(s[..<end])
    }

    private static func trim(_ s: ArraySlice<Unicode.Scalar>) -> ArraySlice<Unicode.Scalar> {
        var lo = s.startIndex
        var hi = s.endIndex
        while lo < hi, isSpace(s[lo]) { lo += 1 }
        while hi > lo, isSpace(s[hi - 1]) { hi -= 1 }
        return s[lo..<hi]
    }

    private static func string<S: Sequence>(_ scalars: S) -> String where S.Element == Unicode.Scalar {
        var view = String.UnicodeScalarView()
        view.append(contentsOf: scalars)
        return String(view)
    }
}
