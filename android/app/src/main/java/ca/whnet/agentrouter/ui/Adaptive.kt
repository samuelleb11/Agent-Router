// How wide the window is, and what that should change.
//
// The tablet is 800dp across in portrait and 1280dp in landscape, where the phone is about
// 400dp. Stretching one column across either of those is the wrong answer twice over: a
// line of body text becomes unreadably long, and a list that could be showing you a session
// beside its feed shows you one thing with a field of empty space next to it.

package ca.whnet.agentrouter.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** Material's own breakpoints, named so the call sites read as intent rather than numbers. */
enum class WindowWidth { COMPACT, MEDIUM, EXPANDED }

@Composable
@ReadOnlyComposable
fun windowWidthDp(): Int = LocalConfiguration.current.screenWidthDp

@Composable
@ReadOnlyComposable
fun windowWidth(): WindowWidth = when {
    windowWidthDp() < 600 -> WindowWidth.COMPACT
    windowWidthDp() < 840 -> WindowWidth.MEDIUM
    else -> WindowWidth.EXPANDED
}

/**
 * Whether a list and the thing it selects should sit side by side.
 *
 * The threshold is below Material's "expanded" on purpose: the tablet's 800dp portrait
 * splits into a 320dp list and a 480dp detail, both of which are comfortably usable, and
 * making portrait behave like a phone would waste the better half of the screen.
 */
@Composable
@ReadOnlyComposable
fun twoPane(): Boolean = windowWidthDp() >= 720

/** How wide the list pane should be. Fixed, so the detail takes the growth. */
@Composable
@ReadOnlyComposable
fun listPaneWidth(): Dp = if (windowWidthDp() >= 1100) 380.dp else 320.dp

/**
 * A column that stops growing once it is wide enough to read.
 *
 * Long measures are hard to read — the eye loses the line coming back. Forms and settings
 * are capped and centred rather than stretched; a chart can have the rest.
 */
@Composable
fun ReadableColumn(
    modifier: Modifier = Modifier,
    max: Dp = 720.dp,
    content: @Composable () -> Unit,
) {
    Box(modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
        Box(Modifier.widthIn(max = max)) { content() }
    }
}

/**
 * Cap a column's measure and centre it, by growing the side padding rather than wrapping
 * anything — so it drops into an existing modifier chain without restructuring a screen.
 *
 * Long lines are hard to read: the eye loses its place coming back to the left. A form or a
 * page of settings gets capped; a chart is welcome to the rest of the width.
 */
fun Modifier.readableWidth(max: Dp = 720.dp): Modifier = this
    .fillMaxWidth()
    // Measured against whatever contains it, not against the window: inside a detail pane
    // the window is far wider than the space actually available, and padding computed from
    // the window pushes the content off to one side.
    .wrapContentWidth(Alignment.CenterHorizontally)
    .widthIn(max = max)

/** A list beside what it selects. On a narrow window only one of them is shown. */
@Composable
fun ListDetail(
    showDetail: Boolean,
    list: @Composable () -> Unit,
    detail: @Composable () -> Unit,
) {
    if (!twoPane()) {
        if (showDetail) detail() else list()
        return
    }
    Row(Modifier.fillMaxSize(), horizontalArrangement = Arrangement.spacedBy(0.dp)) {
        Box(Modifier.width(listPaneWidth()).fillMaxHeight()) { list() }
        VerticalDivider()
        Box(Modifier.weight(1f).fillMaxHeight()) { detail() }
    }
}


/**
 * A little room at the end of a list, and nothing more.
 *
 * Deliberately not an inset: `Scaffold` already pads its content for the system bars, and
 * adding a second copy on top of `innerPadding` padded every page twice over.
 */
fun screenPadding(): PaddingValues = PaddingValues(bottom = 12.dp)

/**
 * Room for the system's bottom furniture, for a bar Scaffold places *below* its content —
 * the one place the padding is not already applied.
 */
@Composable
fun Modifier.bottomBarInsets(): Modifier = this.windowInsetsPadding(WindowInsets.navigationBars)
