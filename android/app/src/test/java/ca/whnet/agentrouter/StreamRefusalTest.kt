package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.RouterClient
import ca.whnet.agentrouter.api.RouterFailure
import ca.whnet.agentrouter.data.RouterState
import ca.whnet.agentrouter.feed.LiveFeed
import ca.whnet.agentrouter.ui.StatusText
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread

/**
 * A live stream the router refuses with an error status reaches the screen in the router's
 * own words, and the live feed never names a status the router did not send.
 *
 * A stream once threw its bare status without reading the body, so a hosted router's "the
 * machine running this session (mini) is offline" read "Router returned 503", and the feed
 * then said nothing at all. A hosted router relays each session's stream to the machine
 * holding it, and refuses as `machines::Relay` does: 503 naming the machine when it is
 * offline — back later, never a session that ended — 404 when no machine holds the session,
 * and 502 when the relay failed. Each is said through the same reading every call uses
 * ([RouterClient.failure]); the feed goes on retrying, and says why in its header.
 */
class StreamRefusalTest {

    /** Just enough HTTP/1.1 for one request per connection: each path answered from a table. */
    private class StandIn : AutoCloseable {
        private val listener = ServerSocket(0, 16, InetAddress.getLoopbackAddress())
        val base = "http://127.0.0.1:${listener.localPort}"
        private val table = ConcurrentHashMap<String, Pair<Int, String>>()

        /** How many times the offline session's stream is refused before its machine is back. */
        @Volatile var offlineFor = 0
        val offlineConnections = AtomicInteger()

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
            val path = input.readLine()?.split(' ')?.getOrNull(1) ?: return
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
            val out = s.getOutputStream()
            fun write(text: String) {
                out.write(text.toByteArray(Charsets.UTF_8))
                out.flush()
            }
            fun reply(status: Int, text: String) {
                val bytes = text.toByteArray(Charsets.UTF_8)
                val type = if (text.startsWith("{")) "application/json" else "text/plain"
                write("HTTP/1.1 $status Answer\r\ncontent-type: $type\r\ncontent-length: ${bytes.size}\r\nconnection: close\r\n\r\n")
                out.write(bytes)
                out.flush()
            }
            if (path == STREAM_OFFLINE) {
                if (offlineConnections.incrementAndGet() <= offlineFor) {
                    reply(503, refusal(OFFLINE))
                    return
                }
                // The machine is back: its stream, until the phone hangs up.
                write("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n")
                write("event: status\ndata: {\"state\":\"working\",\"detail\":\"Thinking\",\"quiet_ms\":0}\n\n")
                try {
                    while (input.read() >= 0) { /* a GET sends nothing more: this ends at hang-up */ }
                } catch (_: IOException) { }
                return
            }
            val (status, text) = table[path] ?: (404 to "")
            reply(status, text)
        }

        override fun close() = listener.close()
    }

    private companion object {
        /** A hosted router's words for a machine that is offline (src/machines.rs, `Relay`). */
        const val OFFLINE = "the machine running this session (mini) is offline"
        const val STREAM_OFFLINE = "/api/live/stream?session=offline"

        fun refusal(words: String) = """{"ok":false,"error":"$words"}"""
    }

    private val router = StandIn()
    private val client = RouterClient(router.base, null)

    @After fun stop() = router.close()

    @Test
    fun `an error status on a live stream is said in the router's words`() = runBlocking {
        for ((session, status, words) in listOf(
            Triple("gone", 404, "No such session."),
            Triple("down", 503, OFFLINE),
            Triple("relay", 502, "mini did not answer in time"),
        )) {
            val path = LiveFeed.path(LiveFeed.STREAM, session)
            router.answer(path, status, refusal(words))
            val e = runCatching { client.events(path).toList() }.exceptionOrNull()
            assertTrue("$status must not look like an empty stream: $e", e is RouterFailure.Refused)
            assertEquals(status, (e as RouterFailure.Refused).code)
            assertEquals(words, RouterState.describe(e))
        }
    }

    /** With nothing to say for itself, an error status is named — the one that was sent. */
    @Test
    fun `an error status with no words names the status the router sent`() = runBlocking {
        val wordless = LiveFeed.path(LiveFeed.STREAM, "wordless")
        router.answer(wordless, 503, "")
        val e = runCatching { client.events(wordless).toList() }.exceptionOrNull()
        assertTrue("$e", e is RouterFailure.Status)
        assertEquals("Router returned 503", RouterState.describe(e!!))

        val plain = LiveFeed.path(LiveFeed.STREAM, "plain")
        router.answer(plain, 500, "upstream exploded")
        val e2 = runCatching { client.events(plain).toList() }.exceptionOrNull()
        assertEquals("Router returned 500: upstream exploded", RouterState.describe(e2!!))
    }

    /**
     * Offline is not ended: the feed says what the router said, once, keeps the header on
     * it while it retries, and is reading the session again as soon as its machine is back.
     */
    @Test
    fun `a session whose machine is offline reads as offline until it is back`() = runBlocking {
        router.offlineFor = 3
        val feed = LiveFeed(LiveSession(sessionId = "offline"), client)
        feed.retryMs = 50
        val running = launch { feed.run() }
        try {
            waitFor("the refusal") { feed.refused != null }
            assertEquals(OFFLINE, feed.refused)
            assertEquals(OFFLINE, feed.notice?.text)
            assertEquals(false, feed.notice?.ok)
            assertFalse(feed.connected)
            assertNull("nothing said the session ended", feed.status)
            assertEquals(
                OFFLINE,
                StatusText.header(feed.status, feed.connected, feed.statusFresh, "project", feed.refused),
            )
            val said = feed.notice
            waitFor("the retries") { router.offlineConnections.get() >= 3 }
            assertSame("said once, not again on every retry", said, feed.notice)

            waitFor("the machine back") { feed.statusFresh }
            assertNull("the stream delivers again, so nothing stands refused", feed.refused)
            assertEquals("working", feed.status?.state)
            assertEquals(
                "Thinking",
                StatusText.header(feed.status, feed.connected, feed.statusFresh, "project", feed.refused),
            )
        } finally {
            running.cancel()
        }
    }

    /**
     * An answer the router refuses is said in its words, from a current router's error
     * status and from an older one's 200 with `ok: false` alike. That 200 once became
     * "Router returned 503", a status nothing had sent. No message is sent in its place here
     * (there is nothing picked to send), so the refusal is what stays on screen.
     */
    @Test
    fun `the live feed never names a status the router did not send`() = runBlocking {
        val feed = LiveFeed(LiveSession(sessionId = "s-1", wrapped = true), client)
        for ((status, words) in listOf(
            200 to "That session isn't waiting on anything right now.",
            409 to "That prompt is no longer waiting",
        )) {
            router.answer("/api/live/answer", status, """{"ok":false,"reason":"$words"}""")
            val sent = feed.answer(listOf("Which?"), mapOf("Which?" to "B"), fallback = emptyList(), rowId = "row")
            assertFalse("$status: not answered", sent)
            assertEquals("$status", words, feed.notice?.text)
            assertEquals(false, feed.notice?.ok)
        }
    }

    private suspend fun waitFor(what: String, withinMs: Long = 8_000, done: () -> Boolean) {
        val deadline = System.nanoTime() + withinMs * 1_000_000
        while (!done()) {
            if (System.nanoTime() > deadline) fail("gave up waiting for $what")
            delay(20)
        }
    }
}
