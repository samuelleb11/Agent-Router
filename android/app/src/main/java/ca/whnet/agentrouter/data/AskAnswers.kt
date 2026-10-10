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
