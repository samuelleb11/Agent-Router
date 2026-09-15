package ca.whnet.agentrouter.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.api.Forecast
import ca.whnet.agentrouter.api.Quota
import ca.whnet.agentrouter.data.Projection
import kotlin.math.roundToInt

/** A window's fill at a glance: the same 70/90 thresholds as the dashboard. */
@Composable
fun quotaTint(fraction: Double, live: Boolean): Color = when {
    !live -> MaterialTheme.colorScheme.outline
    fraction >= 0.9 -> MaterialTheme.colorScheme.error
    fraction >= 0.7 -> Tint.orange
    else -> MaterialTheme.colorScheme.primary
}

@Composable
fun toneColor(tone: Projection.Tone): Color = when (tone) {
    Projection.Tone.BAD -> MaterialTheme.colorScheme.error
    Projection.Tone.WARN -> Tint.orange
    Projection.Tone.PLAIN -> MaterialTheme.colorScheme.primary
}

/**
 * One rate-limit window: how full it is, when it resets, and what the projection makes of
 * that. The wording comes from [Projection] so this and the dashboard never disagree.
 */
@Composable
fun QuotaRow(
    title: String,
    window: Quota.Window?,
    projection: Projection?,
    modifier: Modifier = Modifier,
) {
    val live = window?.utilization != null
    val fraction = window?.fraction ?: 0.0
    val fill = quotaTint(fraction, live)
    Column(modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(title, Modifier.weight(1f), style = MaterialTheme.typography.titleSmall)
            Text(
                if (live) "${(window!!.utilization ?: 0.0).roundToInt()}%" else "—",
                style = MaterialTheme.typography.titleSmall,
                color = fill,
            )
        }
        LinearProgressIndicator(
            progress = { fraction.toFloat() },
            color = fill,
            trackColor = MaterialTheme.colorScheme.surfaceVariant,
            modifier = Modifier.fillMaxWidth(),
        )
        window?.resetMs?.let {
            Text(
                "resets ${Fmt.relative(it)} · ${Fmt.clock(it)}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (window == null) {
            Text("No rate-limit reading yet", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        if (projection != null) {
            Text(
                projection.text,
                style = MaterialTheme.typography.bodySmall,
                color = if (projection.tone == Projection.Tone.PLAIN)
                    MaterialTheme.colorScheme.onSurfaceVariant else toneColor(projection.tone),
            )
            // Estimates say they are estimates.
            projection.basis?.let {
                Text(it, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.outline)
            }
        }
    }
}

/** The burn-down for one window, with its projection band. */
@Composable
fun BurnDownSection(title: String, window: Forecast.Window?) {
    if (window == null || window.series.size < 2) return
    val projection = Projection.describe(window)
    SettingsSection(header = title) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            BurnDownChart(
                series = window.series.map { it.t to it.u },
                projectedEnd = window.projectedEndUtil,
                projectedLow = window.projectedEndUtilLow,
                projectedHigh = window.projectedEndUtilHigh,
                tone = projection?.tone ?: Projection.Tone.PLAIN,
            )
            if (projection != null) {
                Text(
                    projection.text,
                    style = MaterialTheme.typography.bodySmall,
                    color = if (projection.tone == Projection.Tone.PLAIN)
                        MaterialTheme.colorScheme.onSurfaceVariant else toneColor(projection.tone),
                )
            }
        }
    }
}
