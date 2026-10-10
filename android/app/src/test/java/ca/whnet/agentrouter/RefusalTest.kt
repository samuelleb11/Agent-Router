package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.ConfigToggleBody
import ca.whnet.agentrouter.api.LoginStartBody
import ca.whnet.agentrouter.api.OkReply
import ca.whnet.agentrouter.api.PasswordBody
import ca.whnet.agentrouter.api.PushTestReply
import ca.whnet.agentrouter.api.RouterClient
import ca.whnet.agentrouter.api.RouterFailure
import ca.whnet.agentrouter.api.ServiceBody
import ca.whnet.agentrouter.api.ShimAutoBody
import ca.whnet.agentrouter.api.ShimBody
import ca.whnet.agentrouter.api.UsageTokenBody
import ca.whnet.agentrouter.api.post
import ca.whnet.agentrouter.api.postEmpty
import ca.whnet.agentrouter.data.RouterState
import ca.whnet.agentrouter.data.change
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.Collections
import java.util.concurrent.ConcurrentHashMap
import kotlin.concurrent.thread

/**
 * A refused change reaches the screen in the router's own words.
 *
 * The router refuses a change with an error status and the reason under `error` — the
 * wrapper switches under `reason` as well — and a router from before that answered the same
 * refusal as a 200 carrying `error`. Either way the screen shows the sentence the router
 * wrote: never nothing, and never "Router returned 400".
 *
 * Each call goes to a stand-in router on a loopback port through the app's own client and
 * the same code the settings and accounts screens run: [change] for the switches, and a
 * failure through [RouterState.describe] for the replies with a shape of their own. The
 * replies are the router's, word for word (src/refusal.rs and the handlers it names).
 */
class RefusalTest {

    /** Just enough HTTP/1.1 for one request per connection: each path answered from a table. */
    private class StandIn : AutoCloseable {
        private val listener = ServerSocket(0, 8, InetAddress.getLoopbackAddress())
        val base = "http://127.0.0.1:${listener.localPort}"
        private val table = ConcurrentHashMap<String, Pair<Int, String>>()
        val posts: MutableList<String> = Collections.synchronizedList(mutableListOf())

        fun answer(path: String, status: Int, body: String) {
            table[path] = status to body
        }

        init {
            thread(isDaemon = true, name = "stand-in router") {
                while (!listener.isClosed) {
                    val s = try { listener.accept() } catch (_: IOException) { break }
                    thread(isDaemon = true) { serve(s) }
                }
            }
        }

        private fun serve(s: Socket): Unit = s.use {
            val input = s.getInputStream().bufferedReader(Charsets.UTF_8)
            val (method, path) = input.readLine()?.split(' ')?.takeIf { it.size >= 2 } ?: return
            var length = 0
            while (true) {
                val h = input.readLine() ?: return
                if (h.isEmpty()) break
                if (h.lowercase().startsWith("content-length:")) length = h.substringAfter(':').trim().toInt()
            }
            // Read and dropped: what is asked is the path, and every body here is ASCII JSON.
            val body = CharArray(length)
            var n = 0
            while (n < length) {
                val r = input.read(body, n, length - n)
                if (r < 0) break
                n += r
            }
            if (method == "POST") posts.add(path)
            val (status, text) = table[path] ?: (404 to "")
            val reply = text.toByteArray(Charsets.UTF_8)
            val type = if (text.startsWith("{")) "application/json" else "text/plain"
            val out = s.getOutputStream()
            out.write(
                ("HTTP/1.1 $status Answer\r\ncontent-type: $type\r\ncontent-length: ${reply.size}\r\n" +
                    "connection: close\r\n\r\n").toByteArray(Charsets.UTF_8),
            )
            out.write(reply)
            out.flush()
        }

        override fun close() = listener.close()
    }

    private val router = StandIn()
    private val client = RouterClient(router.base, null)

    @After fun stop() = router.close()

    private fun refusal(words: String) = """{"ok":false,"error":"$words"}"""

