// What a window's projection says, and how loudly. Mirrors the dashboard's wording so the
// phone and the Mac never disagree about the same numbers — including the three-state
// verdict, where red is reserved for a crossing a quiet week would not undo.

package ca.whnet.agentrouter.data

import ca.whnet.agentrouter.api.Forecast
import ca.whnet.agentrouter.ui.Fmt
import java.util.Locale
import kotlin.math.roundToInt

data class Projection(val tone: Tone, val text: String, val basis: String?) {

    enum class Tone { PLAIN, WARN, BAD }

    companion object {
        fun describe(w: Forecast.Window?, now: Long = System.currentTimeMillis()): Projection? {
            if (w == null) return null
            if (w.isExpired) {
                val when_ = w.resetAt?.let { Fmt.ago(it * 1000, now) } ?: "earlier"
                return Projection(Tone.PLAIN, "Window rolled over $when_ · nothing seen since", null)
            }
            val left = w.activeHoursLeft
            if (w.basis == "none" || left == null) {
                return Projection(Tone.PLAIN, "Not enough burn yet to project", null)
            }
            val basis = basisText(w)
            val exhaustMs = w.exhaustsAt
            val resetMs = w.resetAt?.times(1000)

            // Red is for a crossing that survives even a quiet week. When the band straddles
            // the reset, the honest answer is that it could go either way, and saying that
            // in red spends the reader's alarm on a coin flip.
            if (w.call == "tight" && exhaustMs != null) {
                val early = (resetMs ?: exhaustMs) - exhaustMs
                val quiet = (((w.projectedEndUtilLow ?: w.used ?: 0.0)) * 100).roundToInt()
                return Projection(
                    Tone.WARN,
                    "${Fmt.hours(left)} of work left · too close to call — on this pace it runs " +
                        "out ${Fmt.span(early)} before reset, on a quieter week it ends ≈$quiet%",
                    basis,
                )
            }
            if (w.call == "crosses" && exhaustMs != null) {
                val early = (resetMs ?: exhaustMs) - exhaustMs
                return Projection(
                    Tone.BAD,
                    "${Fmt.hours(left)} of work left · runs out ${Fmt.relative(exhaustMs, now)}, " +
                        "${Fmt.span(early)} before it resets",
                    basis,
                )
            }
            val end = ((w.projectedEndUtil ?: w.used ?: 0.0) * 100).roundToInt()
            return Projection(
                if (end >= 90) Tone.WARN else Tone.PLAIN,
                "${Fmt.hours(left)} of work left · resets first, ending ≈$end%",
                basis,
            )
        }

        private fun basisText(w: Forecast.Window): String? {
            val burn = w.burnPerActiveHour ?: return null
            if (burn <= 0) return null
            val rate = String.format(Locale.US, "%.1f%% per hour of work", burn * 100)
            if (w.basis == "trailing-windows") {
                return "$rate · nothing burnt yet this window, so this is your recent average"
            }
            fun pct(v: Double) = (v * 100).roundToInt()
            // Disclose the spread where there is one: it is why the line above hedges.
            var duty = "${pct(w.dutyFraction ?: 0.0)}% of the clock spent working"
            val d = w.duty
            if (d != null && w.hasBand) {
                duty = "${pct(d.mid)}% of the clock spent working, ${pct(d.low)}–${pct(d.high)}% across your days"
            }
            return "$rate · $duty"
        }
    }
}
