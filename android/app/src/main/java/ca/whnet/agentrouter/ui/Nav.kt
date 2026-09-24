// Where the app can be. Every destination is declared once, here, so a notification tap
// can build a stack three deep without any screen knowing about the one above it.

package ca.whnet.agentrouter.ui

import kotlinx.serialization.Serializable

@Serializable object RoutersRoute
@Serializable data class RouterDetailRoute(val id: String)
@Serializable data class RouterFormRoute(val id: String?)
@Serializable data class SessionsRoute(val id: String)
@Serializable data class SessionRoute(val id: String, val session: String)
@Serializable data class SettingsRoute(val id: String)
@Serializable data class UsageRoute(val id: String)
@Serializable data class AccountsRoute(val id: String)
// Search is per router: the index belongs to one store, and there is nothing to ask for
// a view across several.
@Serializable data class SearchRoute(val id: String)
@Serializable data class UsageListRoute(val id: String, val kind: String, val since: String?)
@Serializable object RequestDetailRoute
