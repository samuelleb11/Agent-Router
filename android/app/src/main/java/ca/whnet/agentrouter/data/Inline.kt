// Inline markdown — the styling inside one line — as a list of runs.
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
     * `**bold**`, `*italic*` or `_italic_`, `` `code` `` and `[text](url)`, plus bare
     * `http(s)://` URLs. Anything unclosed is left as it was typed — a stray asterisk in
     * a shell command is far more common than an unfinished emphasis.
     */
    fun parse(src: String): List<Run> {
        val out = mutableListOf<Run>()
        val plain = StringBuilder()
        var bold = false
        var italic = false
        var i = 0

        fun flush() {
            if (plain.isNotEmpty()) {
                out.add(Run(plain.toString(), bold = bold, italic = italic))
                plain.setLength(0)
            }
        }

        while (i < src.length) {
            val c = src[i]
            when {
                // Code spans win over everything inside them.
                c == '`' -> {
                    val end = src.indexOf('`', i + 1)
                    if (end > i + 1) {
                        flush()
                        out.add(Run(src.substring(i + 1, end), bold = bold, italic = italic, code = true))
                        i = end + 1
                        continue
                    }
                }
                src.startsWith("**", i) -> {
                    val end = src.indexOf("**", i + 2)
                    if (bold || (end > i + 2)) {
                        flush(); bold = !bold; i += 2; continue
                    }
                    // Unpaired: both asterisks are literal, and neither is a single `*`.
                    plain.append("**"); i += 2; continue
                }
                (c == '*' || c == '_') && isEmphasisEdge(src, i, c) -> {
                    flush(); italic = !italic; i += 1; continue
                }
                c == '[' -> {
                    val close = src.indexOf("](", i)
                    val end = if (close > 0) src.indexOf(')', close + 2) else -1
                    if (close > i + 1 && end > close + 2) {
                        flush()
                        out.add(Run(src.substring(i + 1, close), bold = bold, italic = italic,
                            url = src.substring(close + 2, end)))
                        i = end + 1
                        continue
                    }
                }
                src.startsWith("http://", i) || src.startsWith("https://", i) -> {
                    var end = i
                    while (end < src.length && !src[end].isWhitespace() && src[end] !in "<>\"'") end++
                    // Trailing punctuation belongs to the sentence, not the address.
                    while (end > i && src[end - 1] in ".,;:!?)") end--
                    if (end > i + 8) {
                        flush()
                        val url = src.substring(i, end)
                        out.add(Run(url, bold = bold, italic = italic, url = url))
                        i = end
                        continue
                    }
                }
            }
            plain.append(c)
            i++
        }
        flush()
        return out
    }

    /**
     * Whether a `*` or `_` here opens or closes emphasis, as opposed to being part of a
     * word like `snake_case` or a glob like `*.rs`: it needs a closing partner, and it
     * must sit at a word edge.
     */
    private fun isEmphasisEdge(s: String, i: Int, c: Char): Boolean {
        val prev = s.getOrNull(i - 1)
        val next = s.getOrNull(i + 1)
        if (c == '_' && prev?.isLetterOrDigit() == true && next?.isLetterOrDigit() == true) return false
        if (next == c || prev == c) return false
        val opening = (prev == null || !prev.isLetterOrDigit()) && next != null && !next.isWhitespace()
        val closing = prev != null && !prev.isWhitespace() && (next == null || !next.isLetterOrDigit())
        if (!opening && !closing) return false
        if (opening) {
            // An opener needs a closer somewhere ahead on the same line.
            var j = i + 1
            while (j < s.length) {
                if (s[j] == c && !s[j - 1].isWhitespace() && s[j - 1] != c && s.getOrNull(j + 1) != c) return true
                if (s[j] == '\n') return false
                j++
            }
            return false
        }
        return true
    }
}
