// The session list, arranged the way the web view arranges it — so a phone and a laptop
// looking at the same router agree about what is where.

package ca.whnet.agentrouter.data

import ca.whnet.agentrouter.api.LiveSession

data class SessionGroups(
    /** Moved within the window, or doing something now. */
    val recent: List<Project>,
    val older: List<Project>,
    /** Put away by the reader, and not showing life since. */
    val archived: List<LiveSession>,
) {
    data class Project(val name: String, val sessions: List<LiveSession>) {
        val live: Boolean get() = sessions.any { it.active }
        val newest: Long get() = sessions.maxOfOrNull { it.lastActivityMs } ?: 0
    }

    companion object {
        const val WINDOW_MS = 12L * 3600 * 1000

        /**
         * Whether a session belongs on screen. Used for ordering the visible list, not for
         * deciding what is on the shelf.
         */
        fun isCurrent(r: LiveSession, now: Long): Boolean =
            r.active || r.state == "working" || r.state == "ask" ||
                now - r.lastActivityMs <= WINDOW_MS

        fun build(rows: List<LiveSession>, now: Long = System.currentTimeMillis()): SessionGroups {
            // `archived` arrives from the router already meaning "put away, and quiet ever
            // since" — it clears itself the moment the session does something new. So the
            // shelf is exactly what the router says it is. Re-testing it here against a
            // recency window, as the iOS version once did, un-hid everything: the session
            // you just finished with had by definition moved a moment before you shelved it.
            val shelved = rows.filter { it.archived }
            val rest = rows.filter { !it.archived }
            return SessionGroups(
                recent = group(rest.filter { now - it.lastActivityMs <= WINDOW_MS }),
                older = group(rest.filter { now - it.lastActivityMs > WINDOW_MS }),
                archived = shelved.sortedByDescending { it.lastActivityMs },
            )
        }

        /**
         * Projects alphabetically, so a project keeps its place instead of jumping to the
         * top whenever it moves. Sessions within one newest-first — there, recency is what
         * you are choosing between.
         */
        private fun group(rows: List<LiveSession>): List<Project> =
            rows.groupBy { it.projectName }
                .toSortedMap(String.CASE_INSENSITIVE_ORDER)
                .map { (name, sessions) ->
                    Project(name, sessions.sortedByDescending { it.lastActivityMs })
                }
    }
}

/** What a session's state means, as a colour role the theme resolves. */
enum class StateColor { GREEN, ORANGE, YELLOW, RED, PURPLE, GRAY;
    companion object {
        fun of(state: String?): StateColor = when (state) {
            "working", "tool" -> GREEN
            "ask" -> ORANGE
            "stalled" -> YELLOW
            "failed" -> RED
            "thinking" -> PURPLE
            else -> GRAY
        }
    }
}