    private val hosted = "this router is hosted: it does not run on a developer's machine, so there is " +
        "no Claude Code, login item, editor extension or network switch here to manage — set those up on each machine"

    @Serializable private data class NetworkBody(val enable: Boolean)

    @Serializable private data class PushConfigBody(
        @SerialName("team_id") val teamId: String,
        @SerialName("key_id") val keyId: String,
        val p8: String,
    )

    @Serializable private data class SendBody(val session: String, val text: String)

    @Test
    fun `a refused password is shown in the router's words`() = runBlocking {
        router.answer("/api/auth", 400, refusal("use at least 8 characters"))
        assertEquals("use at least 8 characters", client.change("/api/auth", PasswordBody(password = "short")))

        val pinned = "the password is set by the deployment (AGENT_ROUTER_SERVER_PASSWORD); " +
            "change the Secret and restart the router"
        router.answer("/api/auth", 409, refusal(pinned))
        assertEquals(pinned, client.change("/api/auth", PasswordBody(password = "a new password")))
        assertEquals(listOf("/api/auth", "/api/auth"), router.posts.toList())
    }

    /** No screen on the phone throws this switch; the client reads its refusal all the same. */
    @Test
    fun `a refused network switch is shown in the router's words`() = runBlocking {
        val words = "set a password first — until one exists, callers from other machines are refused"
        router.answer("/api/network", 409, refusal(words))
        assertEquals(words, client.change("/api/network", NetworkBody(enable = true)))
    }

    @Test
    fun `a refused login item switch is shown in the router's words`() = runBlocking {
        router.answer("/api/setup/service", 409, refusal(hosted))
        assertEquals(hosted, client.change("/api/setup/service", ServiceBody(true)))

        // A failure, not a refusal: the router's words all the same.
        val launchd = "launchctl bootstrap gui/501 /Users/u/Library/LaunchAgents/ca.whnet.agent-router.plist: " +
            "Bootstrap failed: 5: Input/output error"
        router.answer("/api/setup/service", 500, refusal(launchd))
        assertEquals(launchd, client.change("/api/setup/service", ServiceBody(true)))
    }

    @Test
    fun `a refused routing switch is shown in the router's words`() = runBlocking {
        router.answer("/api/setup/config", 400, refusal("/Users/u/code/nowhere is not a directory"))
        assertEquals(
            "/Users/u/code/nowhere is not a directory",
            client.change("/api/setup/config", ConfigToggleBody(enable = true, project = "/Users/u/code/nowhere")),
        )

        val broken = "/Users/u/.claude/settings.json is not valid JSON (expected value at line 1 column 1) " +
            "— fix or move it first"
        router.answer("/api/setup/config", 409, refusal(broken))
        assertEquals(broken, client.change("/api/setup/config", ConfigToggleBody(enable = true)))
    }

    @Test
    fun `a refused wrapper switch is shown in the router's words`() = runBlocking {
        val stale = "Not a known extension binary."
        router.answer("/api/setup/shim", 409, """{"ok":false,"reason":"$stale","error":"$stale"}""")
        assertEquals(stale, client.change("/api/setup/shim", ShimBody("/x/claude", "install")))

        router.answer(
            "/api/setup/shim", 400,
            """{"ok":false,"reason":"unknown action \"replace\"","error":"unknown action \"replace\""}""",
        )
        assertEquals("unknown action \"replace\"", client.change("/api/setup/shim", ShimBody("/x/claude", "replace")))

        router.answer("/api/setup/shim/auto", 409, refusal(hosted))
        assertEquals(hosted, client.change("/api/setup/shim/auto", ShimAutoBody(true)))
    }

    /** The phone has no key form, but the client reads this refusal the same way. */
    @Test
    fun `a refused push key is shown in the router's words`() = runBlocking {
        val words = "that key does not sign: that is not a PEM private key"
        router.answer("/api/push/config", 422, refusal(words))
        assertEquals(words, client.change("/api/push/config", PushConfigBody("ABCDE12345", "ABC123DEF4", "not a key")))
    }

