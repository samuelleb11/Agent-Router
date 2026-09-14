// This phone's notification shade, as this app uses it.
//
// A banner goes up when a session blocks on a person; it comes down when the prompt behind
// it is answered — wherever it was answered, which is usually not here.
//
// Two ways down, because neither alone is enough. The router sends a data message naming
// what was answered, which is immediate and silent but which Android will not deliver to a
// force-stopped app (or one a Samsung has put to sleep). So whenever the app comes to the
// front it also asks each router what is still waiting and clears whatever is left over.

package ca.whnet.agentrouter.push

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import ca.whnet.agentrouter.MainActivity
import ca.whnet.agentrouter.R
import ca.whnet.agentrouter.api.PendingReply
import ca.whnet.agentrouter.api.get
import ca.whnet.agentrouter.data.RouterStore

object Notifications {

    const val CHANNEL = "prompts"
    const val EXTRA_SESSION = "session"
    const val EXTRA_ROUTER = "router_ref"

    /** One banner on screen, reduced to what can be reasoned about elsewhere. */
    data class OnScreen(val tag: String, val id: Int, val router: String?, val prompt: String)

    fun ensureChannel(context: Context) {
        val channel = NotificationChannel(
            CHANNEL,
            "Waiting for you",
            NotificationManager.IMPORTANCE_HIGH,
        ).apply {
            description = "A session has stopped and needs an answer."
            enableVibration(true)
        }
        NotificationManagerCompat.from(context).createNotificationChannel(channel)
    }

    /**
     * The tag is the router's own key for the prompt — `req:…` or `ask:…` — so a repeat
     * replaces the banner it raised rather than stacking a second one, and so a take-down
     * can find it again. The id is derived from the tag for the same reason.
     */
    fun idFor(tag: String): Int = tag.hashCode()

    fun post(
        context: Context,
        title: String,
        head: String,
        detail: String,
        session: String?,
        routerRef: String?,
        promptKey: String,
    ) {
        val intent = Intent(context, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
            putExtra(EXTRA_SESSION, session)
            putExtra(EXTRA_ROUTER, routerRef)
            // A distinct action per prompt, so two pending intents are not treated as one.
            action = "open:$promptKey"
        }
        val pending = PendingIntent.getActivity(
            context, idFor(promptKey), intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        // Android gives two lines, where iOS gives title/subtitle/body. The head goes on
        // the collapsed line and the detail expands, so a long command or question is
        // readable without opening the app.
        val text = if (detail.isBlank()) head else "$head — $detail"
        val builder = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_prompt)
            .setContentTitle(title)
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().setBigContentTitle(head).bigText(detail.ifBlank { head }))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(Notification.CATEGORY_MESSAGE)
            .setContentIntent(pending)
            .setAutoCancel(true)
            // Grouped by session, which is what the APNs `thread-id` does for the iPhone.
            .setGroup(session ?: "router")
        NotificationManagerCompat.from(context).notify(promptKey, idFor(promptKey), builder.build())
    }

    /**
     * Banners raised by a prompt, and only those. A test push is left alone — nothing here
     * can say whether it is still wanted.
     */
    fun onScreen(context: Context): List<OnScreen> {
        val manager = context.getSystemService(NotificationManager::class.java) ?: return emptyList()
        return manager.activeNotifications.mapNotNull { sbn ->
            val tag = sbn.tag ?: return@mapNotNull null
            if (!tag.startsWith("req:") && !tag.startsWith("ask:")) return@mapNotNull null
            OnScreen(
                tag = tag,
                id = sbn.id,
                router = sbn.notification.extras.getString(EXTRA_ROUTER),
                prompt = tag,
            )
        }
    }

    /** Take down the banners raised by these prompts. */
    fun takeDown(context: Context, answered: Set<String>) {
        if (answered.isEmpty()) return
        val manager = NotificationManagerCompat.from(context)
        onScreen(context).filter { it.prompt in answered }.forEach { manager.cancel(it.tag, it.id) }
    }

    /**
     * Ask every router that raised a banner whether its prompt is still waiting, and take
     * down the ones that are not. A router that cannot be reached **keeps** its banners: a
     * phone off the LAN knows nothing, and guessing would hide real prompts.
     */
    suspend fun reconcile(context: Context, store: RouterStore) {
        val showing = onScreen(context)
        if (showing.isEmpty()) return
        val manager = NotificationManagerCompat.from(context)
        for (router in store.routers) {
            val mine = showing.filter { it.router == router.id }
            if (mine.isEmpty()) continue
            val client = store.client(router) ?: continue
            val waiting = runCatching { client.get<PendingReply>("/api/push/pending").waiting.toSet() }
                .getOrNull() ?: continue
            mine.filter { it.prompt !in waiting }.forEach { manager.cancel(it.tag, it.id) }
        }
    }
}
