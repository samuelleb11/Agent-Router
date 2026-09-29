// Numbers and times, spelled the way the dashboard spells them — and the session status
// lines, which are mostly a router's sentence and a time.

package ca.whnet.agentrouter.ui

import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.LiveStatus
import java.util.Locale
import kotlin.math.abs

object Fmt {

    /** "just now", "4m", "2h", "3d" — short enough to sit at the end of a row. */
    fun ago(atMs: Long, now: Long = System.currentTimeMillis()): String {
        if (atMs <= 0) return "—"
        val s = (now - atMs) / 1000
        return when {
            s < 45 -> "just now"
            s < 3600 -> "${s / 60}m"
            s < 86_400 -> "${s / 3600}h"
            else -> "${s / 86_400}d"
        }
    }

    /** A duration, for "thought for 2.4s". */
    fun duration(ms: Long): String = when {
        ms < 1000 -> "${ms}ms"
        ms < 60_000 -> String.format(Locale.US, "%.1fs", ms / 1000.0)
        else -> "${ms / 60_000}m ${(ms % 60_000) / 1000}s"
    }

    /**
     * How long something has been going on: "45s", "3m 07s", "3h 12m", "1d 0h" — the
     * iPhone's spelling, so the two headers read alike. Whole seconds, because a counter
     * that ticks in tenths is noise.
     */
    fun elapsed(ms: Long): String {
        val s = if (ms <= 0) 0 else (ms + 500) / 1000
        if (s < 60) return "${s}s"
        val m = s / 60
        if (m < 60) return String.format(Locale.US, "%dm %02ds", m, s % 60)
        val h = m / 60
        if (h < 24) return String.format(Locale.US, "%dh %02dm", h, m % 60)
        return "${h / 24}d ${h % 24}h"
    }

    /** Big counts, shortened: 1234 -> 1.2k, 4_500_000 -> 4.5M. */
    fun compact(n: Long): String {
        val a = abs(n)
        return when {
            a < 1_000 -> n.toString()
            a < 1_000_000 -> String.format(Locale.US, "%.1fk", n / 1_000.0)
            a < 1_000_000_000 -> String.format(Locale.US, "%.1fM", n / 1_000_000.0)
            else -> String.format(Locale.US, "%.1fB", n / 1_000_000_000.0)
        }
    }

    fun usd(v: Double?): String = when {
        v == null -> "—"
        v >= 100 -> String.format(Locale.US, "$%.0f", v)
        v >= 1 -> String.format(Locale.US, "$%.2f", v)
        else -> String.format(Locale.US, "$%.3f", v)
    }

    /** Hours of work, the dashboard's phrasing. */
    fun hours(h: Double?): String = when {
        h == null -> "—"
        h < 1 -> "${(h * 60).toInt()} min"
        h < 10 -> String.format(Locale.US, "%.1f h", h)
        else -> "${h.toInt()} h"
    }

    /** A span, for "45 min before it resets". */
    fun span(ms: Long): String {
        val m = ms / 60_000
        return when {
            m < 60 -> "$m min"
            m < 60 * 24 -> "${m / 60} h ${m % 60} min"
            else -> "${m / (60 * 24)} d"
        }
    }

    /** "in 40 minutes", "in 3 hours" — when something is expected to happen. */
    fun relative(atMs: Long, now: Long = System.currentTimeMillis()): String {
        val d = atMs - now
        if (d <= 0) return "now"
        return "in ${span(d)}"
    }

    /** A clock time, in the phone's own format: "3:45 PM" or "15:45". */
    fun clock(atMs: Long): String =
        java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT).format(java.util.Date(atMs))

    fun percent(fraction: Double): String = "${(fraction * 100).toInt()}%"

    fun bytes(n: Long): String = when {
        n < 1024 -> "$n B"
        n < 1024 * 1024 -> String.format(Locale.US, "%.0f KB", n / 1024.0)
        else -> String.format(Locale.US, "%.1f MB", n / (1024.0 * 1024))
    }
}

/**
 * The sentences the session screens show, built from what the router said. The words are
 * the router's own — it picks them once for every client — so all this adds is how long,
 * and how many are queued. Kept apart from the composables so they can be tested.
 */
object StatusText {

    /** Under four seconds a counter is noise: every step of a turn would flash one. */
    private const val SHOW_AFTER_MS = 4_000L

    /**
     * The open session's second header line. A status is shown only while the stream still
     * vouches for it ([fresh]: connected, heard from lately, and sent on this connection
     * rather than the last). Otherwise the last word it had is the past — "Running Bash ·
     * 12s" frozen on a phone that left the LAN — and saying "reconnecting…" is the truth.
     */
    fun header(st: LiveStatus?, connected: Boolean, fresh: Boolean, before: String): String = when {
        st == null && !connected -> "connecting…"
        st == null -> before
        !fresh -> "reconnecting…"
        else -> line(st)
    }

    /**
     * The open session's header, for a status the stream says is current: the router's
     * sentence, how long it has been so, and how many messages wait behind the turn. A
     * closed session says how long ago it ended instead — time "in" a state means nothing
     * once nothing is running. Durations are measured on the router's clock where it sends
     * one, so a phone whose clock is off by a minute does not add that minute.
     */
    fun line(st: LiveStatus, now: Long = System.currentTimeMillis()): String {
        val said = st.detail.ifEmpty { st.state }
        val ended = st.endedMs
        var line = when {
            st.isClosed && ended != null -> "$said · ${Fmt.elapsed((st.asOfMs ?: now) - ended)} ago"
            st.isClosed -> said
            st.quietMs > SHOW_AFTER_MS -> "$said · ${Fmt.elapsed(st.quietMs)}"
            else -> said
        }
        st.queued?.takeIf { it > 0 }?.let { line += " · $it queued" }
        return line
    }

    /**
     * A list row's line: the router's words for what the session is doing, and for how
     * long — for every session, idle included ("Waiting for you · 1h 00m": how long one has
     * sat waiting for you is worth a glance too). An ended one says how long ago it ended
     * instead. The web view and the iPhone follow the same rule, so the three read alike.
     *
     * Measured on the router's clock ([LiveSession.asOfMs]) wherever it sends one, never
     * against the phone's, so a phone whose clock is off does not shift every row. Null for
     * an older router, which sends no words: its rows look exactly as they always did.
     */
    fun row(s: LiveSession, now: Long = System.currentTimeMillis()): String? {
        val said = s.detail?.takeIf { it.isNotBlank() } ?: return null
        if (s.activity == null) return null
        val asOf = s.asOfMs ?: now
        if (s.isClosed) return s.endedMs?.let { "$said · ${Fmt.elapsed(asOf - it)} ago" } ?: said
        val since = s.sinceMs ?: return said
        val took = asOf - since
        return if (took > SHOW_AFTER_MS) "$said · ${Fmt.elapsed(took)}" else said
    }
}
