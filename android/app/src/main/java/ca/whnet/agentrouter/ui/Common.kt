// The handful of shapes every screen is built from.

package ca.whnet.agentrouter.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.data.DotLook
import ca.whnet.agentrouter.data.StateColor

/**
 * A titled group with an explanatory line under it.
 *
 * The iPhone app leans on grouped-list sections that carry both a header and a footer, in
 * about twenty places, and those footers are real explanatory copy rather than decoration.
 * Material has no single component for that, so this is it: header above, content on a
 * surface card, footer below in the muted style.
 */
@Composable
fun SettingsSection(
    header: String? = null,
    footer: String? = null,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    Column(modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
        if (header != null) {
            Text(
                header.uppercase(),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(start = 4.dp, bottom = 6.dp),
            )
        }
        Card(
            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Column(Modifier.fillMaxWidth()) { content() }
        }
        if (footer != null) {
            Text(
                footer,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(start = 4.dp, end = 4.dp, top = 6.dp),
            )
        }
    }
}

/**
 * The coloured dot that says what a session or a router is doing. An `active` one wears a
 * faint halo: the colour says what it is doing, the ring says it is doing it *now*.
 */
@Composable
fun StateDot(state: String?, size: Int = 10, modifier: Modifier = Modifier, active: Boolean = false) =
    StateDot(StateColor.of(state), size, modifier, active)

/**
 * The same dot from a colour already decided. A `hollow` one is a session whose process has
 * gone: the outline says there was a session here, the empty middle that nothing is running
 * it any more — which a gray fill cannot say, since gray is also "idle, and still there".
 */
@Composable
fun StateDot(
    color: StateColor,
    size: Int = 10,
    modifier: Modifier = Modifier,
    active: Boolean = false,
    hollow: Boolean = false,
) {
    val tint = dotColor(color)
    Box(
        modifier.size((size + if (active) 6 else 0).dp),
        contentAlignment = Alignment.Center,
    ) {
        if (active) Box(Modifier.size((size + 6).dp).background(tint.copy(alpha = 0.3f), CircleShape))
        if (hollow) Box(Modifier.size(size.dp).border(1.5.dp, tint, CircleShape))
        else Box(Modifier.size(size.dp).background(tint, CircleShape))
    }
}

/** A dot whose look was decided elsewhere — see [DotLook] for the rules, which are tested there. */
@Composable
fun StateDot(look: DotLook, size: Int = 10, modifier: Modifier = Modifier) =
    StateDot(look.color, size, modifier, active = look.active, hollow = look.hollow)

/** A session's dot, from the router's own reading of it when it sends one. */
@Composable
fun SessionDot(session: LiveSession, size: Int = 10, modifier: Modifier = Modifier, active: Boolean = session.active) =
    StateDot(DotLook.of(session, active), size, modifier)

@Composable
fun dotColor(state: String?): Color = dotColor(StateColor.of(state))

@Composable
fun dotColor(color: StateColor): Color = when (color) {
    StateColor.GREEN -> Color(0xFF34C759)
    StateColor.ORANGE -> Color(0xFFFF9F0A)
    StateColor.YELLOW -> Color(0xFFFFD60A)
    StateColor.RED -> MaterialTheme.colorScheme.error
    StateColor.PURPLE -> Color(0xFFBF5AF2)
    StateColor.GRAY -> MaterialTheme.colorScheme.outline
}

/** What a screen shows when it has nothing to show, and why. */
@Composable
fun EmptyState(
    title: String,
    detail: String? = null,
    modifier: Modifier = Modifier,
    action: (@Composable () -> Unit)? = null,
) {
    Box(modifier.fillMaxWidth().padding(32.dp), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(title, style = MaterialTheme.typography.titleMedium, textAlign = TextAlign.Center)
            if (detail != null) {
                Text(
                    detail,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
            }
            if (action != null) action()
        }
    }
}
