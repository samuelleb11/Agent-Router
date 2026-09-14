// A router as this phone knows it: a name, an address, and a password kept elsewhere.

package ca.whnet.agentrouter.data

import android.content.Context
import ca.whnet.agentrouter.api.RouterClient
import ca.whnet.agentrouter.api.routerJson
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import java.io.File
import java.util.UUID

@Serializable
data class Router(
    val id: String = UUID.randomUUID().toString(),
    val name: String,
    val host: String,
    val port: Int = 8787,
) {
    val address: String get() = "${host.trim()}:$port"

    /**
     * Always plain `http`. That is the router's own trust model — one password over a
     * network you control — and why the app ships a cleartext network-security-config.
     *
     * The host must be a bare IP. The router refuses an off-host request whose `Host`
     * header is a registrable name, as protection against DNS rebinding, so
     * `http://macbook.local:8787` gets a 403 even with the right password.
     */
    val baseUrl: String get() = "http://${host.trim()}:$port"
}

/**
 * Every router this phone knows, and the per-router state built on top.
 *
 * Addresses go in a JSON file; passwords never do — those live in [Secrets], keyed by the
 * router's id so renaming one or moving it to a new address does not lose its password.
 */
class RouterStore(private val context: Context) {

    val secrets = Secrets(context)
    private val file = File(context.filesDir, "routers.json")

    private var _routers: MutableList<Router> = load()
    val routers: List<Router> get() = _routers

    private fun load(): MutableList<Router> = runCatching {
        if (!file.exists()) return@runCatching mutableListOf<Router>()
        routerJson.decodeFromString<List<Router>>(file.readText()).toMutableList()
    }.getOrElse { mutableListOf() }

    /** Written through a temporary file: a half-written list is worse than an old one. */
    private fun save() {
        runCatching {
            val tmp = File(file.parentFile, "routers.json.tmp")
            tmp.writeText(routerJson.encodeToString(_routers))
            tmp.renameTo(file)
        }
    }

    fun add(router: Router, password: String?) {
        _routers.add(router)
        secrets.set(password, router.id)
        save()
    }

    /** A blank password means "keep the one already stored", as the edit form promises. */
    fun update(router: Router, password: String?) {
        val i = _routers.indexOfFirst { it.id == router.id }
        if (i >= 0) _routers[i] = router else _routers.add(router)
        if (!password.isNullOrEmpty()) secrets.set(password, router.id)
        save()
    }

    fun remove(id: String) {
        _routers.removeAll { it.id == id }
        secrets.remove(id)
        save()
    }

    fun byId(id: String): Router? = _routers.firstOrNull { it.id == id }

    fun password(id: String): String? = secrets.get(id)

    /** A client for this router, or null when there is no usable address. */
    fun client(router: Router): RouterClient? {
        if (router.host.isBlank() || router.port !in 1..65535) return null
        return RouterClient(router.baseUrl, secrets.get(router.id))
    }
}
