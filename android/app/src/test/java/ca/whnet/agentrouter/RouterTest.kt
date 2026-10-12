package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.AuthStatus
import ca.whnet.agentrouter.api.routerJson
import ca.whnet.agentrouter.data.Router
import ca.whnet.agentrouter.data.RouterAddress
import kotlinx.serialization.encodeToString
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A router is `http://host:port` on a Mac, or `https://name[:port]` when it is hosted. Every
 * router saved before https existed has no scheme in the file, and must go on meaning exactly
 * what it meant.
 */
class RouterTest {
    @Test
    fun `token-only router auth retains the active token count`() {
        val auth = routerJson.decodeFromString<AuthStatus>(
            """{"configured":false,"access_tokens":3,"remote_bind":true,"from_loopback":false,"mode":"server"}""",
        )
        val encoded = routerJson.encodeToString(auth)
        assertTrue("A token-only router must not be reported as unguarded: $encoded", encoded.contains("\"access_tokens\":3"))
        assertTrue(auth.hasCredentials)
        assertEquals("access tokens", auth.credentialStatus)
        for (tokens in listOf("", ",\"access_tokens\":null", ",\"access_tokens\":0")) {
            val empty = routerJson.decodeFromString<AuthStatus>("{\"configured\":false$tokens}")
            assertFalse(empty.hasCredentials)
        }
        assertTrue(routerJson.decodeFromString<AuthStatus>("{\"configured\":true}").hasCredentials)
    }


    private fun hosted(port: Int) =
        Router(id = "h", name = "Hosted", host = "agent-router.example.com", port = port, scheme = Router.Scheme.Https)

    // ------------------------------------------------------------ the saved file

    @Test
    fun `a router saved before https reads as the http router it was`() {
        // As routers.json has always held them: the port only when it is not the default,
        // and the host as it was typed.
        val saved = """[{"id":"r1","name":"Mac","host":"10.0.0.5","port":8788},
                        {"id":"r2","name":"Studio","host":" 10.0.0.6 "}]"""
        val (a, b) = routerJson.decodeFromString<List<Router>>(saved)

        assertEquals(Router.Scheme.Http, a.scheme)
        assertEquals("http://10.0.0.5:8788", a.baseUrl)
        assertEquals("10.0.0.5:8788", a.address)

        assertEquals(Router.Scheme.Http, b.scheme)
        assertEquals(8787, b.port)
        assertEquals("http://10.0.0.6:8787", b.baseUrl)
        assertEquals("10.0.0.6:8787", b.address)
    }

    @Test
    fun `an http router is written back exactly as older builds wrote it`() {
        val r = Router(id = "r1", name = "Mac", host = "10.0.0.5", port = 8787)
        assertEquals("""{"id":"r1","name":"Mac","host":"10.0.0.5"}""", routerJson.encodeToString(r))
        assertEquals(
            """{"id":"r1","name":"Mac","host":"10.0.0.5","port":8788}""",
            routerJson.encodeToString(r.copy(port = 8788)),
        )
    }

    @Test
    fun `a scheme this build does not know reads as http`() {
        val r = routerJson.decodeFromString<Router>("""{"id":"r","name":"X","host":"10.0.0.5","scheme":"wss"}""")
        assertEquals("http://10.0.0.5:8787", r.baseUrl)
    }

