package ca.whnet.agentrouter.ui

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

// The accent the iPhone app uses, as the seed for everything else.
private val Seed = Color(0xFF2F6FEB)

private val Light = lightColorScheme(primary = Seed)
private val Dark = darkColorScheme(primary = Color(0xFF9EC2FF))

/**
 * Material You where the phone offers it. The Note 9 is on Android 10 and does not, so it
 * gets the seeded scheme — which is also what keeps the two phone apps recognisably one
 * product rather than one of them wearing the system wallpaper's colours.
 */
@Composable
fun AgentRouterTheme(
    dark: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    val scheme = when {
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
            val ctx = LocalContext.current
            if (dark) dynamicDarkColorScheme(ctx) else dynamicLightColorScheme(ctx)
        }
        dark -> Dark
        else -> Light
    }
    MaterialTheme(colorScheme = scheme, content = content)
}
