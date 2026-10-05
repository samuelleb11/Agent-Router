// Everything the app knows about one router right now, and how it keeps that current.

package ca.whnet.agentrouter.data

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import ca.whnet.agentrouter.api.Account
import ca.whnet.agentrouter.api.AccountRefBody
import ca.whnet.agentrouter.api.ActivateResponse
import ca.whnet.agentrouter.api.ArchiveBody
import ca.whnet.agentrouter.api.AuthStatus
import ca.whnet.agentrouter.api.Forecast
import ca.whnet.agentrouter.api.Health
import ca.whnet.agentrouter.api.LiveReply
import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.Quota
import ca.whnet.agentrouter.api.SetupStatus
import ca.whnet.agentrouter.api.ShimStatus
import ca.whnet.agentrouter.api.RouterClient
import ca.whnet.agentrouter.api.RouterFailure
import ca.whnet.agentrouter.api.get
import ca.whnet.agentrouter.api.post
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlin.coroutines.coroutineContext

class RouterState(router: Router, private val store: RouterStore) {

    var router: Router by mutableStateOf(router)
        private set
    private var client: RouterClient? = store.client(router)

    var health: Health? by mutableStateOf(null)
    var auth: AuthStatus? by mutableStateOf(null)
    var sessions: List<LiveSession> by mutableStateOf(emptyList())
    var quota: Quota? by mutableStateOf(null)
    var forecast: Forecast? by mutableStateOf(null)
    var accounts: List<Account> by mutableStateOf(emptyList())
    var setup: SetupStatus? by mutableStateOf(null)
    var shim: ShimStatus? by mutableStateOf(null)

    /** The account id whose activation is in flight, so its row can show progress. */
    var switching: String? by mutableStateOf(null)

    /**
     * Why the last listing failed, when it did — the page must say so rather than show an
     * empty week, which reads as "nothing is running" and is a different claim entirely.
     */
    var sessionsFailure: String? by mutableStateOf(null)
    var failure: RouterFailure? by mutableStateOf(null)
    var lastRefresh: Long? by mutableStateOf(null)
    var refreshing: Boolean by mutableStateOf(false)

    /** Whether the session list has been fetched at all — "empty" and "not yet" differ. */
    var sessionsLoaded: Boolean by mutableStateOf(false)

    val isReachable: Boolean get() = health?.ok == true && failure == null

    /**
     * Whether the router said it is hosted, the last time it answered `/api/auth`. False
     * until it has, so whatever is worded by this reads as for a router on a Mac, as it
     * always did, until the router itself says otherwise.
     */
    val isHosted: Boolean get() = auth?.isHosted == true

    val activeAccount: Forecast.ForecastAccount? get() = forecast?.accounts?.firstOrNull { it.isActive }

    /** Accounts to offer in the switcher: everything the router lists, in its order. */
    val switchable: List<Forecast.ForecastAccount> get() = forecast?.accounts ?: emptyList()

    fun credential(id: String): Account? = accounts.firstOrNull { it.id == id }

    /** For the pages that stream: the same address and password this state uses. */
    val liveClient: RouterClient? get() = client

    fun apply(router: Router) {
        this.router = router
        client = store.client(router)
    }

