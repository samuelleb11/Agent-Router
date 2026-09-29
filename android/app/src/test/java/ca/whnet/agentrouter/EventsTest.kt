package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.RouterClient
import ca.whnet.agentrouter.api.RouterFailure
import ca.whnet.agentrouter.api.SseEvent
import ca.whnet.agentrouter.feed.LiveFeed
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread

/**
 * The event stream against a stand-in router on a loopback port: frames in, a refusal
 * surfaced, and — the reason this exists — a collector that stops really closing the
 * connection, even one that has gone silent. The feed gives up on a stream that says
 * nothing for ten seconds; if giving up left the socket blocked in a read, the retry would
 * wait out the 45 s read timeout behind it, which is the frozen header all over again.
 */
class EventsTest {

    /** Just enough HTTP/1.1 to answer one stream request per connection. */
    private class StandIn : AutoCloseable {
        private val listener = ServerSocket(0, 8, InetAddress.getLoopbackAddress())
        val base = "http://127.0.0.1:${listener.localPort}"

        /** Set by the test once its collector has stopped. */
        val gaveUp = CountDownLatch(1)
        /** Counted down once the quiet stream has checked whether anyone is still there. */
        val probed = CountDownLatch(1)
        @Volatile var peerClosed = false
        /** Connections made to the stream that falls silent after one status. */
        val silentConnections = AtomicInteger()
        /** Connections made to the stream that backfills two rows and then falls silent. */
        val backfillConnections = AtomicInteger()

        init {
            thread(isDaemon = true, name = "stand-in router") {
                while (!listener.isClosed) {
                    val s = try { listener.accept() } catch (_: IOException) { break }
                    thread(isDaemon = true) { serve(s) }
                }
            }
        }

        private fun serve(s: Socket): Unit = s.use {
            val input = s.getInputStream().bufferedReader()
            val path = input.readLine()?.split(' ')?.getOrNull(1) ?: return
            while (true) { if (input.readLine().isNullOrEmpty()) break }
            val out = s.getOutputStream()
            fun write(text: String) { out.write(text.toByteArray(Charsets.UTF_8)); out.flush() }
            val stream = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n"
            when {
                "session=denied" in path ->
                    write("HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                "session=frames" in path -> {
                    write(stream)
                    write("data: {\"a\":1}\n\nevent: status\ndata: {}\n\n: keep-alive\n\ndata: clipped… héllo\n\n")
                }
                "session=quiet" in path -> {
                    write(stream)
                    write("event: status\ndata: {\"state\":\"working\"}\n\n")
                    // Then nothing, like a link that died without a word — until the client
                    // has given up. Only then is it asked whether anyone is still listening:
                    // writing earlier would wake the very read being tested.
                    gaveUp.await(90, TimeUnit.SECONDS)
                    try {
                        repeat(40) { write(": anyone?\n\n"); Thread.sleep(50) }
                    } catch (_: IOException) {
                        peerClosed = true
                    }
                    probed.countDown()
                }
                "session=silent" in path -> {
                    silentConnections.incrementAndGet()
                    write(stream)
                    write("event: status\ndata: {\"state\":\"working\",\"detail\":\"Working\",\"quiet_ms\":0}\n\n")
                    // Nothing more, ever: held open until the client hangs up.
                    try {
                        while (input.read() >= 0) { /* a GET sends nothing more: this ends at hang-up */ }
                    } catch (_: IOException) { }
                }
                "session=backfill" in path -> {
                    backfillConnections.incrementAndGet()
                    write(stream)
                    // What every connection opens with: the transcript so far, then a status.
                    write("data: {\"role\":\"user\",\"kind\":\"text\",\"text\":\"hello\"}\n\n")
                    write("data: {\"role\":\"assistant\",\"kind\":\"text\",\"text\":\"hi\"}\n\n")
                    write("event: status\ndata: {\"state\":\"idle\",\"detail\":\"Waiting for you\",\"quiet_ms\":0}\n\n")
                    try {
                        while (input.read() >= 0) { /* silent until the client hangs up */ }
                    } catch (_: IOException) { }
                }
            }
        }

