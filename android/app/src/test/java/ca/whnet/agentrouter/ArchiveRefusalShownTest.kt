package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.RouterClient
import ca.whnet.agentrouter.data.Router
import ca.whnet.agentrouter.data.RouterState
import ca.whnet.agentrouter.feed.LiveFeed
import ca.whnet.agentrouter.ui.PutAway
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.Collections
import java.util.concurrent.ConcurrentHashMap
import kotlin.concurrent.thread

/**
 * A put-away the router refuses is shown as one, in the router's words, on the screen it was
 * asked from: the open session stays open and says why, and the Sessions list says why.
 *
 * [RouterState.archive] has returned the router's words for a refusal since AGEN-0026, but
 * neither screen acted on them. The open session went back to the list whatever came back, so
 * a refused put-away closed the session as if it had worked and its words went nowhere; the
 * list dropped the answer, so a refusal there showed nothing. Each screen now hands the answer
 * to [PutAway], and that is what is held here: the composables are one call into it.
 *
 * Each put-away goes through the code the screens run, [RouterState] and the session's
 * [LiveFeed], and the app's own client to a stand-in router on a loopback port. The refusal is
 * the router's, word for word (`live::archive`), in the 404 a current router sends and in a
 * 200, which no router has sent but which must never read as done either.
 */
class ArchiveRefusalShownTest {

    /** Just enough HTTP/1.1 for one request per connection: each path answered from a table. */
    private class StandIn : AutoCloseable {
        private val listener = ServerSocket(0, 8, InetAddress.getLoopbackAddress())
        val port = listener.localPort
        val base = "http://127.0.0.1:$port"
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
            val type = if (text.startsWith("{") || text.startsWith("[")) "application/json" else "text/plain"
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

    private companion object {
        /** The router's words for a session it has no transcript of (`live::archive`). */
        const val NO_SUCH_SESSION = "No such session."

        /** What the router answers once it has put a session away, or brought one back. */
        const val DONE = """{"ok":true,"archived":1}"""

        fun refusal(words: String) = """{"ok":false,"reason":"$words"}"""
    }

    private val router = StandIn()
    private val state = RouterState(Router(name = "Stand-in", host = "127.0.0.1", port = router.port)) {
        RouterClient(it.baseUrl, null)
    }
    private val session = LiveSession(sessionId = "s-1", wrapped = true)
    private val feed = LiveFeed(session, RouterClient(router.base, null))

    init {
        // Asked for again once a put-away is done, so the row moves where the listing puts it.
        router.answer("/api/live/sessions", 200, "[]")
    }

    @After fun stop() = router.close()

    @Test
    fun `a put-away the router refuses leaves the open session open, saying why in its words`() = runBlocking {
        for (status in listOf(200, 404)) {
            router.answer("/api/live/archive", status, refusal(NO_SUCH_SESSION))
            feed.notice = null
            var wentBack = 0
            PutAway.fromSession(state, session, feed) { wentBack++ }
            assertEquals("$status: refused, so the session stays open", 0, wentBack)
            assertEquals("$status: the router's words, in the screen's notice", NO_SUCH_SESSION, feed.notice?.text)
            assertEquals("$status: said as a refusal", false, feed.notice?.ok)
        }
        assertEquals(listOf("/api/live/archive", "/api/live/archive"), router.posts.toList())
    }

    @Test
    fun `a put-away the router makes goes back to the list, with nothing to say`() = runBlocking {
        router.answer("/api/live/archive", 200, DONE)
        var wentBack = 0
        PutAway.fromSession(state, session, feed) { wentBack++ }
        assertEquals("put away, so back to the list", 1, wentBack)
        assertNull("nothing went wrong", feed.notice)
    }

    @Test
    fun `an archive the router refuses from the Sessions list says why in its words`() = runBlocking {
        // Put away by a swipe, or brought back from the fold: the same answer either way.
        for (archived in listOf(true, false)) for (status in listOf(200, 404)) {
            router.answer("/api/live/archive", status, refusal(NO_SUCH_SESSION))
            val said = mutableListOf<String>()
            PutAway.fromList(state, session, archived) { said += it }
            assertEquals("archived=$archived at $status: the router's words, once", listOf(NO_SUCH_SESSION), said)
        }
        assertEquals(List(4) { "/api/live/archive" }, router.posts.toList())
    }

    @Test
    fun `an archive the router makes from the Sessions list says nothing`() = runBlocking {
        router.answer("/api/live/archive", 200, DONE)
        val said = mutableListOf<String>()
        PutAway.fromList(state, session, archived = true) { said += it }
        assertEquals("done: the listing moves the row, and there is nothing to say", emptyList<String>(), said)
    }
}