    @Test
    fun `an https router survives the file`() {
        for (port in listOf(443, 8443, 8787)) {
            val r = hosted(port)
            val json = routerJson.encodeToString(r)
            assertTrue(json, json.contains(""""scheme":"https""""))
            assertEquals(r, routerJson.decodeFromString<Router>(json))
        }
    }

    // ------------------------------------------------------------ base URL

    @Test
    fun `an https router's base URL leaves out the default port`() {
        assertEquals("https://agent-router.example.com", hosted(443).baseUrl)
        assertEquals("https://agent-router.example.com", hosted(443).address)
    }

    @Test
    fun `an https router's base URL keeps an explicit port`() {
        assertEquals("https://agent-router.example.com:8443", hosted(8443).baseUrl)
        assertEquals("https://agent-router.example.com:8443", hosted(8443).address)
    }

    @Test
    fun `an http router's base URL always names its port`() {
        val r = Router(id = "r", name = "Mac", host = "10.0.0.5", port = 80)
        assertEquals("http://10.0.0.5:80", r.baseUrl)
    }

    // ------------------------------------------------------------ the form

    private fun valid(host: String, port: String = "8787") =
        RouterAddress.parse(host, port) as? RouterAddress.Valid
            ?: throw AssertionError("expected $host / $port to be valid, got ${RouterAddress.parse(host, port)}")

    @Test
    fun `a hosted router is typed as its https URL`() {
        val a = valid("https://agent-router.example.com")
        assertEquals(Router.Scheme.Https, a.scheme)
        assertEquals("agent-router.example.com", a.host)
        assertEquals(443, a.port)
        assertTrue(a.fromUrl)
        assertEquals("https://agent-router.example.com", a.baseUrl)

        val b = valid("  HTTPS://agent-router.example.com:8443/ ")
        assertEquals(Router.Scheme.Https, b.scheme)
        assertEquals(8443, b.port)
        assertEquals("https://agent-router.example.com:8443", b.baseUrl)
    }

    @Test
    fun `an http URL is the same router as its host and port`() {
        val url = valid("http://10.0.0.5:8787", port = "1234")
        val fields = valid("10.0.0.5", port = "8787")
        assertEquals(url.baseUrl, fields.baseUrl)
        assertEquals("http://10.0.0.5:8787", fields.baseUrl)
        assertEquals(Router.Scheme.Http, fields.scheme)
        assertTrue(url.fromUrl)
        assertFalse(fields.fromUrl)
        // A URL without a port means its scheme's, whatever the Port field says.
        assertEquals(80, valid("http://10.0.0.5", port = "8787").port)
        assertEquals("http://[fd00::5]:8787", valid("http://[fd00::5]:8787").baseUrl)
    }

    @Test
    fun `addresses that cannot work are refused`() {
        assertEquals(RouterAddress.Blank, RouterAddress.parse("  ", "8787"))
        for ((host, port) in listOf(
            "10.0.0.5:8787" to "8787",
            "10.0.0.5/live" to "8787",
            "me@10.0.0.5" to "8787",
            "ftp://agent-router.example.com" to "",
            "https://" to "",
            "https://agent-router.example.com/dashboard" to "",
            "https://agent-router.example.com?x=1" to "",
            "https://me@agent-router.example.com" to "",
            "https://agent-router.example.com:0" to "",
            "https://agent-router.example.com:70000" to "",
            "https://agent router.example.com" to "",
        )) {
            val a = RouterAddress.parse(host, port)
            assertTrue("$host / $port gave $a", a is RouterAddress.Invalid && !a.inPort)
        }
        for (port in listOf("", "0", "65536")) {
            val a = RouterAddress.parse("10.0.0.5", port)
            assertTrue("port $port gave $a", a is RouterAddress.Invalid && a.inPort)
        }
    }

    // ------------------------------------------------------------ Test connection

    @Test
    fun `a hosted router passes the connection test`() {
        // `GET /api/auth` as a hosted router answers it, through its ingress.
        val auth = routerJson.decodeFromString<AuthStatus>(
            """{"configured":true,"updated_at":1790000000,"from_loopback":false,"mode":"server",
                "password_pinned":true,"public_url":"https://agent-router.example.com/",
                "remote_bind":true,"listening_on":"0.0.0.0",
                "lan_url":"https://agent-router.example.com/","env_override":false}""",
        )
        assertTrue(auth.isHosted)
        assertTrue(auth.reachableOffHost)
    }

    @Test
    fun `an answer from off the machine shows the router is reachable`() {
        val auth = routerJson.decodeFromString<AuthStatus>(
            """{"configured":true,"from_loopback":false,"remote_bind":false}""",
        )
        assertFalse(auth.isHosted)
        assertTrue(auth.reachableOffHost)
    }

    @Test
    fun `a Mac listening on loopback alone still fails the connection test`() {
        // What an emulator sees through 10.0.2.2: an answer, but only by way of loopback.
        for (json in listOf(
            """{"configured":true,"from_loopback":true,"remote_bind":false,"mode":"local"}""",
            """{"configured":true,"from_loopback":true,"remote_bind":false}""",
        )) {
            assertFalse(json, routerJson.decodeFromString<AuthStatus>(json).reachableOffHost)
        }
        val listening = routerJson.decodeFromString<AuthStatus>(
            """{"configured":true,"from_loopback":false,"remote_bind":true,"mode":"local"}""",
        )
        assertTrue(listening.reachableOffHost)
    }
}
