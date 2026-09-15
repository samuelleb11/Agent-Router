package ca.whnet.agentrouter

import android.content.Intent
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.toRoute
import ca.whnet.agentrouter.data.AppState
import ca.whnet.agentrouter.data.PendingOpen
import ca.whnet.agentrouter.push.Notifications
import ca.whnet.agentrouter.push.PushRegistrar
import ca.whnet.agentrouter.ui.*

class MainActivity : ComponentActivity() {

    private val app: AppState by viewModels()

    private val askNotifications =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        consume(intent)
        if (PushRegistrar.needsRuntimePermission && Build.VERSION.SDK_INT >= 33) {
            askNotifications.launch(android.Manifest.permission.POST_NOTIFICATIONS)
        }
        setContent { AgentRouterTheme { Root(app) } }
    }

    /** `singleTask`, so a tap while the app is already open arrives here rather than onCreate. */
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        consume(intent)
    }

    private fun consume(intent: Intent?) {
        val session = intent?.getStringExtra(Notifications.EXTRA_SESSION) ?: return
        val router = intent.getStringExtra(Notifications.EXTRA_ROUTER)
        // The router this came from, or — when the phone no longer knows that id and there
        // is exactly one router — that one, which is nearly always what was meant.
        val id = app.store.byId(router ?: "")?.id
            ?: app.store.routers.singleOrNull()?.id
            ?: return
        app.pendingOpen = PendingOpen(id, session)
    }
}

@androidx.compose.runtime.Composable
private fun Root(app: AppState) {
    val nav = rememberNavController()

    // A tap that arrived before any screen existed — a cold launch from the notification
    // itself does that — is honoured as soon as there is a graph to navigate.
    val pending = app.pendingOpen
    LaunchedEffect(pending) {
        val open = pending ?: return@LaunchedEffect
        app.pendingOpen = null
        val router = app.store.byId(open.routerId) ?: return@LaunchedEffect
        nav.navigate(RouterDetailRoute(router.id))
        nav.navigate(SessionsRoute(router.id))
        nav.navigate(SessionRoute(router.id, open.sessionId))
    }

    NavHost(nav, startDestination = RoutersRoute) {
        composable<RoutersRoute> {
            RoutersListScreen(
                app = app,
                onOpen = { nav.navigate(RouterDetailRoute(it.id)) },
                onAdd = { nav.navigate(RouterFormRoute(null)) },
                // Wide windows keep the list and show the router beside it; the callbacks
                // still navigate, because those destinations are full screens either way.
                detail = { router ->
                    RouterDetailScreen(
                        app = app,
                        router = router,
                        onBack = null,
                        onSessions = { nav.navigate(SessionsRoute(router.id)) },
                        onUsage = { nav.navigate(UsageRoute(router.id)) },
                        onAccounts = { nav.navigate(AccountsRoute(router.id)) },
                        onSettings = { nav.navigate(SettingsRoute(router.id)) },
                    )
                },
            )
        }
        composable<RouterFormRoute> { entry ->
            val id = entry.toRoute<RouterFormRoute>().id
            RouterFormScreen(
                app = app,
                existing = id?.let { app.store.byId(it) },
                onDone = { nav.popBackStack() },
                onRemoved = { nav.popBackStack(RoutersRoute, inclusive = false) },
            )
        }
        composable<RouterDetailRoute> { entry ->
            val router = app.store.byId(entry.toRoute<RouterDetailRoute>().id)
            if (router == null) { nav.popBackStack(); return@composable }
            RouterDetailScreen(
                app = app,
                router = router,
                onBack = { nav.popBackStack() },
                onSessions = { nav.navigate(SessionsRoute(router.id)) },
                onUsage = { nav.navigate(UsageRoute(router.id)) },
                onAccounts = { nav.navigate(AccountsRoute(router.id)) },
                onSettings = { nav.navigate(SettingsRoute(router.id)) },
            )
        }
        composable<UsageRoute> { entry ->
            val router = app.store.byId(entry.toRoute<UsageRoute>().id)
            if (router == null) { nav.popBackStack(); return@composable }
            UsageScreen(
                app = app,
                router = router,
                onBack = { nav.popBackStack() },
                onList = { kind, since -> nav.navigate(UsageListRoute(router.id, kind, since)) },
            )
        }
        composable<UsageListRoute> { entry ->
            val route = entry.toRoute<UsageListRoute>()
            val router = app.store.byId(route.id)
            if (router == null) { nav.popBackStack(); return@composable }
            val back = { nav.popBackStack(); Unit }
            when (route.kind) {
                "projects" -> ProjectsScreen(app, router, route.since, back)
                "sessions" -> ProxySessionsScreen(app, router, route.since, back)
                "tools" -> ToolsScreen(app, router, route.since, back)
                else -> RequestsScreen(app, router, route.since, back) {
                    app.openRequest = it
                    nav.navigate(RequestDetailRoute)
                }
            }
        }
        composable<RequestDetailRoute> {
            val request = app.openRequest
            if (request == null) { nav.popBackStack(); return@composable }
            RequestDetailScreen(request) { nav.popBackStack() }
        }
        composable<AccountsRoute> { entry ->
            val router = app.store.byId(entry.toRoute<AccountsRoute>().id)
            if (router == null) { nav.popBackStack(); return@composable }
            AccountsScreen(app, router) { nav.popBackStack() }
        }
        composable<SessionsRoute> { entry ->
            val router = app.store.byId(entry.toRoute<SessionsRoute>().id)
            if (router == null) { nav.popBackStack(); return@composable }
            SessionsScreen(
                app = app,
                router = router,
                onBack = { nav.popBackStack() },
                onOpen = { nav.navigate(SessionRoute(router.id, it.sessionId)) },
                onSettings = { nav.navigate(SettingsRoute(router.id)) },
                detail = { session -> SessionScreen(app, router, session, onBack = null) },
            )
        }
        composable<SessionRoute> { entry ->
            val route = entry.toRoute<SessionRoute>()
            val router = app.store.byId(route.id)
            if (router == null) { nav.popBackStack(); return@composable }
            val state = app.state(router)
            // Opened from a notification, the session list may not have arrived yet; a
            // placeholder carrying the id is enough for the feed to connect.
            val session = state.sessions.firstOrNull { it.sessionId == route.session }
                ?: ca.whnet.agentrouter.api.LiveSession(sessionId = route.session, wrapped = true)
            SessionScreen(app, router, session) { nav.popBackStack() }
        }
        composable<SettingsRoute> { entry ->
            val router = app.store.byId(entry.toRoute<SettingsRoute>().id)
            if (router == null) { nav.popBackStack(); return@composable }
            SettingsScreen(
                app = app,
                router = router,
                onBack = { nav.popBackStack() },
                onEdit = { nav.navigate(RouterFormRoute(router.id)) },
            )
        }
    }
}
