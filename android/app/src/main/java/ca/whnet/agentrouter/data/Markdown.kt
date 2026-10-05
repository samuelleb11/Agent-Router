// Assistant prose, read exactly as the web console reads it: `md()` in
// ui/src/model/markdown.ts, the reference for both phones (decision F in
// docs/web-console-carbon.md; SE-C90 to SE-C104 in docs/web-console-contracts.md), save that its
// lists follow the phones' rule, which the web now shares. The fixtures the three clients share,
// fixtures/live/markdown-blocks.json, hold all three to it (SharedFixtureTest). A line's inline
// styling is [Inline]'s.
//
// The web reads Markdown with regular expressions. This reads the same patterns character by
// character, so the two cannot disagree about what a space is (JavaScript's `\s`), where a line
// ends, or where a fence may open and close: anywhere, not only at the start of a line. Lists were
// decided on 2026-10-05 (decision F, SE-C99) as the phones always drew them: a numbered list is
// drawn numbered, and a run that mixes bullets and numbers splits into a list of each. The web
// console has drawn them the same way since.

package ca.whnet.agentrouter.data

object Markdown {

    sealed interface Block {
        /** One line of prose, the whitespace that starts it kept (SE-C103). */
        data class Paragraph(val text: String) : Block

        /**
         * The level the web draws: `#` is the third, `##` the fourth, `###` the fifth and
         * anything deeper the sixth, so a heading sits under the screen's own (SE-C98).
         */
        data class Heading(val level: Int, val text: String) : Block
        data class Bullets(val items: List<String>) : Block

        /** Drawn 1., 2., …, as the web draws it too (decision F, SE-C99). */
        data class Numbered(val items: List<String>) : Block
        data class Quote(val text: String) : Block

        /** Verbatim (SE-C93). */
        data class Code(val text: String) : Block

        /** The head row first, then the body (SE-C101). */
        data class Table(val rows: List<List<String>>) : Block
    }

    /** The blocks the web's `md()` draws. */
    fun blocks(src: String): List<Block> {
        val out = mutableListOf<Block>()
        var at = 0
        // A fence runs from three backticks to the next three, wherever on a line they fall. An
        // unterminated one is not code: its backticks and everything after them stay prose
        // (SE-C94).
        while (true) {
            val open = src.indexOf("```", at)
            if (open < 0) break
            val close = src.indexOf("```", open + 3)
            if (close < 0) break
            prose(src.substring(at, open), out)
            out.add(Block.Code(fenced(src.substring(open + 3, close))))
            at = close + 3
        }
        prose(src.substring(at), out)
        return out
    }

    /**
     * A fence's code: its first line dropped when it is a language tag — one word, or nothing —
     * and a newline before the closing fence dropped (SE-C93). Opened and closed on one line, the
     * whole of it is code (SE-C104).
     */
    private fun fenced(inner: String): String {
        val nl = inner.indexOf('\n')
        val code = if (nl >= 0 && isTag(inner.substring(0, nl))) inner.substring(nl + 1) else inner
        return code.removeSuffix("\n")
    }

    /** `^[ \t]*[^\s`]*[ \t]*$` */
    private fun isTag(line: String): Boolean {
        var i = 0
        while (i < line.length && (line[i] == ' ' || line[i] == '\t')) i++
        while (i < line.length && !isSpace(line[i]) && line[i] != '`') i++
        while (i < line.length && (line[i] == ' ' || line[i] == '\t')) i++
        return i == line.length
    }

    /**
     * A stretch of prose, line by line: a list, a table, a heading, a quote, or else a paragraph
     * of its own; consecutive lines are not joined, and blank lines add nothing (SE-C103).
     */
    private fun prose(src: String, out: MutableList<Block>) {
        if (src.isEmpty()) return
        val lines = src.split('\n').map(::trimEnd)
        val list = mutableListOf<Pair<Boolean, String>>()
        var head: List<String>? = null
        val rows = mutableListOf<List<String>>()

        fun flushList() {
            // Bullets and numbers are never one list: a run that mixes them is a list of each, on
            // the web as here (decision F, SE-C99).
            var i = 0
            while (i < list.size) {
                var j = i
                while (j < list.size && list[j].first == list[i].first) j++
                val items = list.subList(i, j).map { it.second }
                out.add(if (list[i].first) Block.Numbered(items) else Block.Bullets(items))
                i = j
            }
            list.clear()
        }

        fun flushTable() {
            head?.let { out.add(Block.Table(listOf(it) + rows)) }
            head = null
            rows.clear()
        }

        var i = 0
        while (i < lines.size) {
            val line = lines[i]
            val row = isRow(line)
            // A table is a row of pipes immediately followed by a separator row (SE-C101).
            if (row && head == null && i + 1 < lines.size && isRule(lines[i + 1])) {
                flushList()
                head = cells(line)
                i += 2
                continue
            }
            i++
            if (head != null) {
                if (row && !isRule(line)) {
                    rows.add(cells(line))
                    continue
                }
                flushTable()
            }
            val dash = bullet(line)
            if (dash != null) {
                list.add(false to dash)
                continue
            }
            val number = numbered(line)
            if (number != null) {
                list.add(true to number)
                continue
            }
            flushList()
            val h = heading(line)
            val q = quote(line)
            when {
                h != null -> out.add(h)
                q != null -> out.add(Block.Quote(q))
                line.any { !isSpace(it) } -> out.add(Block.Paragraph(line))
            }
        }
        flushList()
        flushTable()
    }

