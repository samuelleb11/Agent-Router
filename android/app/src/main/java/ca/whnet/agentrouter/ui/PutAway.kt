// Putting a session away, or bringing it back, and what the screen it was asked from does
// once the router has answered: decided here, apart from the composables, so it can be tested.

package ca.whnet.agentrouter.ui

import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.data.RouterState
import ca.whnet.agentrouter.feed.LiveFeed

/**
 * What came of a put-away is what [RouterState.archive] returns: null when the router did
 * it, and otherwise what went wrong, in the router's words whatever status came with them.
 * Neither screen may read a refusal as done. The open session's screen once went back to the
 * list whatever the router said, so a refused put-away closed the session as if it had
 * worked, and the list dropped the answer, so a refusal there showed nothing at all.
 */
object PutAway {

    /**
     * From the open session's own menu. Put away, the screen goes [back] to the list: null in
     * a pane, beside the list, where there is nothing to go back to. Refused, the session
     * stays open and the feed's notice, the screen's snackbar, says why.
     */
    suspend fun fromSession(state: RouterState, session: LiveSession, feed: LiveFeed, back: (() -> Unit)?) {
        when (val refused = state.archive(session, archived = true)) {
            null -> back?.invoke()
            else -> feed.notice = LiveFeed.Notice(refused, ok = false)
        }
    }

    /**
     * From the Sessions list: put away, or brought back when [archived] is false. Refused,
     * the list is told why, to [say] it. Done, there is nothing to say: the listing the router
     * has just been asked for again moves the row.
     */
    suspend fun fromList(state: RouterState, session: LiveSession, archived: Boolean, say: suspend (String) -> Unit) {
        state.archive(session, archived)?.let { say(it) }
    }
}