    suspend fun refresh(includeAccounts: Boolean = false) {
        val c = client
        if (c == null) {
            failure = RouterFailure.Unreachable("bad address")
            return
        }
        refreshing = true
        try {
            coroutineScope {
                val h = async { runCatching { c.get<Health>("/_router/health") } }
                val a = async { runCatching { c.get<AuthStatus>("/api/auth") } }
                val s = async { runCatching { c.get<List<LiveSession>>("/api/live/sessions") } }
                val f = async { runCatching { c.get<Forecast>("/api/forecast") } }
                val q = async { runCatching { c.get<Quota>("/api/quota") } }

                var first: Throwable? = null
                h.await().onSuccess { health = it }.onFailure { first = first ?: it }
                a.await().onSuccess { auth = it }.onFailure { first = first ?: it }
                f.await().onSuccess { forecast = it }.onFailure { first = first ?: it }
                q.await().onSuccess { quota = it }.onFailure { first = first ?: it }
                // Its own failure: a slow or refused listing must not blank the page.
                s.await()
                    .onSuccess { sessions = it; sessionsFailure = null }
                    .onFailure { sessionsFailure = describe(it, isHosted) }
                sessionsLoaded = true

                // `probe=1` makes the router check every credential, so the accounts are
                // asked for less often than the rest — on demand, and once a minute. Without
                // it a row carries no verdict: no failed dot, and no detail line.
                if (first == null && (includeAccounts || accounts.isEmpty())) {
                    runCatching { c.get<List<Account>>(c.query("/api/accounts", mapOf("probe" to "1"))) }
                        .onSuccess { accounts = it }.onFailure { first = it }
                    runCatching { c.get<SetupStatus>("/api/setup") }.onSuccess { setup = it }
                    runCatching { c.get<ShimStatus>("/api/setup/shim") }.onSuccess { shim = it }
                }

                val e = first
                if (e == null) {
                    failure = null
                    lastRefresh = System.currentTimeMillis()
                } else {
                    failure = e as? RouterFailure ?: RouterFailure.Unreachable(describe(e))
                }
            }
        } finally {
            refreshing = false
        }
    }

    /** Just the session list, for the sessions page's faster cadence. */
    suspend fun refreshSessions() {
        val c = client ?: return
        runCatching { c.get<List<LiveSession>>("/api/live/sessions") }
            .onSuccess { sessions = it; sessionsFailure = null }
            .onFailure { sessionsFailure = describe(it, isHosted) }
        sessionsLoaded = true
    }

    /**
     * Just `/api/auth`, for a page that words itself by [isHosted] and can be reached
     * before any page that asks for it has — a notification opens straight onto a session.
     * A failure leaves [auth] as it was: the pages that refresh everything report it.
     */
    suspend fun refreshAuth() {
        val c = client ?: return
        runCatching { c.get<AuthStatus>("/api/auth") }
            .onSuccess { auth = it }
            .onFailure { if (it is CancellationException) throw it }
    }

    /** Returns what went wrong, or null. */
    suspend fun archive(session: LiveSession, archived: Boolean): String? {
        val c = client ?: return "bad address"
        return runCatching {
            c.post<ArchiveBody, LiveReply>("/api/live/archive", ArchiveBody(session.sessionId, archived))
            refreshSessions()
            null
        }.getOrElse { describe(it) }
    }

    /** Make this account the one that serves. */
    suspend fun activate(id: String) {
        val c = client ?: return
        if (id == activeAccount?.id) return
        switching = id
        try {
            c.post<AccountRefBody, ActivateResponse>("/api/accounts/activate", AccountRefBody(id))
            refresh(includeAccounts = true)
        } catch (e: Throwable) {
            if (e is CancellationException) throw e
            failure = e as? RouterFailure ?: RouterFailure.Unreachable(describe(e))
        } finally {
            switching = null
        }
    }

    /** Runs until the coroutine that started it is cancelled — the page going away. */
    suspend fun poll(seconds: Long = 10) {
        var n = 0
        while (coroutineContext.isActive) {
            refresh(includeAccounts = n % 6 == 0)
            n++
            delay(seconds * 1000)
        }
    }

    companion object {
        /**
         * A failure as a sentence, with cancellation left alone so it can propagate.
         * [hosted] is whether the router is a hosted one ([RouterState.isHosted]), which
         * changes who sets a missing password: its deployment, not a dashboard on a Mac.
         */
        fun describe(e: Throwable, hosted: Boolean = false): String {
            if (e is CancellationException) throw e
            if (hosted && e is RouterFailure.NoPasswordSet) return RouterFailure.NoPasswordSet.HOSTED
            return e.message ?: e::class.simpleName ?: "something went wrong"
        }
    }
}
