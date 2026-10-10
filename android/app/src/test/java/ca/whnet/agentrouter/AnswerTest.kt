package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.RouterClient
import ca.whnet.agentrouter.feed.LiveFeed
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.Collections
import kotlin.concurrent.thread

/**
 * Answering a question from the feed, against a stand-in router on a loopback port.
 *
 * The feed keeps every question form it has drawn, and a form is only marked answered when
 * this phone answered it. So a question answered in the editor leaves its form live here,
 * and once the model has asked the next one, that stale form is still one tap from sending.
 * The router's prompt says which questions are waiting now; answers to any others must not
 * go to it — the router would write them into the new question and approve it, dismissing
 * it in the editor before anyone had read it.
 */
class AnswerTest {

    /** Just enough HTTP/1.1 for one request per connection, every request written down. */
    private class StandIn(private val prompt: String) : AutoCloseable {
        private val listener = ServerSocket(0, 8, InetAddress.getLoopbackAddress())
        val base = "http://127.0.0.1:${listener.localPort}"
        val seen: MutableList<String> = Collections.synchronizedList(mutableListOf())

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
            // Every body here is ASCII JSON, so its length in bytes is its length in chars.
            val body = CharArray(length)
            var n = 0
            while (n < length) {
                val r = input.read(body, n, length - n)
                if (r < 0) break
                n += r
            }
            seen.add("$method $path ${String(body, 0, n)}".trim())
            val reply = when {
                path.startsWith("/api/live/prompt") -> prompt
                path.startsWith("/api/live/answer") -> """{"ok":true,"request_id":"req-Q2"}"""
                else -> """{"ok":true,"via":"wrapper"}"""
            }.toByteArray(Charsets.UTF_8)
            val out = s.getOutputStream()
            out.write(
                ("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: ${reply.size}\r\n" +
                    "connection: close\r\n\r\n").toByteArray(Charsets.UTF_8),
            )
            out.write(reply)
            out.flush()
        }

        val posts: List<String> get() = synchronized(seen) { seen.filter { it.startsWith("POST") } }

        override fun close() = listener.close()
    }

    /** The question the model asked second, which is what the router says is waiting now. */
    private val router = StandIn(
        """{"pending":true,"request_id":"req-Q2","tool":"AskUserQuestion","question":true,
            "display_name":null,"description":null,"reason":null,
            "input":{"questions":[{"question":"Which database?","header":"DB","multiSelect":false,
                "options":[{"label":"Postgres","description":""},{"label":"SQLite","description":""}]}]},
            "suggestions":[],"count":1}""".replace(Regex("\\s*\\n\\s*"), ""),
    )

    @After fun stop() = router.close()

    private fun feed() = LiveFeed(
        LiveSession(sessionId = "s-1", wrapped = true, activity = "waiting", tone = "attention"),
        RouterClient(router.base, null),
    )

    @Test
    fun `a form for a question no longer asked is not sent to the one asked now`() = runBlocking {
        val feed = feed()
        // The first question's form, answered in the editor long ago and still live here.
        val sent = feed.answer(
            listOf("Which framework?"), mapOf("Which framework?" to "React"), listOf("Framework: React"), "row-q1",
        )
        assertFalse("nothing was answered", sent)
        assertEquals(
            "neither an answer nor a stray message reached the session",
            emptyList<String>(), router.posts,
        )
        assertEquals("Not sent: a different question is waiting now.", feed.notice?.text)
        assertEquals(false, feed.notice?.ok)
    }

    @Test
    fun `a form for the question asked now answers it by name`() = runBlocking {
        val feed = feed()
        val sent = feed.answer(
            listOf("Which database?"), mapOf("Which database?" to "Postgres"), listOf("DB: Postgres"), "row-q2",
        )
        assertTrue(sent)
        assertEquals(
            listOf("""POST /api/live/answer {"session":"s-1","answers":{"Which database?":"Postgres"},"request_id":"req-Q2"}"""),
            router.posts,
        )
        assertEquals("Answered.", feed.notice?.text)
    }
}
