// Where the router's messages arrive.
//
// Everything is data-only in both directions: the router never asks Google to draw a
// notification, because a notification drawn by the system while the app is in the
// background never reaches the code that knows how to tag it, group it by session, or take
// it down again. So this builds every banner itself.
//
// This file is compiled only when `google-services.json` is present — see the source set in
// app/build.gradle.kts.

package ca.whnet.agentrouter.push

import ca.whnet.agentrouter.data.RouterStore
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import org.json.JSONArray

class RouterMessagingService : FirebaseMessagingService() {

    override fun onNewToken(token: String) {
        PushRegistrar.remember(this, token)
        // Every router this phone knows is told, not just the one that happened to be open.
        CoroutineScope(Dispatchers.IO).launch {
            PushRegistrar.registerEverywhere(RouterStore(applicationContext), applicationContext)
        }
    }

    override fun onMessageReceived(message: RemoteMessage) {
        val data = message.data

        // A take-down: the prompts named here have been answered, wherever that happened.
        data["dismiss"]?.let { raw ->
            val answered = buildSet {
                runCatching {
                    val arr = JSONArray(raw)
                    for (i in 0 until arr.length()) add(arr.getString(i))
                }
            }
            Notifications.takeDown(this, answered)
            return
        }

        val promptKey = data["request_id"] ?: return
        Notifications.ensureChannel(this)
        Notifications.post(
            context = this,
            title = data["title"].orEmpty().ifBlank { "Agent Router" },
            head = data["head"].orEmpty(),
            detail = data["detail"].orEmpty(),
            session = data["session"],
            routerRef = data["router_ref"],
            promptKey = promptKey,
        )
    }
}
