// The one object the screens share: every router, and the live state hanging off each.

package ca.whnet.agentrouter.data

import android.app.Application
import androidx.lifecycle.AndroidViewModel
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
