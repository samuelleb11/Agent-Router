package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.RouterClient
import ca.whnet.agentrouter.feed.LiveFeed
import ca.whnet.agentrouter.ui.AsideDialog
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
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
 * A side question the router refuses says why in the dialog it was asked from, in the
 * router's words and as a failure, never as an answer.
 *
 * The words once went to the session screen's snackbar, which sits behind the open dialog's
 * dimmed background, while the dialog itself said only "No answer came back.". The dialog's
 * state is [AsideDialog], and that is what is held here: the composable draws it, an answer
 * as text and a failure in the error colour beside an error icon.
 *
 * Each question goes through the session's [LiveFeed] and the app's own client to a stand-in
 * router on a loopback port. The refusal is the router's, word for word (`live::aside`), in
 * the 504 a current router sends and in a 200, which no router has sent but which must never
 * read as an answer either.
 */
class AsideRefusalShownTest {

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
        /** The router's words for a fork that gave no answer in time (`live::aside`). */
        const val NO_ANSWER = "No answer within three minutes."

        /** A side question answered, as `live::aside` reports it. */
        const val ANSWERED = """{"ok":true,"answer":"Two tests are left to fix.","fork":"f-1",""" +
            """"cost":0.01,"took_ms":5200}"""

        fun refusal(words: String) = """{"ok":false,"reason":"$words"}"""
    }

    private val router = StandIn()
    private val feed = LiveFeed(LiveSession(sessionId = "s-1", wrapped = true), RouterClient(router.base, null))

    @After fun stop() = router.close()

    /** The dialog as the menu opens it, with a question typed in. */
    private fun asking(question: String) = AsideDialog().apply {
        show()
        this.question = question
    }

    @Test
    fun `a side question the router refuses says why in the dialog itself, as a failure`() = runBlocking {
        for (status in listOf(200, 504)) {
            router.answer("/api/live/aside", status, refusal(NO_ANSWER))
            val dialog = asking("What is left to do?")
            dialog.ask(feed)
            assertEquals(
                "$status: the router's words, as a failure and not as an answer",
                AsideDialog.Outcome.Failure(NO_ANSWER), dialog.outcome,
            )
            assertTrue("$status: shown in the dialog, which stays open", dialog.open)
            assertFalse("$status: no longer waiting", dialog.running)
            assertNull("$status: said in the dialog, not in the snackbar behind it", feed.notice)
        }
        assertEquals(listOf("/api/live/aside", "/api/live/aside"), router.posts.toList())
    }

    @Test
    fun `a side question answered is shown as the answer`() = runBlocking {
        router.answer("/api/live/aside", 200, ANSWERED)
        val dialog = asking("What is left to do?")
        dialog.ask(feed)
        assertEquals(AsideDialog.Outcome.Answer("Two tests are left to fix."), dialog.outcome)
        assertTrue("shown in the dialog, which stays open", dialog.open)
        assertNull("nothing went wrong", feed.notice)
    }
}
