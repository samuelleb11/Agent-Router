// A router as this phone knows it: a name, an address, and a password kept elsewhere.

package ca.whnet.agentrouter.data

import android.content.Context
import ca.whnet.agentrouter.api.RouterClient
import ca.whnet.agentrouter.api.routerJson
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import java.io.File
import java.util.UUID

@Serializable
data class Router(
    val id: String = UUID.randomUUID().toString(),
    val name: String,
    val host: String,
    val port: Int = 8787,
    /**
     * Last, and defaulted, so every router saved before https existed reads as the `http`
     * router it always was. The file only spells out a value that is not the default, so
     * an http router is still written exactly as older builds wrote it.
     */
    val scheme: Scheme = Scheme.Http,
) {
    @Serializable
    enum class Scheme(val text: String, val defaultPort: Int) {
        /** A router on a Mac, reached at its LAN IP. */
        @SerialName("http") Http("http", 80),

        /** A hosted router, reached by name through its HTTPS ingress. */
        @SerialName("https") Https("https", 443),
    }

    /** `10.0.0.5:8787` for a router on a Mac, as always; the whole URL for a hosted one. */
    val address: String get() = if (scheme == Scheme.Https) baseUrl else "${host.trim()}:$port"

    val baseUrl: String get() = baseUrl(scheme, host, port)

    companion object {
        /**
         * `http://host:port`, port always written, exactly as every router before https was
         * reached. That is the router's own trust model — one password over a network you
         * control — and why the app ships a cleartext network-security-config. Its host must
         * be a bare IP: the router refuses an off-host request whose `Host` header is a
         * registrable name, as protection against DNS rebinding, so
         * `http://macbook.local:8787` gets a 403 even with the right password.
         *
         * `https://name`, with the port only when it is not 443, for a hosted router. That
         * one is reached by the name its deployment allows, through an ingress whose
         * certificate the system already trusts.
         */
        fun baseUrl(scheme: Scheme, host: String, port: Int): String = when (scheme) {
            Scheme.Http -> "http://${host.trim()}:$port"
            Scheme.Https ->
                if (port == Scheme.Https.defaultPort) "https://${host.trim()}" else "https://${host.trim()}:$port"
        }
    }
}

/**
 * What the form's Host and Port fields say, read the way a [Router] is stored.
 *
 * The Host field takes a host whose port is in the field beside it — the form as it always
 * was — or a whole URL with its scheme, which brings its own port: the one it names, or its
 * scheme's. So `https://agent-router.example.com` is a hosted router on 443, and
 * `http://10.0.0.5:8787` is the same router as host `10.0.0.5`, port `8787`.
 */
sealed interface RouterAddress {
    /** Nothing typed yet: not valid, and nothing to complain about either. */
    data object Blank : RouterAddress

    /** [why] is a sentence for the form; [inPort] says which field is at fault. */
    data class Invalid(val why: String, val inPort: Boolean = false) : RouterAddress

    data class Valid(
        val scheme: Router.Scheme,
        val host: String,
        val port: Int,
        /** The port came from a URL, so the Port field has nothing to add. */
        val fromUrl: Boolean,
    ) : RouterAddress {
        val baseUrl: String get() = Router.baseUrl(scheme, host, port)
    }

    companion object {
        /** Whether the Host field holds a URL, whose port then wins over the Port field's. */
        fun isUrl(hostField: String): Boolean = "://" in hostField

        fun parse(hostField: String, portField: String): RouterAddress {
            val text = hostField.trim()
            if (text.isEmpty()) return Blank
            val sep = text.indexOf("://")
            if (sep < 0) {
                if (!text.startsWith("[") && ':' in text) {
                    return Invalid("Put the port in its own field, or type the whole URL, such as http://10.0.0.5:8787")
                }
                val port = portField.trim().toIntOrNull()?.takeIf { it in 1..65535 }
                    ?: return Invalid("The port is a number from 1 to 65535", inPort = true)
                return exactly(Router.Scheme.Http, text, port, fromUrl = false)
            }

            val named = text.substring(0, sep).lowercase()
            val scheme = Router.Scheme.entries.firstOrNull { it.text == named }
                ?: return Invalid("A router's URL starts with http:// or https://")
            val rest = text.substring(sep + 3)
            val cut = rest.indexOfAny(charArrayOf('/', '?', '#')).let { if (it < 0) rest.length else it }
            val authority = rest.substring(0, cut)
            val tail = rest.substring(cut)
            if (tail.isNotEmpty() && tail != "/") {
                return Invalid("Leave out the path: a router answers at the root of its address")
            }
            if (authority.isEmpty()) return Invalid("Type the router's address after ${scheme.text}://")
            if ('@' in authority) return Invalid("Leave out the user name: the password has its own field")

            // The port follows the last colon, unless that colon is inside an IPv6 literal.
            val colon = authority.lastIndexOf(':').takeIf { it > authority.lastIndexOf(']') } ?: -1
            val host = if (colon < 0) authority else authority.substring(0, colon)
            val port = if (colon < 0) {
                scheme.defaultPort
            } else {
                authority.substring(colon + 1).toIntOrNull()?.takeIf { it in 1..65535 }
                    ?: return Invalid("The port is a number from 1 to 65535")
            }
            return exactly(scheme, host, port, fromUrl = true)
        }

        /**
         * The URL built from what was typed must name exactly this host and port and nothing
         * more. A host with a slash or an `@` in it still makes *a* URL, just not one whose
         * requests reach the router.
         */
        private fun exactly(scheme: Router.Scheme, host: String, port: Int, fromUrl: Boolean): RouterAddress {
            val url = Router.baseUrl(scheme, host, port).toHttpUrlOrNull()
            val exact = url != null && url.port == port && url.encodedPath == "/" &&
                url.query == null && url.fragment == null &&
                url.username.isEmpty() && url.password.isEmpty()
            return if (exact) Valid(scheme, host, port, fromUrl) else Invalid("That is not a host name or an IP address")
        }
    }
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
