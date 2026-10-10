package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.RouterClient
import ca.whnet.agentrouter.data.Router
import ca.whnet.agentrouter.data.RouterState
import ca.whnet.agentrouter.feed.LiveFeed
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
 * Putting a session away, or asking on the side, when the router refuses: shown as a
 * refusal, in the router's words, whatever status came with it.
 *
 * The router refuses either with an error status and `{ok: false, reason}` (src/live.rs,
 * `bad`), and no router has ever answered either with `ok: false` in a 2xx. One must never
 * read as success all the same. Putting away discarded the reply, so a 200 saying
 * `ok: false` read as done; the side question returned it, and the screen put its reason
 * where the answer goes. Both read `ok` now, as a stop does: the put-away returns the
 * router's words as what went wrong, and the side question says them in the notice, where
 * its failure has always gone, with no answer to show.
 *
 * Each call goes through the code the screens run — [RouterState] for the put-away, the
 * session's [LiveFeed] for the side question — and the app's own client to a stand-in
 * router on a loopback port. The refusals are the router's, word for word (`live::archive`
 * and `live::aside`), each in a 200 and in the status a current router sends.
 */
class ArchiveAndAsideRefusalTest {

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

    private companion object {
        /** The router's words for a session it has no transcript of (`live::archive`). */
        const val NO_SUCH_SESSION = "No such session."

        /** The router's words for a fork that gave no answer in time (`live::aside`). */
        const val NO_ANSWER = "No answer within three minutes."

        fun refusal(words: String) = """{"ok":false,"reason":"$words"}"""
    }

    private val router = StandIn()

    @After fun stop() = router.close()

    @Test
    fun `a put-away the router refuses with ok false returns its words, never done`() = runBlocking {
        val state = RouterState(Router(name = "Stand-in", host = "127.0.0.1", port = router.port)) {
            RouterClient(it.baseUrl, null)
        }
        for (status in listOf(200, 404)) {
            router.answer("/api/live/archive", status, refusal(NO_SUCH_SESSION))
            val said = state.archive(LiveSession(sessionId = "s-1"), archived = true)
            assertEquals("$status is a refusal, not put away", NO_SUCH_SESSION, said)
        }
        assertEquals(listOf("/api/live/archive", "/api/live/archive"), router.posts.toList())
    }

    @Test
    fun `a side question the router refuses with ok false is said as a refusal, never as an answer`() = runBlocking {
        val feed = LiveFeed(LiveSession(sessionId = "s-1", wrapped = true), RouterClient(router.base, null))
        for (status in listOf(200, 504)) {
            router.answer("/api/live/aside", status, refusal(NO_ANSWER))
            feed.notice = null
            val reply = feed.aside("What is left to do?")
            assertNull("$status: nothing answered, so nothing to show as the answer", reply)
            assertEquals("$status is a refusal, in the router's words", NO_ANSWER, feed.notice?.text)
            assertEquals("$status", false, feed.notice?.ok)
        }
        assertEquals(listOf("/api/live/aside", "/api/live/aside"), router.posts.toList())
    }
}