    /** As the settings screen's Send test reads it: its reply has a shape of its own. */
    @Test
    fun `a refused push test is shown in the router's words`() = runBlocking {
        router.answer("/api/push/test", 409, refusal("no push credentials are set"))
        val said = runCatching {
            val r = client.postEmpty<PushTestReply>("/api/push/test")
            r.error ?: r.results.joinToString("\n") { "…${it.device}: ${if (it.ok) "delivered" else it.error}" }
        }.getOrElse { RouterState.describe(it) }
        assertEquals("no push credentials are set", said)
    }

    @Test
    fun `a refused usage key is shown in the router's words`() = runBlocking {
        val words = "that does not look like a claude.ai session key (expected sk-ant-sid…)"
        router.answer("/api/usage-token", 400, refusal(words))
        assertEquals(words, client.change("/api/usage-token", UsageTokenBody("default", "sk-ant-api03-abc")))
    }

    /** As the add-account dialog reads it: the start's problem, or the failure described. */
    @Test
    fun `a refused sign-in is shown in the router's words`() = runBlocking {
        router.answer("/api/login/start", 409, refusal("a sign-in is already in progress"))
        val busy = runCatching {
            client.post<LoginStartBody, OkReply>("/api/login/start", LoginStartBody("work")).problem
        }.getOrElse { RouterState.describe(it) }
        assertEquals("a sign-in is already in progress", busy)

        router.answer("/api/login/start", 400, refusal("give the account a name first"))
        assertEquals("give the account a name first", client.change("/api/login/start", LoginStartBody(" ")))
    }

    /** A router from before refusals had a status: the same words, in a 200. */
    @Test
    fun `an older router's refusal still reads in its words`() = runBlocking {
        router.answer("/api/auth", 200, """{"error":"use at least 8 characters"}""")
        assertEquals("use at least 8 characters", client.change("/api/auth", PasswordBody(password = "short")))
        router.answer("/api/setup/shim/auto", 200, """{"ok":false,"reason":"database is locked"}""")
        assertEquals("database is locked", client.change("/api/setup/shim/auto", ShimAutoBody(true)))
        // And a change that went through says nothing at all.
        router.answer("/api/setup/service", 200, """{"ok":true}""")
        assertEquals(null, client.change("/api/setup/service", ServiceBody(false)))
    }

    /**
     * The live endpoints a hosted router relays refuse in the same shape — 404 for a session
     * nobody holds, 503 for a machine that is offline, 502 for a relay that failed — and read
     * the same way, with nothing more to it.
     */
    @Test
    fun `a relayed live refusal reads in its words too`() = runBlocking {
        for ((status, body, words) in listOf(
            Triple(404, """{"ok":false,"error":"No such session."}""", "No such session."),
            Triple(503, """{"ok":false,"error":"the machine holding that session is offline"}""", "the machine holding that session is offline"),
            Triple(502, """{"ok":false,"reason":"the relay failed"}""", "the relay failed"),
        )) {
            router.answer("/api/live/send", status, body)
            val e = runCatching { client.post<SendBody, OkReply>("/api/live/send", SendBody("s", "hi")) }.exceptionOrNull()
            assertTrue("$status is not a reply", e is RouterFailure.Refused)
            assertEquals(words, RouterState.describe(e!!))
        }
    }

    /** An error status with no words of the router's in it still says what it was. */
    @Test
    fun `an error status with nothing to say still names its status`() = runBlocking {
        router.answer("/api/auth", 403, "agent-router: caller is not on this machine")
        assertEquals(
            "Router returned 403: agent-router: caller is not on this machine",
            client.change("/api/auth", PasswordBody(password = "long enough")),
        )
        router.answer("/api/auth", 401, "agent-router: password required")
        assertEquals("Wrong password for this router", client.change("/api/auth", PasswordBody(password = "long enough")))
    }
}
