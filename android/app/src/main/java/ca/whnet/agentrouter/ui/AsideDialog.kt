// The side question's dialog: what it asks, whether it is still waiting, and what came back.
// Kept apart from the composable so it can be tested.

package ca.whnet.agentrouter.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import ca.whnet.agentrouter.feed.LiveFeed

/**
 * Ask a forked copy of the session something without disturbing it, and show what came back
 * in the dialog it was asked from, which is where the person asking is looking.
 *
 * The router runs `claude --resume --fork-session` with no tools, which can take a while (up
 * to three minutes before it gives up), so the dialog says so while it waits rather than
 * looking hung, and cannot be closed under it.
 *
 * When nothing is answered the dialog says why, in the router's words, as an
 * [Outcome.Failure] and never as an answer. Those words once went to the screen's snackbar,
 * behind the dialog's dimmed background, while the dialog itself said only "No answer came
 * back.".
 */
class AsideDialog {

    /** What came back: the fork's answer, or why there is none. */
    sealed interface Outcome {
        val text: String

        data class Answer(override val text: String) : Outcome

        /** Nothing answered, and why, in the router's words: shown as a failure. */
        data class Failure(override val text: String) : Outcome
    }

    var open: Boolean by mutableStateOf(false)
        private set
    var question: String by mutableStateOf("")
    var running: Boolean by mutableStateOf(false)
        private set
    var outcome: Outcome? by mutableStateOf(null)
        private set

    fun show() {
        open = true
    }

    /** Closed, and what came back with it. Never while it waits: nothing else would show the answer. */
    fun close() {
        if (running) return
        open = false
        outcome = null
    }

    /**
     * Ask [question] of a fork of [feed]'s session. A refusal is told to the dialog rather
     * than to the feed's notice, so it is said once, here.
     */
    suspend fun ask(feed: LiveFeed) {
        if (running || question.isBlank()) return
        running = true
        outcome = null
        try {
            var refused: String? = null
            val answer = feed.aside(question) { refused = it }?.answer?.takeIf { it.isNotBlank() }
            outcome = answer?.let { Outcome.Answer(it) } ?: Outcome.Failure(refused ?: "No answer came back.")
        } finally {
            running = false
        }
    }
}
