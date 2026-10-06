package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.RouterClient
import ca.whnet.agentrouter.feed.LiveFeed
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Test
import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.Collections
import java.util.concurrent.ConcurrentHashMap
import kotlin.concurrent.thread

/**
 * A stop or a permission answer the router refuses is shown as a refusal, in the router's
 * words, whatever status came with it.
 *
 * The router refuses either with an error status and `{ok: false, reason}` (src/live.rs,
 * `bad`), and no router has ever answered either with `ok: false` in a 2xx. One must never
 * read as success all the same. Stop read the status alone, so a 200 saying `ok: false` said "Asked it to
 * stop."; it reads `ok` now, as the two answers do. The permission answer already read it,
 * and this keeps it so: the card stays up, saying why.
 *
 * Each call goes through the feed the session screen runs and the app's own client to a
 * stand-in router on a loopback port. The refusals are the router's, word for word
 * (`live::interrupt`'s `stop_refusal`, and `live::answer`'s `pick_prompt`), each in a 200 and
 * in the 409 a current router sends.
 */
class InterruptAndPermissionRefusalTest {

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

    private companion object {
        /** The router's words for a stop asked of a session that has closed (`stop_refusal`). */
        const val NOT_RUNNING = "That session isn't running any more."

        /** The router's words for an answer naming a prompt that is not waiting (`pick_prompt`). */
        const val GONE = "That prompt is no longer waiting"

        /** A Bash approval waiting, as `/api/live/prompt` reports it. */
        const val PERMISSION = """{"pending":true,"request_id":"req-P","tool":"Bash","question":false,""" +
            """"display_name":"Bash","description":"rm -r build","count":1}"""

        fun refusal(words: String) = """{"ok":false,"reason":"$words"}"""
    }

    private val router = StandIn()
    private val feed = LiveFeed(LiveSession(sessionId = "s-1", wrapped = true), RouterClient(router.base, null))

    @After fun stop() = router.close()

    @Test
    fun `a stop the router refuses with ok false is shown as a refusal, never as asked`() = runBlocking {
        for (status in listOf(200, 409)) {
            router.answer("/api/live/interrupt", status, refusal(NOT_RUNNING))
            feed.notice = null
            feed.interrupt()
            assertEquals("$status is a refusal, not \"Asked it to stop.\"", NOT_RUNNING, feed.notice?.text)
            assertEquals("$status", false, feed.notice?.ok)
            assertFalse("$status: no longer stopping", feed.stopping)
        }
        assertEquals(listOf("/api/live/interrupt", "/api/live/interrupt"), router.posts.toList())
    }

    /** The card tapped is the one waiting; the router refuses the tap's answer. The card stays up. */
    @Test
    fun `a permission answer the router refuses with ok false is shown as a refusal`() = runBlocking {
        router.answer(LiveFeed.path(LiveFeed.PROMPT, "s-1"), 200, PERMISSION)
        feed.loadPrompt()
        val card = feed.pendingPrompt
        assertNotNull("the card the reader taps", card)
        for (status in listOf(200, 409)) {
            router.answer("/api/live/answer", status, refusal(GONE))
            feed.notice = null
            feed.answerPermission(card!!, allow = true, said = "Allowed.")
            assertEquals("$status is a refusal, not \"Allowed.\"", GONE, feed.notice?.text)
            assertEquals("$status", false, feed.notice?.ok)
            assertEquals("$status: still waiting, so still shown", "req-P", feed.pendingPrompt?.requestId)
        }
        assertEquals(listOf("/api/live/answer", "/api/live/answer"), router.posts.toList())
    }
}
