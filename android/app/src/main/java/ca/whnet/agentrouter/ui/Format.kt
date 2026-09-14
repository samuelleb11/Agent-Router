// Numbers and times, spelled the way the dashboard spells them.

package ca.whnet.agentrouter.ui

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

    fun bytes(n: Long): String = when {
        n < 1024 -> "$n B"
        n < 1024 * 1024 -> String.format(Locale.US, "%.0f KB", n / 1024.0)
        else -> String.format(Locale.US, "%.1f MB", n / (1024.0 * 1024))
    }
}
