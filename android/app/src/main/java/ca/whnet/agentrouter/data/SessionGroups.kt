// The session list, arranged the way the web view arranges it — so a phone and a laptop
// looking at the same router agree about what is where.

package ca.whnet.agentrouter.data

import ca.whnet.agentrouter.api.Activity
import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.LiveStatus

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

        /**
         * The heading's dot, ranked alike on the web view, the iPhone and here: the most
         * pressing of the project's sessions — waiting on you, then failed, then gone quiet,
         * then at work, then idle — so a project never reads green while something in it is
         * blocked on you or broken. The empty ring only once every session in it has ended:
         * one idle session still running is a filled dot. The halo while anything is live.
         *
         * An older router says nothing about what its sessions are doing, so there the dot
         * is what it always was: plain busy while something is live, and none otherwise.
         */
        val dot: DotLook?
            get() {
                if (sessions.none { it.activity != null }) {
                    return if (live) DotLook(StateColor.GREEN, active = true) else null
                }
                val running = sessions.filterNot { it.isClosed }
                if (running.isEmpty()) return DotLook(StateColor.GRAY, hollow = true)
                val said = running.map { it.color }.toSet()
                return DotLook(RANKED.firstOrNull { it in said } ?: StateColor.GRAY, active = live)
            }

        private companion object {
            /**
             * A heading's colours, most pressing first: waiting on you, failed, stalled, at
             * work. Anything else still running — idle, or an activity this app does not
             * know — is the neutral gray after them.
             */
            val RANKED = listOf(StateColor.ORANGE, StateColor.RED, StateColor.YELLOW, StateColor.GREEN)
        }
    }

    /**
     * One machine's share of the list, arranged as a whole list is. [label] is the machine's
     * name, and null for rows that do not name one — which is every row when the list is not
     * split by machine at all.
     */
    data class Machine(val label: String?, val groups: SessionGroups)

    companion object {
        const val WINDOW_MS = 12L * 3600 * 1000

        /**
         * On screen rather than folded under "older": moved within the window, or live now.
         *
         * A session blocked on a question writes nothing while it waits — for hours, at
         * times — so going by its last activity alone folded it away exactly when it
         * mattered most. `active` is the router's word for "busy, or waiting on a person".
         * An older router means "wrote something in the last two minutes" by it, which is
         * inside the window anyway, so against one of those this is the recency test it
         * always was.
         */
        fun isRecent(r: LiveSession, now: Long): Boolean =
            r.active || now - r.lastActivityMs <= WINDOW_MS

        fun build(rows: List<LiveSession>, now: Long = System.currentTimeMillis()): SessionGroups {
            // `archived` arrives from the router already meaning "put away, and quiet ever
            // since" — it clears itself the moment the session does something new. So the
            // shelf is exactly what the router says it is. Re-testing it here against a
            // recency window, as the iOS version once did, un-hid everything: the session
            // you just finished with had by definition moved a moment before you shelved it.
            val shelved = rows.filter { it.archived }
            val rest = rows.filter { !it.archived }
            return SessionGroups(
                recent = group(rest.filter { isRecent(it, now) }),
                older = group(rest.filterNot { isRecent(it, now) }),
                archived = shelved.sortedByDescending { it.lastActivityMs },
            )
        }

        /**
         * The list a machine at a time, for a hosted router that merges several machines'
         * sessions: machines by name, alphabetically, each arranged exactly as [build]
         * arranges a whole list — so one project on two machines stays two projects, and
         * each machine keeps its own older sessions and its own shelf. Rows that name no
         * machine come last, together.
         *
         * With one machine or none — and a router on a Mac names none — this is a single
         * entry, unlabelled, holding [build] of every row: the list as it always was.
         */
        fun byMachine(rows: List<LiveSession>, now: Long = System.currentTimeMillis()): List<Machine> {
            if (rows.mapNotNull { it.machineLabel }.distinct().size <= 1) {
                return listOf(Machine(null, build(rows, now)))
            }
            // Sorted as a list rather than through a sorted map: two names that differ only
            // in case compare equal, and a map would keep one machine and drop the other.
            return rows.groupBy { it.machineLabel }
                .toList()
                .sortedWith(compareBy(nullsLast(String.CASE_INSENSITIVE_ORDER)) { it.first })
                .map { (label, mine) -> Machine(label, build(mine, now)) }
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
        /** The old vocabulary: `state`, which every router sends. */
        fun of(state: String?): StateColor = when (state) {
            "working", "tool" -> GREEN
            "ask" -> ORANGE
            "stalled" -> YELLOW
            "failed" -> RED
            "thinking" -> PURPLE
            else -> GRAY
        }

        /**
         * The router's own choice first, so the web view and both phones colour a session
         * alike; then what [activity] implies; then the old [state], for a router that sends
         * neither. An activity this app does not know is neutral gray — never a guess from
         * the legacy state, which for a new activity says only what an old client should do.
         */
        fun of(tone: String?, activity: String?, state: String?): StateColor =
            ofTone(tone) ?: activity?.let(::ofActivity) ?: of(state)

        /** Null for no tone, or for one newer than this app — then [activity] decides. */
        fun ofTone(tone: String?): StateColor? = when (tone) {
            "busy" -> GREEN
            "attention" -> ORANGE
            "warn" -> YELLOW
            "error" -> RED
            "idle", "closed" -> GRAY
            else -> null
        }

        fun ofActivity(activity: String): StateColor = when (activity) {
            Activity.GENERATING, Activity.TOOL, Activity.BACKGROUND -> GREEN
            Activity.WAITING -> ORANGE
            Activity.STALLED -> YELLOW
            Activity.FAILED -> RED
            else -> GRAY
        }
    }
}

