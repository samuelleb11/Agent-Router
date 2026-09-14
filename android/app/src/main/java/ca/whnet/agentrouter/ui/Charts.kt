// The small pieces the dashboard is drawn from. Hand-drawn on a Canvas rather than pulled
// from a charting library: these are four simple shapes, and a library would bring its own
// idea of colour and spacing into a screen that has one already.

package ca.whnet.agentrouter.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.unit.dp

/** One number with its name under it — the row of tiles across the top of the dashboard. */
@Composable
fun StatTile(label: String, value: String, detail: String? = null, modifier: Modifier = Modifier) {
    Column(modifier.padding(vertical = 8.dp, horizontal = 4.dp)) {
        Text(value, style = MaterialTheme.typography.titleLarge)
        Text(
            label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (detail != null) {
            Text(
                detail,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.outline,
            )
        }
    }
}

/** A labelled proportion, for "what filled the context window" and its kin. */
@Composable
fun ShareBar(label: String, fraction: Double, value: String, color: Color? = null) {
    val tint = color ?: MaterialTheme.colorScheme.primary
    Column(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(label, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
            Text(
                value,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Box(
            Modifier.fillMaxWidth().height(6.dp).clip(RoundedCornerShape(3.dp))
                .background(MaterialTheme.colorScheme.surfaceVariant),
        ) {
            Box(
                Modifier.fillMaxWidth(fraction.coerceIn(0.0, 1.0).toFloat())
                    .height(6.dp).clip(RoundedCornerShape(3.dp)).background(tint),
            )
        }
    }
}

/** Activity over time, as bars. Values are already bucketed by the caller. */
@Composable
fun ActivityChart(values: List<Double>, modifier: Modifier = Modifier, height: Int = 90) {
    val tint = MaterialTheme.colorScheme.primary
    val empty = MaterialTheme.colorScheme.surfaceVariant
    Canvas(modifier.fillMaxWidth().height(height.dp)) {
        if (values.isEmpty()) return@Canvas
        val max = values.max().takeIf { it > 0 } ?: 1.0
        val slot = size.width / values.size
        val barWidth = (slot * 0.7f).coerceAtLeast(1f)
        values.forEachIndexed { i, v ->
            val h = (size.height * (v / max)).toFloat().coerceAtLeast(if (v > 0) 2f else 1f)
            drawRect(
                color = if (v > 0) tint else empty,
                topLeft = Offset(i * slot + (slot - barWidth) / 2, size.height - h),
                size = androidx.compose.ui.geometry.Size(barWidth, h),
            )
        }
    }
}

/**
 * A window burning down towards its reset.
 *
 * The line is what has actually been read; the dashed continuation is the projection, and
 * the band around it is the spread across the account's days. Where the band is wide, that
 * width *is* the message — it is why the wording above hedges.
 */
@Composable
fun BurnDownChart(
    series: List<Pair<Double, Double>>,
    projectedEnd: Double?,
    projectedLow: Double?,
    projectedHigh: Double?,
    tone: ca.whnet.agentrouter.data.Projection.Tone,
    modifier: Modifier = Modifier,
    height: Int = 110,
) {
    val line = when (tone) {
        ca.whnet.agentrouter.data.Projection.Tone.BAD -> MaterialTheme.colorScheme.error
        ca.whnet.agentrouter.data.Projection.Tone.WARN -> Color(0xFFFF9F0A)
        else -> MaterialTheme.colorScheme.primary
    }
    val grid = MaterialTheme.colorScheme.surfaceVariant
    Canvas(modifier.fillMaxWidth().height(height.dp)) {
        val h = size.height
        val w = size.width
        // The ceiling: 100% of the window.
        drawLine(grid, Offset(0f, 2f), Offset(w, 2f), strokeWidth = 1f)

        if (series.size >= 2) {
            val t0 = series.first().first
            val t1 = series.last().first
            // Readings occupy the left two-thirds; the projection runs to the reset.
            val span = (t1 - t0).takeIf { it > 0 } ?: 1.0
            val readWidth = w * 0.62f
            val path = Path()
            series.forEachIndexed { i, (t, u) ->
                val x = (((t - t0) / span) * readWidth).toFloat()
                val y = (h - (h * u.coerceIn(0.0, 1.0))).toFloat()
                if (i == 0) path.moveTo(x, y) else path.lineTo(x, y)
            }
            drawPath(path, line, style = androidx.compose.ui.graphics.drawscope.Stroke(width = 3f))

            val lastY = (h - (h * series.last().second.coerceIn(0.0, 1.0))).toFloat()
            // The band, where the router gave one.
            if (projectedLow != null && projectedHigh != null && projectedHigh > projectedLow) {
                val yLow = (h - (h * projectedHigh.coerceIn(0.0, 1.0))).toFloat()
                val yHigh = (h - (h * projectedLow.coerceIn(0.0, 1.0))).toFloat()
                val band = Path().apply {
                    moveTo(readWidth, lastY)
                    lineTo(w, yLow)
                    lineTo(w, yHigh)
                    close()
                }
                drawPath(band, line.copy(alpha = 0.18f))
            }
            if (projectedEnd != null) {
                val yEnd = (h - (h * projectedEnd.coerceIn(0.0, 1.0))).toFloat()
                var x = readWidth
                // Dashed, because it has not happened yet.
                while (x < w) {
                    val next = (x + 8f).coerceAtMost(w)
                    val f0 = (x - readWidth) / (w - readWidth)
                    val f1 = (next - readWidth) / (w - readWidth)
                    drawLine(
                        line.copy(alpha = 0.7f),
                        Offset(x, lastY + (yEnd - lastY) * f0),
                        Offset(next, lastY + (yEnd - lastY) * f1),
                        strokeWidth = 2f,
                    )
                    x += 14f
                }
            }
        }
    }
}

/** A thin line of recent activity, for a row that has no room for a chart. */
@Composable
fun Sparkline(values: List<Double>, modifier: Modifier = Modifier) {
    val tint = MaterialTheme.colorScheme.primary
    Canvas(modifier.width(64.dp).height(18.dp)) {
        if (values.size < 2) return@Canvas
        val max = values.max().takeIf { it > 0 } ?: 1.0
        val step = size.width / (values.size - 1)
        val path = Path()
        values.forEachIndexed { i, v ->
            val x = i * step
            val y = (size.height - size.height * (v / max)).toFloat()
            if (i == 0) path.moveTo(x, y) else path.lineTo(x, y)
        }
        drawPath(path, tint, style = androidx.compose.ui.graphics.drawscope.Stroke(width = 2f))
    }
}


/**
 * Two rate-limit windows over time, as they were actually read — not projected.
 *
 * Each series climbs to its reset and drops back; the sawtooth is the point, so the lines
 * are drawn with gaps where a window rolled over rather than joined straight down through
 * the middle, which would read as a burn that never happened.
 */
@Composable
fun QuotaHistoryChart(
    fiveHour: List<Pair<Double, Double>>,
    sevenDay: List<Pair<Double, Double>>,
    modifier: Modifier = Modifier,
    height: Int = 100,
) {
    val five = MaterialTheme.colorScheme.primary
    val seven = Color(0xFFFF9F0A)
    val grid = MaterialTheme.colorScheme.surfaceVariant
    Canvas(modifier.fillMaxWidth().height(height.dp)) {
        val all = fiveHour + sevenDay
        if (all.size < 2) return@Canvas
        val t0 = all.minOf { it.first }
        val t1 = all.maxOf { it.first }
        val span = (t1 - t0).takeIf { it > 0 } ?: 1.0
        drawLine(grid, Offset(0f, 2f), Offset(size.width, 2f), strokeWidth = 1f)

        fun series(points: List<Pair<Double, Double>>, color: Color) {
            if (points.size < 2) return
            var path = Path()
            var started = false
            var previous = 0.0
            points.forEach { (t, u) ->
                val x = (((t - t0) / span) * size.width).toFloat()
                val y = (size.height - size.height * u.coerceIn(0.0, 1.0)).toFloat()
                // A big drop is a reset, not a burn: break the line there.
                if (started && u < previous - 0.15) {
                    drawPath(path, color, style = androidx.compose.ui.graphics.drawscope.Stroke(width = 2f))
                    path = Path()
                    started = false
                }
                if (!started) { path.moveTo(x, y); started = true } else path.lineTo(x, y)
                previous = u
            }
            drawPath(path, color, style = androidx.compose.ui.graphics.drawscope.Stroke(width = 2f))
        }
        series(fiveHour, five)
        series(sevenDay, seven)
    }
}
