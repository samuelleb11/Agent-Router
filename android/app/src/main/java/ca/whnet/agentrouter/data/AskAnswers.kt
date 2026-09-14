// Turning what was picked into what the tool expects — and into what a person would have
// typed, for when the prompt cannot be answered directly.

package ca.whnet.agentrouter.data

import ca.whnet.agentrouter.api.AskQuestion

object AskAnswers {

    data class Selection(val chosen: Set<String> = emptySet(), val other: String = "")

    data class Composed(val answers: Map<String, String>, val parts: List<String>)

    /**
     * `question text -> answer`, plus one line per answered question.
     *
     * The map is keyed by the question's own text because that is what the router's
     * `/api/live/answer` expects. Option order is the tool's own, not the order they were
     * tapped in; "Other" is whatever was typed, and typing it is choosing it.
     *
     * [Composed.parts] is the fallback: if the session is no longer waiting on the prompt
     * the router returns 503, and the picked answers are sent as an ordinary message
     * instead of being lost.
     */
    fun compose(
        questions: List<AskQuestion>,
        selections: Map<String, Selection>,
    ): Composed {
        val answers = LinkedHashMap<String, String>()
        val parts = mutableListOf<String>()
        for (q in questions) {
            val sel = selections[q.question] ?: continue
            val picked = (q.options ?: emptyList())
                .map { it.label }
                .filter { it in sel.chosen }
                .toMutableList()
            val other = sel.other.trim()
            if (other.isNotEmpty()) picked.add(other)
            if (picked.isEmpty()) continue
            val a = picked.joinToString(", ")
            answers[q.question] = a
            parts.add("${q.title}: $a")
        }
        return Composed(answers, parts)
    }
}

/**
 * Assistant prose, split into the blocks the web view's `md()` draws. Ported line for line
 * from the iOS app so the three clients render the same feed the same way.
 */
object Markdown {

    sealed interface Block {
        data class Paragraph(val text: String) : Block
        data class Heading(val level: Int, val text: String) : Block
        data class Bullets(val items: List<String>) : Block
        data class Numbered(val items: List<String>) : Block
        data class Quote(val text: String) : Block
        data class Code(val text: String) : Block
        data class Table(val rows: List<List<String>>) : Block
    }

    private fun cells(line: String): List<String> {
        var t = line.trim()
        if (t.startsWith("|")) t = t.substring(1)
        if (t.endsWith("|")) t = t.dropLast(1)
        return t.split("|").map { it.trim() }
    }

    private fun isRow(l: String): Boolean {
        val t = l.trim()
        return t.startsWith("|") && t.endsWith("|") && t.length > 1
    }

    private fun isRule(l: String): Boolean = isRow(l) && l.all { it in "|-: " }

    fun blocks(src: String): List<Block> {
        val out = mutableListOf<Block>()
        var para = mutableListOf<String>()
        var bullets = mutableListOf<String>()
        var numbered = mutableListOf<String>()
        var quote = mutableListOf<String>()
        var table = mutableListOf<List<String>>()
        var code: MutableList<String>? = null

        fun flush() {
            if (para.isNotEmpty()) { out.add(Block.Paragraph(para.joinToString("\n"))); para = mutableListOf() }
            if (bullets.isNotEmpty()) { out.add(Block.Bullets(bullets)); bullets = mutableListOf() }
            if (numbered.isNotEmpty()) { out.add(Block.Numbered(numbered)); numbered = mutableListOf() }
            if (quote.isNotEmpty()) { out.add(Block.Quote(quote.joinToString("\n"))); quote = mutableListOf() }
            if (table.isNotEmpty()) { out.add(Block.Table(table)); table = mutableListOf() }
        }

        val lines = src.split("\n")
        var i = 0
        while (i < lines.size) {
            val raw = lines[i]
            val line = raw.trim()
            val held = code
            if (held != null) {
                if (line.startsWith("```")) { out.add(Block.Code(held.joinToString("\n"))); code = null }
                else held.add(raw)
                i++
                continue
            }
            if (line.startsWith("```")) { flush(); code = mutableListOf(); i++; continue }
            if (line.isEmpty()) { flush(); i++; continue }

            if (isRow(line)) {
                if (table.isEmpty()) {
                    // A table is a header row followed by a |---| rule; without the rule a
                    // line of pipes is prose and must stay prose.
                    if (i + 1 < lines.size && isRule(lines[i + 1])) {
                        flush(); table = mutableListOf(cells(line)); i += 2; continue
                    }
                } else if (!isRule(line)) {
                    table.add(cells(line)); i++; continue
                }
            } else if (table.isNotEmpty()) {
                flush()
            }

            val hash = line.indexOfFirst { it != '#' }
            if (line.startsWith("#") && hash > 0 && hash < line.length && line[hash] == ' ') {
                flush()
                out.add(Block.Heading(minOf(hash, 6), line.substring(hash).trim()))
                i++
                continue
            }
            if (line.startsWith("- ") || line.startsWith("* ")) {
                if (para.isNotEmpty() || numbered.isNotEmpty() || quote.isNotEmpty()) flush()
                bullets.add(line.substring(2))
                i++
                continue
            }
            val dot = line.indexOfFirst { it == '.' || it == ')' }
            if (dot > 0 && line.take(dot).all { it.isDigit() } &&
                dot + 1 < line.length && line[dot + 1] == ' '
            ) {
                if (para.isNotEmpty() || bullets.isNotEmpty() || quote.isNotEmpty()) flush()
                numbered.add(line.substring(dot + 2))
                i++
                continue
            }
            if (line.startsWith(">")) {
                if (para.isNotEmpty() || bullets.isNotEmpty() || numbered.isNotEmpty()) flush()
                quote.add(line.substring(1).trim())
                i++
                continue
            }
            if (bullets.isNotEmpty() || numbered.isNotEmpty() || quote.isNotEmpty()) flush()
            para.add(line)
            i++
        }
        code?.let { out.add(Block.Code(it.joinToString("\n"))) }
        flush()
        return out
    }
}