/**
 * A dot, decided: its colour, whether it wears the halo that says "now", and whether it is
 * the empty ring of a session whose process has gone. Deciding it apart from drawing it is
 * what lets these rules be tested, and lets the header and the open tab share one.
 */
data class DotLook(val color: StateColor, val active: Boolean = false, val hollow: Boolean = false) {
    companion object {
        /** A list row's dot: the router's reading of the session, as of the last listing. */
        fun of(s: LiveSession, active: Boolean = s.active): DotLook =
            DotLook(s.color, active = active && !s.isClosed, hollow = s.isClosed)

        /**
         * The open session's dot, from its own stream while the stream vouches for it
         * ([fresh]). Once it cannot — dropped, or silent past the heartbeat — neutral gray
         * rather than the colour it last had: known once, not now. Before the first status
         * arrives, the list row's, which is all there is.
         */
        fun stream(status: LiveStatus?, fresh: Boolean, row: LiveSession): DotLook = when {
            status != null && fresh -> DotLook(status.color, active = status.isActive, hollow = status.isClosed)
            status != null -> DotLook(StateColor.GRAY)
            else -> of(row, active = false)
        }

        /**
         * A tab over the open feed. The open session's tab is its header's dot, from the
         * stream, which is current to the second; every other tab is its list row, which is
         * all this screen knows of it. Taken from the list, the open tab could sit gray for
         * five seconds beside a header already saying the turn had started. Tabs wear no halo.
         */
        fun tab(tab: LiveSession, openId: String, status: LiveStatus?, fresh: Boolean): DotLook =
            if (tab.sessionId == openId) stream(status, fresh, tab).copy(active = false)
            else of(tab, active = false)
    }
}

/**
 * How many of a router's sessions are at work and how many are waiting on you — counted as
 * the web view's header and the iPhone's router row count them, so a session blocked on you
 * is told apart from one at work. One that has ended is neither, and is never counted.
 */
data class SessionCounts(val working: Int, val waiting: Int) {

    /** A few words, and the colour to say them in. */
    data class Line(val text: String, val color: StateColor)

    /** "4 working · 3 waiting", leaving out a half that is zero; null when both are. */
    val summary: String?
        get() = listOfNotNull(
            working.takeIf { it > 0 }?.let { "$it working" },
            waiting.takeIf { it > 0 }?.let { "$it waiting" },
        ).joinToString(" · ").ifEmpty { null }

    /** Waiting on you is the one worth colouring: it is the one that needs a thumb. */
    val color: StateColor get() = if (waiting > 0) StateColor.ORANGE else StateColor.GREEN

    companion object {
        /**
         * Null from an older router: no row names its activity, and `active` — busy and
         * waiting in one — is all it says.
         */
        fun of(rows: List<LiveSession>): SessionCounts? =
            if (rows.none { it.activity != null }) null
            else SessionCounts(working = rows.count { it.isWorking }, waiting = rows.count { it.needsYou })

        /**
         * The router row's line about its sessions: "4 working · 3 waiting", orange when
         * anything waits. From an older router, "3 active sessions" as it always was. Null
         * when nothing is either.
         */
        fun routerLine(rows: List<LiveSession>): Line? {
            val counts = of(rows) ?: return rows.count { it.active }.takeIf { it > 0 }
                ?.let { n -> Line("$n active session${if (n == 1) "" else "s"}", StateColor.GREEN) }
            return counts.summary?.let { Line(it, counts.color) }
        }

        /**
         * The router page's line about its sessions: "4 working · 3 waiting · 9 this week",
         * or from an older router "7 active · 9 this week".
         */
        fun weekLine(rows: List<LiveSession>): String {
            val counts = of(rows)
            val live = if (counts != null) counts.summary
            else rows.count { it.active }.takeIf { it > 0 }?.let { "$it active" }
            return listOfNotNull(live, "${rows.size} this week").joinToString(" · ")
        }
    }
}

/** A row's colour: the router's reading when it sends one, the old state otherwise. */
val LiveSession.color: StateColor get() = StateColor.of(tone, activity, state)

/** The stream's colour, by the same rule as the row's, so the header and the list agree. */
val LiveStatus.color: StateColor get() = StateColor.of(tone, activity, state)
