// The one object the screens share: every router, and the live state hanging off each.

package ca.whnet.agentrouter.data

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import ca.whnet.agentrouter.feed.LiveFeed
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/** What a tapped notification asks for: the router it came from and the session. */
data class PendingOpen(val routerId: String, val sessionId: String)

class AppState(app: Application) : AndroidViewModel(app) {

    val store = RouterStore(app)

    /** Bumped whenever the router list changes, so Compose re-reads it. */
    var revision: Int by mutableStateOf(0)
        private set

    /**
     * A tap waiting to be honoured. Set from the launch intent, which can arrive before
     * any screen exists, and cleared once the navigation has happened.
     */
    var pendingOpen: PendingOpen? by mutableStateOf(null)

    /** The request the Requests list is drilling into. A row is too big for a route arg. */
    var openRequest: ca.whnet.agentrouter.api.RequestRow? by mutableStateOf(null)

    // Held outside Compose state on purpose: these are caches created on demand from
    // inside a composition, and making them observable would mean a read that also
    // writes — the same trap the iOS app documents around its own per-router caches.
    private val states = mutableMapOf<String, RouterState>()

    /**
     * Open feeds, and the coroutines streaming them.
     *
     * They live here rather than in the screen because rotating the device destroys and
     * rebuilds every composable: a feed owned by the screen would drop its connection,
     * clear its rows and start the backfill again, which is exactly the "it reset" you see.
     * Owned by the ViewModel, the stream simply carries on through the rotation.
     */
    private val feeds = mutableMapOf<String, LiveFeed>()
    private val feedJobs = mutableMapOf<String, Job>()

    /** How many session feeds to keep streaming at once. */
    private val keepFeeds = 3

    fun feed(router: Router, session: ca.whnet.agentrouter.api.LiveSession): LiveFeed {
        feeds[session.sessionId]?.let { return it }
        val created = LiveFeed(session, store.client(router), hosted = { states[router.id]?.isHosted == true })
        feeds[session.sessionId] = created
        feedJobs[session.sessionId] = viewModelScope.launch {
            launch { created.loadFacts() }
            created.run()
        }
        // Oldest first, so switching between a few sessions does not accumulate streams.
        while (feeds.size > keepFeeds) {
            val oldest = feeds.keys.first()
            feedJobs.remove(oldest)?.cancel()
            feeds.remove(oldest)
        }
        return created
    }

    override fun onCleared() {
        feedJobs.values.forEach { it.cancel() }
        feedJobs.clear()
        feeds.clear()
        super.onCleared()
    }

    val routers: List<Router> get() { revision; return store.routers }

    fun state(router: Router): RouterState =
        states.getOrPut(router.id) { RouterState(router, store) }.also { it.apply(router) }

    fun add(router: Router, password: String?) {
        store.add(router, password)
        revision++
    }

    fun update(router: Router, password: String?) {
        store.update(router, password)
        states[router.id]?.apply(router)
        revision++
    }

    fun remove(id: String) {
        store.remove(id)
        states.remove(id)
        revision++
    }
}
