// Inline markdown — the styling inside one line — as a list of runs, read exactly as the web
// console reads it: `inline()` in ui/src/model/markdown.ts, the reference for both phones
// (decision F; SE-C91, SE-C92, SE-C95, SE-C96 in docs/web-console-contracts.md). The fixtures the
// three clients share, fixtures/live/markdown-blocks.json, hold this to it (SharedFixtureTest).
//
// Kept apart from Compose so it can be tested on the JVM: the composable that draws a
// paragraph turns these runs into spans, and nothing here knows what a span is.

package ca.whnet.agentrouter.data

object Inline {

    data class Run(
        val text: String,
        val bold: Boolean = false,
        val italic: Boolean = false,
        val code: Boolean = false,
        /** A link's destination; the text is what is shown for it. */
        val url: String? = null,
    )

    /**
     * The web reads a line in four passes — inline code, bold, italic, links — each over the
     * output of the one before, in which whatever an earlier pass built reads as one character
     * no pattern names. So bold can hold inline code, italic can hold bold and a link's text can
     * hold all three, while nothing is ever read inside inline code (SE-C95).
     *
     * - `` `code` ``, from one backtick to the next on the same line, verbatim.
     * - `**bold**`, with no asterisk inside.
     * - `*italic*`, only when its opening asterisk follows the start of the line, a space or
     *   "(", so `d*no*e`, `*.rs` and `2 ** 8` stay as typed; underscores never mean emphasis.
     * - `[text](https://…)`: only an http or https address is a link, and a bare one is text.
     *
     * Anything unclosed is left as it was typed. Adjacent runs in the same styles are one.
     */
    fun parse(src: String): List<Run> {
        // Inline code first.
        var pieces = mutableListOf<Piece>()
        var i = 0
        while (i < src.length) {
            val end = if (src[i] == '`') stop(src, i + 1, '`') else -1
            if (end >= 0) {
                pieces.add(Piece.Node(listOf(Run(src.substring(i + 1, end), code = true)), src.substring(i, end + 1)))
                i = end + 1
            } else {
                pieces.add(Piece.Char(src[i]))
                i++
            }
        }
        pieces = pass(pieces) { s, at, all ->
            if (s[at] != '*' || at + 1 >= s.length || s[at + 1] != '*') return@pass null
            val end = stop(s, at + 2, '*')
            if (end < 0 || end + 1 >= s.length || s[end + 1] != '*') return@pass null
            Match(end + 2, 0, styled(all.subList(at + 2, end)) { it.copy(bold = true) })
        }
        pieces = pass(pieces) { s, at, all ->
            val lead = when {
                at == 0 && s[0] == '*' -> 0
                (Markdown.isSpace(s[at]) || s[at] == '(') && at + 1 < s.length && s[at + 1] == '*' -> 1
                else -> return@pass null
            }
            val end = stop(s, at + lead + 1, '*')
            if (end < 0) return@pass null
            Match(end + 1, lead, styled(all.subList(at + lead + 1, end)) { it.copy(italic = true) })
        }
        pieces = pass(pieces) { s, at, all ->
            if (s[at] != '[') return@pass null
            val close = stop(s, at + 1, ']')
            if (close < 0 || close + 1 >= s.length || s[close + 1] != '(') return@pass null
            var k = close + 2
            if (!s.startsWith("http", k)) return@pass null
            k += 4
            k = when {
                k + 1 < s.length && s[k] == 's' && s[k + 1] == ':' -> k + 2
                k < s.length && s[k] == ':' -> k + 1
                else -> return@pass null
            }
            var end = k
            while (end < s.length && s[end] != ')' && !Markdown.isSpace(s[end])) end++
            if (end == k || end >= s.length || s[end] != ')') return@pass null
            val url = source(all.subList(close + 2, end))
            Match(end + 1, 0, styled(all.subList(at + 1, close)) { it.copy(url = url) })
        }
        return merged(runs(pieces))
    }

    private sealed interface Piece {
        data class Char(val c: kotlin.Char) : Piece

        /** What an earlier pass built: its runs, and the text it was read from. */
        data class Node(val runs: List<Run>, val source: String) : Piece
    }

    /** Where a match ends, from which of its characters the node starts, and the node's runs. */
    private class Match(val end: Int, val from: Int, val runs: List<Run>)

    /** The character a node stands for in what a later pass reads, one no pattern names. */
    private val OPAQUE = Char(0xE000)

    /**
     * The end of a pattern's middle, `[^x\n]+x` read from [from]: the index of the closing [x],
     * with at least one character before it and no newline; -1 when there is none.
     */
    private fun stop(s: String, from: Int, x: kotlin.Char): Int {
        var j = from
        while (j < s.length && s[j] != x && s[j] != '\n') j++
        return if (j > from && j < s.length && s[j] == x) j else -1
    }

    /** One pass: every match, leftmost first and never overlapping, becomes a node. */
    private fun pass(pieces: List<Piece>, match: (String, Int, List<Piece>) -> Match?): MutableList<Piece> {
        val flat = buildString { for (p in pieces) append(if (p is Piece.Char) p.c else OPAQUE) }
        val out = mutableListOf<Piece>()
        var at = 0
        var i = 0
        while (i < flat.length) {
            val m = match(flat, i, pieces)
            if (m == null) {
                i++
                continue
            }
            out.addAll(pieces.subList(at, i + m.from))
            out.add(Piece.Node(m.runs, source(pieces.subList(i + m.from, m.end))))
            at = m.end
            i = m.end
        }
        out.addAll(pieces.subList(at, pieces.size))
        return out
    }

    private fun runs(pieces: List<Piece>): List<Run> {
        val out = mutableListOf<Run>()
        val text = StringBuilder()
        for (p in pieces) {
            when (p) {
                is Piece.Char -> text.append(p.c)
                is Piece.Node -> {
                    if (text.isNotEmpty()) out.add(Run(text.toString()))
                    text.setLength(0)
                    out.addAll(p.runs)
                }
            }
        }
        if (text.isNotEmpty()) out.add(Run(text.toString()))
        return out
    }

    private fun styled(pieces: List<Piece>, style: (Run) -> Run): List<Run> = runs(pieces).map(style)

    private fun source(pieces: List<Piece>): String = buildString {
        for (p in pieces) {
            when (p) {
                is Piece.Char -> append(p.c)
                is Piece.Node -> append(p.source)
            }
        }
    }

    /** Adjacent runs in the same styles as one, and no empty run. */
    private fun merged(runs: List<Run>): List<Run> {
        val out = mutableListOf<Run>()
        for (r in runs) {
            if (r.text.isEmpty()) continue
            val last = out.lastOrNull()
            if (last != null && last.copy(text = r.text) == r) out[out.size - 1] = last.copy(text = last.text + r.text)
            else out.add(r)
        }
        return out
    }
}
