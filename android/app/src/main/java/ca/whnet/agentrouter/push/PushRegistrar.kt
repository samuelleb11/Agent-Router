// This phone's side of notifications: asking Android, holding Google's token, and telling
// every router about it. The router does the rest.

package ca.whnet.agentrouter.push

import android.content.Context
import android.os.Build
import androidx.core.app.NotificationManagerCompat
import ca.whnet.agentrouter.BuildConfig
import ca.whnet.agentrouter.api.OkReply
import ca.whnet.agentrouter.api.PushRegisterBody
import ca.whnet.agentrouter.api.post
import ca.whnet.agentrouter.data.Router
import ca.whnet.agentrouter.data.RouterStore
import ca.whnet.agentrouter.data.RouterState

object PushRegistrar {

    private const val PREFS = "push"
    private const val TOKEN = "fcm-token"

    /** Per router, the last registration result — what the settings screen shows. */
    val results = mutableMapOf<String, String>()

    fun token(context: Context): String? =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(TOKEN, null)

    fun remember(context: Context, token: String) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(TOKEN, token).apply()
    }

    /**
     * Whether this build can be told anything at all. Without `google-services.json` the
     * app builds and runs but has no Firebase — the same shape as an iOS build without the
     * push entitlement, which the iPhone app also reports rather than pretends about.
     */
    val configured: Boolean get() = BuildConfig.HAS_FIREBASE

    fun allowed(context: Context): Boolean =
        NotificationManagerCompat.from(context).areNotificationsEnabled()

    /** What to say about this phone under Settings. */
    fun phoneStatus(context: Context): String = when {
        !configured -> "this build has no Firebase project — see docs/google-push-requirements.md"
        !allowed(context) -> "off in Android settings"
        token(context) == null -> "allowed, waiting for Google's token"
        else -> "on"
    }

    /** Android 13 added a permission for this; below it, notifications are on by default. */
    val needsRuntimePermission: Boolean get() = Build.VERSION.SDK_INT >= 33

    /** Tell one router about this phone. Returns what went wrong, or null. */
    suspend fun register(store: RouterStore, router: Router, context: Context): String? {
        val token = token(context) ?: return "no device token yet".also { results[router.id] = it }
        val client = store.client(router) ?: return "bad address".also { results[router.id] = it }
        val body = PushRegisterBody(
            token = token,
            name = "${Build.MANUFACTURER} ${Build.MODEL}",
            platform = "fcm",
            environment = if (BuildConfig.DEBUG) "development" else "production",
            routerRef = router.id,
        )
        return try {
            val reply: OkReply = client.post("/api/push/register", body)
            val problem = reply.problem
            results[router.id] = problem ?: "registered"
            problem
        } catch (e: Throwable) {
            val why = RouterState.describe(e)
            results[router.id] = why
            why
        }
    }

    suspend fun registerEverywhere(store: RouterStore, context: Context) {
        store.routers.forEach { register(store, it, context) }
    }
}