    /**
     * `^\s*[-*]\s+(.*)$`: indented or not, so nothing nests (SE-C99); a `* ` marker is never
     * emphasis, because a line's block is decided before its inline formatting (SE-C97).
     */
    private fun bullet(l: String): String? {
        var i = 0
        while (i < l.length && isSpace(l[i])) i++
        if (i >= l.length || (l[i] != '-' && l[i] != '*')) return null
        return rest(l, i + 1)
    }

    /** `^\s*\d+[.)]\s+(.*)$` */
    private fun numbered(l: String): String? {
        var i = 0
        while (i < l.length && isSpace(l[i])) i++
        val digits = i
        while (i < l.length && l[i] in '0'..'9') i++
        if (i == digits || i >= l.length || (l[i] != '.' && l[i] != ')')) return null
        return rest(l, i + 1)
    }

    /**
     * `^(#{1,6})\s+(.*)$`, at the level the web draws it; seven `#`, or a `#` with no space after
     * it, is prose (SE-C98).
     */
    private fun heading(l: String): Block.Heading? {
        var h = 0
        while (h < l.length && l[h] == '#') h++
        if (h !in 1..6) return null
        val text = rest(l, h) ?: return null
        return Block.Heading(minOf(h + 2, 6), text)
    }

    /** `^>\s?(.*)$`: each quoted line a quote of its own (SE-C100). */
    private fun quote(l: String): String? {
        if (!l.startsWith(">")) return null
        val from = if (l.length > 1 && isSpace(l[1])) 2 else 1
        val text = l.substring(from)
        return if (text.any(::endsLine)) null else text
    }

    /** `\s+(.*)$` from [at]: at least one space, then the rest of the line, all of which `.` reads. */
    private fun rest(l: String, at: Int): String? {
        if (at >= l.length || !isSpace(l[at])) return null
        var i = at
        while (i < l.length && isSpace(l[i])) i++
        val text = l.substring(i)
        return if (text.any(::endsLine)) null else text
    }

    /** `^\s*\|.*\|\s*$`, of a line with nothing left to trim at its end. */
    private fun isRow(l: String): Boolean {
        var i = 0
        while (i < l.length && isSpace(l[i])) i++
        if (i >= l.length || l[i] != '|' || l.length - 1 <= i || l.last() != '|') return false
        return l.substring(i + 1, l.length - 1).none(::endsLine)
    }

    /** `^\s*\|[\s:|-]+\|\s*$`, likewise: without it, a line of pipes is prose (SE-C101). */
    private fun isRule(l: String): Boolean {
        var i = 0
        while (i < l.length && isSpace(l[i])) i++
        if (i >= l.length || l[i] != '|' || l.length - 1 <= i + 1 || l.last() != '|') return false
        return l.substring(i + 1, l.length - 1).all { isSpace(it) || it == ':' || it == '|' || it == '-' }
    }

    /** A row's cells, trimmed, their inline formatting read later. */
    private fun cells(l: String): List<String> {
        var s = l
        var i = 0
        while (i < s.length && isSpace(s[i])) i++
        if (i < s.length && s[i] == '|') s = s.substring(i + 1)
        val last = s.lastIndexOf('|')
        if (last >= 0 && s.substring(last + 1).all(::isSpace)) s = s.substring(0, last)
        return s.split('|').map(::trim)
    }

    /** JavaScript's `\s`, which is also what its `trim()` removes. */
    internal fun isSpace(c: Char): Boolean = when (c.code) {
        in 0x09..0x0D, 0x20, 0xA0, 0x1680, in 0x2000..0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000, 0xFEFF -> true
        else -> false
    }

    /** What JavaScript's `.` never reads: the line terminators. */
    private fun endsLine(c: Char): Boolean = when (c.code) {
        0x0A, 0x0D, 0x2028, 0x2029 -> true
        else -> false
    }

    private fun trimEnd(s: String): String = s.trimEnd(::isSpace)

    private fun trim(s: String): String = s.trim(::isSpace)
}