        override fun close() = listener.close()
    }

    private val router = StandIn()

    @After fun stop() = router.close()

    @Test
    fun `frames arrive in order and the flow ends with the router's stream`() = runBlocking {
        val events = RouterClient(router.base, null).events("/api/live/stream?session=frames").toList()
        assertEquals(
            listOf(SseEvent(null, """{"a":1}"""), SseEvent("status", "{}"), SseEvent(null, "clipped… héllo")),
            events,
        )
    }

    @Test
    fun `a refused password is said as such, so the feed stops retrying`() = runBlocking {
        try {
            RouterClient(router.base, null).events("/api/live/stream?session=denied").toList()
            fail("a 401 must not look like an empty stream")
        } catch (e: RouterFailure) {
            assertTrue("$e", e is RouterFailure.Unauthorized)
        }
    }

    @Test
    fun `a collector that stops closes the connection at once, even a silent one`() = runBlocking {
        val seen = mutableListOf<SseEvent>()
        val started = System.nanoTime()
        val finished = withTimeoutOrNull(1_000) {
            RouterClient(router.base, null).events("/api/live/stream?session=quiet").collect { seen.add(it) }
        }
        val tookMs = (System.nanoTime() - started) / 1_000_000
        router.gaveUp.countDown()
        assertNull("the stream never ends on its own; the timeout ended it", finished)
        assertEquals(listOf(SseEvent("status", """{"state":"working"}""")), seen)
        assertTrue("stopping took $tookMs ms: the read was left blocked in the socket", tookMs < 5_000)
        assertTrue(router.probed.await(10, TimeUnit.SECONDS))
        assertTrue("the connection was abandoned, not closed", router.peerClosed)
    }

    /**
     * The phone that left the LAN: the stream simply stops. The feed must stop vouching for
     * the last status — the header then says it is reconnecting, and offers no Stop — keep
     * that status rather than forget what it knew, and open a fresh connection instead of
     * waiting out the dead one.
     */
    @Test
    fun `a feed whose stream falls silent stops vouching for its status and reconnects`() = runBlocking {
        val feed = LiveFeed(LiveSession(sessionId = "silent"), RouterClient(router.base, null))
        feed.silentMs = 300
        val running = launch { feed.run() }
        try {
            waitFor("the first status") { feed.statusFresh }
            assertEquals("working", feed.status?.state)
            waitFor("the silence to be noticed") { !feed.connected }
            assertFalse(feed.statusFresh)
            assertEquals("kept, for the header to call it old", "working", feed.status?.state)
            waitFor("a fresh connection") { router.silentConnections.get() >= 2 }
            waitFor("a status on the new connection") { feed.statusFresh }
        } finally {
            running.cancel()
        }
    }

    /**
     * Every connection starts with the whole backfill, so a reconnect after a silence must
     * rebuild the feed, not add to it — or every row the reader had would appear twice.
     */
    @Test
    fun `a reconnect after a silence does not append the backfill twice`() = runBlocking {
        val feed = LiveFeed(LiveSession(sessionId = "backfill"), RouterClient(router.base, null))
        feed.silentMs = 300
        val running = launch { feed.run() }
        try {
            waitFor("the first backfill") { feed.statusFresh }
            assertEquals(listOf("hello", "hi"), feed.rows.map { it.item.text })
            waitFor("the silence to be noticed") { !feed.connected }
            waitFor("a fresh connection") { router.backfillConnections.get() >= 2 }
            waitFor("a status on the new connection") { feed.statusFresh }
            assertEquals("rebuilt, not doubled", listOf("hello", "hi"), feed.rows.map { it.item.text })
        } finally {
            running.cancel()
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
