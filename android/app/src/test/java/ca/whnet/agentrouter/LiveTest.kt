package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.AnswerBody
import ca.whnet.agentrouter.api.AskQuestion
import ca.whnet.agentrouter.api.BackgroundWork
import ca.whnet.agentrouter.api.LiveItem
import ca.whnet.agentrouter.api.LivePrompt
import ca.whnet.agentrouter.api.LiveReply
import ca.whnet.agentrouter.api.LiveStatus
import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.SessionStats
import ca.whnet.agentrouter.api.SseEvent
import ca.whnet.agentrouter.api.SseLineSplitter
import ca.whnet.agentrouter.api.SseParser
import ca.whnet.agentrouter.api.routerJson
import ca.whnet.agentrouter.data.AskAnswers
import ca.whnet.agentrouter.data.DotLook
import ca.whnet.agentrouter.data.Markdown
import ca.whnet.agentrouter.data.Markdown.Block
import ca.whnet.agentrouter.data.SessionCounts
import ca.whnet.agentrouter.data.SessionGroups
import ca.whnet.agentrouter.data.StateColor
import ca.whnet.agentrouter.data.color
import ca.whnet.agentrouter.feed.LiveFeed
import ca.whnet.agentrouter.feed.LiveFeed.QuestionRoute
import ca.whnet.agentrouter.ui.Fmt
import ca.whnet.agentrouter.ui.StatusText
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The feed must read the router's stream exactly as the web view and the iPhone do. */
class LiveTest {

    @Test
    fun `sse frames assemble and keep-alives are ignored`() {
        val p = SseParser()
        assertNull(p.feed("""data: {"a":1}"""))
        assertEquals(SseEvent(null, """{"a":1}"""), p.feed(""))
        assertNull(p.feed("event: status"))
        assertNull(p.feed("data: {}"))
        assertEquals(SseEvent("status", "{}"), p.feed(""))
        assertNull(p.feed(": keep-alive"))
        assertNull("a blank line with nothing pending delivers nothing", p.feed(""))
        assertNull(p.feed("data: a"))
        assertNull(p.feed("data: b"))
        assertEquals("a\nb", p.feed("")?.data)
        assertNull(p.feed("data:x"))
        assertEquals("the space after the colon is optional", "x", p.feed("")?.data)
        // The event name does not leak into the next frame.
        assertNull(p.feed("data: y"))
        assertNull(p.feed("")?.event)
    }

    /**
     * The regression behind an evening of a blank feed on iOS: a frame ends with an empty
     * line, and the byte splitter must deliver it rather than swallow it.
     */
    @Test
    fun `raw bytes become frames, empty lines included`() {
        val splitter = SseLineSplitter()
        val parser = SseParser()
        val events = mutableListOf<SseEvent>()
        for (b in "data: a\n\nevent: status\r\ndata: {}\r\n\r\n: ping\n\n".toByteArray()) {
            val line = splitter.feed(b.toInt() and 0xff) ?: continue
            parser.feed(line)?.let { events.add(it) }
        }
        assertEquals(listOf(SseEvent(null, "a"), SseEvent("status", "{}")), events)
    }

    /**
     * Found by reading a real feed on the phone: the router clips long text and appends an
     * ellipsis, which arrived as "â¦" because the splitter was building a String one byte
     * at a time. A line is bytes until it is whole.
     */
    @Test
    fun `multi-byte characters survive the splitter`() {
        val splitter = SseLineSplitter()
        val parser = SseParser()
        val events = mutableListOf<SseEvent>()
        val text = "data: clipped…  héllo 🌍\n\n"
        for (b in text.toByteArray(Charsets.UTF_8)) {
            val line = splitter.feed(b.toInt() and 0xff) ?: continue
            parser.feed(line)?.let { events.add(it) }
        }
        assertEquals(listOf(SseEvent(null, "clipped…  héllo 🌍")), events)
    }

    @Test
    fun `stream items decode`() {
        val it: LiveItem = routerJson.decodeFromString(
            """{"t":"2026-09-12T08:30:10.762Z","role":"assistant","kind":"tool_use","name":"Bash","text":"sed -n 1,5p x","dur_ms":1200,"sidechain":false}"""
        )
        assertEquals("tool_use", it.kind)
        assertEquals("Bash", it.name)
        assertEquals(1200L, it.durMs)

        val a: LiveItem = routerJson.decodeFromString(
            """{"role":"assistant","kind":"ask","sidechain":false,"ask":{"questions":[{"question":"Which one?","header":"Pick","multiSelect":false,"options":[{"label":"A","description":"first"},{"label":"B"}]}]}}"""
        )
        val q = a.ask!!.questions.first()
        assertEquals("Pick", q.title)
        assertEquals(listOf("A", "B"), q.options?.map { o -> o.label })
        assertFalse(q.isMulti)

        val st: LiveStatus = routerJson.decodeFromString("""{"state":"tool","detail":"in Bash","quiet_ms":5200}""")
        assertTrue(st.isBusy)
        assertEquals(5200L, st.quietMs)
        assertNull("an older router says nothing about the queue", st.queued)

        val queued: LiveStatus = routerJson.decodeFromString("""{"state":"working","detail":"Working","quiet_ms":10,"queued":2}""")
        assertEquals(2, queued.queued)

        val reply: LiveReply = routerJson.decodeFromString("""{"ok":true,"via":"peer","pid":12,"msg_id":"m","framed":true}""")
        assertEquals(true, reply.framed)
        val refused: LiveReply = routerJson.decodeFromString("""{"ok":false,"reason":"No such session."}""")
        assertEquals("No such session.", refused.reason)

        val stats: SessionStats = routerJson.decodeFromString(
            """{"model":"claude-opus-5","effort":"high","input":10,"output":5,"cache_read":100,"cache_write":3,"cache_write_5m":3,"cache_write_1h":0,"cost":0.42,"total":118,"turns":2}"""
        )
        assertEquals(118L, stats.total)
    }

    private fun session(
        id: String, project: String, agoHours: Double, active: Boolean = false,
        state: String = "idle", archived: Boolean = false, now: Long,
    ): LiveSession = routerJson.decodeFromString(
        """{"session_id":"$id","cwd":"/code/$project","project":"-code-$project","git_branch":null,"title":null,
            "created_ms":0,"last_activity_ms":${now - (agoHours * 3600 * 1000).toLong()},
            "active":$active,"state":"$state","archived":$archived,"wrapped":false,"bytes":10}"""
    )

    @Test
    fun `sessions group like the web view`() {
        val now = System.currentTimeMillis()
        val rows = listOf(
            session("aaaa", "project-b", 0.1, active = true, state = "working", now = now),
            session("bbbb", "project-a", 2.0, now = now),
            session("cccc", "project-a", 20.0, now = now),
            session("dddd", "zeta", 30.0, archived = true, now = now),
            session("eeee", "project-a", 1.0, active = true, now = now),
            // Put away a moment ago, which is the ordinary case: you archive the session you
            // have just stopped using. The router clears `archived` itself once it moves
            // again, so recency must not second-guess it here.
            session("ffff", "project-b", 0.05, archived = true, now = now),
        )
        val g = SessionGroups.build(rows, now)
        assertEquals("projects alphabetical, not by activity", listOf("project-a", "project-b"), g.recent.map { it.name })
        assertEquals("newest first within a project", listOf("eeee", "bbbb"), g.recent[0].sessions.map { it.shortId })
        assertEquals(listOf("cccc"), g.older.flatMap { p -> p.sessions.map { it.shortId } })
        assertEquals(
            "on the shelf however recently it moved, newest first",
            listOf("ffff", "dddd"), g.archived.map { it.shortId },
        )
        assertFalse(
            "and not also on screen",
            g.recent.flatMap { it.sessions }.any { it.shortId == "ffff" },
        )
        assertTrue(g.recent[1].live)
        assertEquals("project-b", rows[0].projectName)
        assertEquals("aaaa", rows[0].label)
    }

    @Test
    fun `answers compose the way the person would have typed`() {
        val qs: List<AskQuestion> = routerJson.decodeFromString(
            """[{"question":"Colour?","header":"Colour","multiSelect":false,"options":[{"label":"Red"},{"label":"Blue"}]},
                {"question":"Toppings?","header":"Toppings","multiSelect":true,"options":[{"label":"Olives"},{"label":"Basil"},{"label":"Chili"}]},
                {"question":"Name?","options":[]}]"""
        )
        val sel = mapOf(
            "Colour?" to AskAnswers.Selection(chosen = setOf("Blue")),
            "Toppings?" to AskAnswers.Selection(chosen = setOf("Chili", "Olives"), other = "extra cheese"),
        )
        val (answers, parts) = AskAnswers.compose(qs, sel)
        assertEquals("Blue", answers["Colour?"])
        assertEquals(
            "the tool's option order, then what was typed",
            "Olives, Chili, extra cheese", answers["Toppings?"],
        )
        assertNull("an unanswered question is left out", answers["Name?"])
        assertEquals(listOf("Colour: Blue", "Toppings: Olives, Chili, extra cheese"), parts)

        // Typing Other alone is an answer; choosing nothing is not.
        assertEquals(
            "Sam",
            AskAnswers.compose(qs, mapOf("Name?" to AskAnswers.Selection(other = "  Sam "))).answers["Name?"],
        )
        assertTrue(AskAnswers.compose(qs, mapOf("Name?" to AskAnswers.Selection())).parts.isEmpty())
    }

    /**
     * The web view's blocks, `#` at the level it draws (SE-C98), each line a paragraph of its own
     * (SE-C103). fixtures/live/markdown-blocks.json holds the rest (SharedFixtureTest); a numbered
     * list is a list of its own and stays numbered, on every client since 2026-10-05 (SE-C99).
     */
    @Test
    fun `markdown blocks match the web view`() {
        val src = "# Title\nSome **bold** text\n\n- one\n- two\n\n1. first\n2) second\n\n> quoted\n\n" +
            "| a | b |\n|---|---|\n| 1 | 2 |\n\n```\ncode\n```\n| not | a table |"
        assertEquals(
            listOf(
                Block.Heading(3, "Title"),
                Block.Paragraph("Some **bold** text"),
                Block.Bullets(listOf("one", "two")),
                Block.Numbered(listOf("first", "second")),
                Block.Quote("quoted"),
                Block.Table(listOf(listOf("a", "b"), listOf("1", "2"))),
                Block.Code("code"),
                Block.Paragraph("| not | a table |"),
            ),
            Markdown.blocks(src),
        )
        assertEquals(listOf(Block.Paragraph("plain"), Block.Paragraph("lines")), Markdown.blocks("plain\nlines"))
    }
}

/**
 * What a router that watches the Claude Code process adds to the list, the stream and the
 * prompt — and that a router which does not still reads exactly as before. Every new key is
 * optional, so both shapes are decoded here, side by side.
 */
class SessionActivityTest {

    private val id = "5e6f7081-0000-4000-8000-00000000000e"

    private fun status(json: String): LiveStatus = routerJson.decodeFromString(json)

    private fun row(json: String): LiveSession = routerJson.decodeFromString(json)

    /** A row with only the keys every router has always sent. */
    private fun legacyRow(state: String = "working", active: Boolean = true, lastActivityMs: Long = 1_790_000_000_000) = row(
        """{"session_id":"aaaa1111","cwd":"/code/x","project":"-code-x","git_branch":null,"title":null,
            "created_ms":0,"last_activity_ms":$lastActivityMs,"active":$active,"state":"$state",
            "archived":false,"wrapped":false,"bytes":10}"""
    )

    @Test
    fun `a row from a router that reads the process decodes every new key`() {
        val s = row(
            """{"session_id":"$id","cwd":"/code/agent-router","project":"-code-agent-router","git_branch":"dev",
                "title":"Session liveness","created_ms":1790000000000,"last_activity_ms":1790000500000,
                "active":true,"state":"ask","archived":false,"wrapped":true,"bytes":123,
                "activity":"waiting","open":true,"detail":"Bash wants to run","since_ms":1790000400000,
                "waiting_on":{"kind":"permission","tool":"Bash","request_id":"req-7","tool_use_id":"toolu_01","summary":"git status"},
                "background":{"agents":1,"workflows":0,"shells":2},"tone":"attention","source":"registry",
                "as_of_ms":1790000520000}"""
        )
        assertEquals("waiting", s.activity)
        assertEquals(true, s.open)
        assertEquals("Bash wants to run", s.detail)
        assertEquals(1_790_000_400_000L, s.sinceMs)
        assertEquals("permission", s.waitingOn?.kind)
        assertEquals("Bash", s.waitingOn?.tool)
        assertEquals("req-7", s.waitingOn?.requestId)
        assertEquals("toolu_01", s.waitingOn?.toolUseId)
        assertEquals("git status", s.waitingOn?.summary)
        assertEquals(BackgroundWork(agents = 1, workflows = 0, shells = 2), s.background)
        assertEquals("attention", s.tone)
        assertEquals("registry", s.source)
        assertEquals(1_790_000_520_000L, s.asOfMs)
        assertTrue(s.needsYou)
        assertFalse(s.isClosed)
        assertEquals(StateColor.ORANGE, s.color)
        assertEquals(
            "two minutes on the router's clock, whatever the phone's says",
            "Bash wants to run · 2m 00s", StatusText.row(s, now = 0),
        )
    }

    @Test
    fun `a row from an older router reads exactly as it always did`() {
        val s = legacyRow()
        assertNull(s.activity)
        assertNull(s.open)
        assertNull(s.detail)
        assertNull(s.sinceMs)
        assertNull(s.waitingOn)
        assertNull(s.background)
        assertNull(s.tone)
        assertNull(s.asOfMs)
        assertFalse(s.isClosed)
        assertFalse("only a router that sends activity can say so", s.needsYou)
        assertNull("no words from an older router, so no extra line", StatusText.row(s))
        for ((state, colour) in listOf(
            "working" to StateColor.GREEN, "tool" to StateColor.GREEN, "ask" to StateColor.ORANGE,
            "stalled" to StateColor.YELLOW, "failed" to StateColor.RED, "idle" to StateColor.GRAY,
        )) {
            assertEquals(state, colour, legacyRow(state = state).color)
        }
        // The fewest keys a row can have still decodes: one bad row must never blank the list.
        val bare = row("""{"session_id":"bare0000"}""")
        assertEquals("idle", bare.state)
        assertEquals(StateColor.GRAY, bare.color)
    }

    @Test
    fun `a closed session is a hollow gray ring, never live and never stoppable`() {
        val s = row(
            """{"session_id":"cccc2222","project":"p","last_activity_ms":1790000000000,"active":false,"state":"idle",
                "archived":false,"wrapped":false,"bytes":0,"activity":"closed","open":false,
                "detail":"Session ended mid-turn","ended_ms":1790000000000,"ended_how":"unknown","mid_turn":true,
                "tone":"closed","source":"registry","as_of_ms":1790000060000}"""
        )
        assertTrue(s.isClosed)
        assertEquals(StateColor.GRAY, s.color)
        assertEquals(true, s.midTurn)
        assertEquals("unknown", s.endedHow)
        assertEquals(1_790_000_000_000L, s.endedMs)
        assertEquals(
            "how long ago it ended, on the router's clock",
            "Session ended mid-turn · 1m 00s ago", StatusText.row(s, now = 0),
        )
        // Any one of the three is enough to know it.
        assertTrue(LiveSession(sessionId = "x", open = false).isClosed)
        assertTrue(LiveSession(sessionId = "x", tone = "closed").isClosed)
        assertTrue(LiveSession(sessionId = "x", activity = "closed").isClosed)
        assertFalse("not knowing is not closed", LiveSession(sessionId = "x", open = null).isClosed)

        val st = status(
            """{"state":"idle","detail":"Session ended","quiet_ms":0,"activity":"closed","open":false,
                "tone":"closed","ended_ms":1000,"as_of_ms":3601000,"queued":0}"""
        )
        assertTrue(st.isClosed)
        assertFalse(st.isBusy)
        assertFalse(st.isActive)
        assertEquals("Session ended · 1h 00m ago", StatusText.line(st, now = 0))
    }

    /**
     * Background work is `working` in the legacy state — which is why older builds still show
     * Stop there, and why the router refuses that Stop — but it is not something to stop.
     */
    @Test
    fun `stop follows the activity when there is one, and the old state otherwise`() {
        val bg = status(
            """{"state":"working","detail":"Background workflow running","quiet_ms":9000,"activity":"background",
                "tone":"busy","background":{"agents":0,"workflows":1,"shells":0},"open":true,
                "since_ms":1,"as_of_ms":9001,"queued":0}"""
        )
        assertFalse(bg.isBusy)
        assertTrue("still live, so the dot keeps its halo", bg.isActive)
        assertEquals(StateColor.GREEN, bg.color)
        assertEquals(1, bg.background?.workflows)

        for ((activity, state) in listOf("generating" to "working", "tool" to "tool", "stalled" to "working")) {
            assertTrue(activity, status("""{"state":"$state","detail":"","quiet_ms":0,"activity":"$activity"}""").isBusy)
        }
        for ((activity, state) in listOf(
            "waiting" to "ask", "idle" to "idle", "closed" to "idle", "failed" to "failed",
            "background" to "working", "compacting" to "working",
        )) {
            assertFalse(activity, status("""{"state":"$state","detail":"","quiet_ms":0,"activity":"$activity"}""").isBusy)
        }

        // An older router: the state, exactly as before, and no halo the header never had.
        assertTrue(status("""{"state":"working","detail":"Working","quiet_ms":10}""").isBusy)
        assertTrue(status("""{"state":"tool","detail":"in Bash","quiet_ms":10}""").isBusy)
        assertFalse(status("""{"state":"ask","detail":"","quiet_ms":10}""").isBusy)
        assertFalse(status("""{"state":"idle","detail":"Waiting for you","quiet_ms":10}""").isBusy)
        assertFalse(status("""{"state":"working","detail":"Working","quiet_ms":10}""").isActive)
    }

    @Test
    fun `an activity this app does not know is neutral, never a crash and never a Stop`() {
        val s = row("""{"session_id":"x","state":"working","active":true,"activity":"compacting"}""")
        assertEquals("not the legacy state's green", StateColor.GRAY, s.color)
        assertNull("not one of the kinds worth a line", StatusText.row(s))
        val st = status("""{"state":"working","detail":"Compacting","quiet_ms":1,"activity":"compacting","tone":"sparkle"}""")
        assertEquals(StateColor.GRAY, st.color)
        assertFalse(st.isBusy)
        assertEquals("a tone this app does not know defers to the activity", StateColor.YELLOW, StateColor.of("sparkle", "stalled", "stalled"))
    }

    @Test
    fun `the router's tone decides the colour, then the activity, then the old state`() {
        assertEquals(StateColor.GREEN, StateColor.of("busy", "background", "working"))
        assertEquals(StateColor.ORANGE, StateColor.of("attention", "waiting", "ask"))
        assertEquals(StateColor.YELLOW, StateColor.of("warn", "stalled", "stalled"))
        assertEquals(StateColor.RED, StateColor.of("error", "failed", "failed"))
        assertEquals(StateColor.GRAY, StateColor.of("idle", "idle", "idle"))
        assertEquals(StateColor.GRAY, StateColor.of("closed", "closed", "idle"))
        assertEquals(StateColor.ORANGE, StateColor.of(null, "waiting", "ask"))
        assertEquals(StateColor.GREEN, StateColor.of(null, "generating", "working"))
        assertEquals(StateColor.GREEN, StateColor.of(null, null, "working"))
        assertEquals(StateColor.PURPLE, StateColor.of(null, null, "thinking"))
    }

    /**
     * A session blocked on a question writes nothing while it waits. Going by last activity
     * alone folded one that had waited twelve hours under "older", on all three clients.
     */
    @Test
    fun `a session waiting on you stays recent however long it has waited`() {
        val now = 1_790_000_000_000L
        val longAgo = now - 30L * 3600 * 1000
        val waiting = legacyRow(state = "ask", lastActivityMs = longAgo)
            .copy(sessionId = "wait0000", activity = "waiting", tone = "attention")
        val quiet = legacyRow(state = "idle", active = false, lastActivityMs = longAgo).copy(sessionId = "quie0000")
        val g = SessionGroups.build(listOf(waiting, quiet), now)
        assertEquals(listOf("wait0000"), g.recent.flatMap { p -> p.sessions.map { it.shortId } })
        assertEquals(listOf("quie0000"), g.older.flatMap { p -> p.sessions.map { it.shortId } })
        assertEquals(DotLook(StateColor.ORANGE, active = true), g.recent.single().dot)
        assertTrue(SessionGroups.isRecent(waiting, now))
        assertFalse(SessionGroups.isRecent(quiet, now))
    }

    /**
     * One ranking on all three clients — waiting on you, then failed, then stalled, then at
     * work, then idle — and the empty ring only once every session has ended. They had
     * drifted three ways: a project whose sessions had all ended drew a hollow ring on the
     * web, a gray dot on the iPhone and nothing here, and a failed session never reached
     * the heading at all.
     */
    @Test
    fun `a project's heading is ranked the same on every client`() {
        // Shaped as the router sends them: legacy state and `active` beside the new keys.
        fun at(activity: String, tone: String, state: String, active: Boolean) =
            legacyRow(state = state, active = active).copy(activity = activity, tone = tone, open = activity != "closed")
        val generating = at("generating", "busy", "working", true)
        val background = at("background", "busy", "working", true)
        val stalled = at("stalled", "warn", "stalled", true)
        val waiting = at("waiting", "attention", "ask", true)
        val failed = at("failed", "error", "failed", false)
        val idle = at("idle", "idle", "idle", false)
        val closed = at("closed", "closed", "idle", false)
        fun dot(vararg s: LiveSession) = SessionGroups.Project("p", s.toList()).dot
        val live = { c: StateColor -> DotLook(c, active = true) }

        assertEquals(live(StateColor.GREEN), dot(generating, background))
        assertEquals("stalled outranks at work", live(StateColor.YELLOW), dot(generating, stalled))
        assertEquals(live(StateColor.YELLOW), dot(stalled, closed))
        assertEquals("waiting on you outranks everything", live(StateColor.ORANGE), dot(stalled, waiting, failed))
        assertEquals("failed outranks stalled and at work", live(StateColor.RED), dot(generating, stalled, failed))
        assertEquals("nothing live: no halo", DotLook(StateColor.RED), dot(failed, idle, closed))
        assertEquals("idle, and still running", DotLook(StateColor.GRAY), dot(idle, closed))
        assertEquals(DotLook(StateColor.GRAY), dot(idle))
        assertEquals("every session ended: the empty ring", DotLook(StateColor.GRAY, hollow = true), dot(closed, closed))
        assertEquals(
            "an activity this app does not know is neutral, and not an ended session",
            DotLook(StateColor.GRAY), dot(legacyRow(active = false).copy(activity = "compacting"), closed),
        )
        // An older router says nothing about what its sessions do: busy while something is
        // live, as the dot always was, and no dot at all otherwise.
        assertEquals(live(StateColor.GREEN), dot(legacyRow(state = "ask")))
        assertEquals(live(StateColor.GREEN), dot(legacyRow(state = "stalled")))
        assertNull(dot(legacyRow(state = "idle", active = false)))
    }

    /** RC9: the permission card never appeared because this URL was sent as literal text. */
    @Test
    fun `the prompt is asked for by the session's real id`() {
        assertEquals("/api/live/prompt?session=$id", LiveFeed.path(LiveFeed.PROMPT, id))
        assertEquals("/api/live/stream?session=$id", LiveFeed.path(LiveFeed.STREAM, id))
        assertEquals("/api/live/stats?session=$id", LiveFeed.path(LiveFeed.STATS, id))
        assertFalse(LiveFeed.path(LiveFeed.PROMPT, id).contains('$'))
        assertFalse(LiveFeed.path(LiveFeed.PROMPT, id).contains('{'))
    }

    /**
     * `${'$'}` in a Kotlin string is a literal dollar sign, so `"…${'$'}{x}"` is the text `${x}`,
     * not x's value. That slip had the phone asking for the prompt of a session named
     * `${session.sessionId}` — the router said, truthfully, that nothing was waiting there —
     * and printed `mode: ${m.label}` in a menu. Nothing in this app means to show or send a
     * literal `${`, so finding the idiom anywhere in its source is finding the bug.
     */
    @Test
    fun `no string in the app escapes its own template`() {
        // Gradle runs unit tests from the module directory, android/app.
        val src = File("src")
        assertTrue("expected to run from android/app: ${src.absoluteFile}", src.isDirectory)
        val idioms = listOf("\${'\$'}{", "\${\"\$\"}{")
        val testDir = "${File.separator}test${File.separator}"
        val found = src.walkTopDown()
            .filter { it.isFile && it.extension == "kt" && testDir !in it.path }
            .flatMap { f ->
                f.readLines().withIndex()
                    .filter { (_, line) -> idioms.any { it in line } }
                    .map { (i, _) -> "${f.path}:${i + 1}" }
            }
            .toList()
        assertTrue("looked at nothing", src.walkTopDown().any { it.name == "LiveFeed.kt" })
        assertEquals(emptyList<String>(), found)
    }

    @Test
    fun `an answer names its prompt only when it knows it`() {
        assertEquals(
            """{"session":"s","answers":{"Colour?":"Blue"},"request_id":"req-3"}""",
            routerJson.encodeToString(AnswerBody("s", mapOf("Colour?" to "Blue"), requestId = "req-3")),
        )
        assertEquals(
            "unnamed, the body is byte for byte the one this app always sent",
            """{"session":"s","answers":{"Colour?":"Blue"}}""",
            routerJson.encodeToString(AnswerBody("s", mapOf("Colour?" to "Blue"))),
        )
        assertEquals(
            """{"session":"s","allow":true,"suggestion":0,"request_id":"req-4"}""",
            routerJson.encodeToString(AnswerBody("s", allow = true, suggestion = 0, requestId = "req-4")),
        )
    }

    @Test
    fun `the prompt's count is optional`() {
        val p: LivePrompt = routerJson.decodeFromString(
            """{"pending":true,"request_id":"r1","tool":"Bash","question":false,"display_name":"Run git",
                "description":"git status","reason":null,"input":{"command":"git status"},
                "suggestions":[{"index":0,"label":"Always allow Bash(git status:*) in this project"}],"count":3}"""
        )
        assertEquals(3, p.count)
        assertTrue(p.isPermission)
        assertFalse(p.isQuestion)
        assertEquals("Always allow Bash(git status:*) in this project", p.suggestions.single().label)

        val older: LivePrompt = routerJson.decodeFromString(
            """{"pending":true,"request_id":"q1","tool":"AskUserQuestion","question":true,"suggestions":[]}"""
        )
        assertNull(older.count)
        assertTrue(older.isQuestion)
        assertFalse(older.isPermission)

        val none: LivePrompt = routerJson.decodeFromString("""{"pending":false}""")
        assertFalse(none.isPermission)
        assertFalse(none.isQuestion)
    }

    /**
     * The router puts a question's answers into the tool's own input, and approves any other
     * tool with its input untouched — so answers that reached a Bash approval by mistake
     * would run the command. They must name the question, or wait until they can.
     */
    @Test
    fun `a question's answers never land on a permission prompt`() {
        val question = LivePrompt(pending = true, requestId = "q1", tool = "AskUserQuestion", question = true, count = 1)
        val earlier = question.copy(requestId = "q0")
        val permission = LivePrompt(pending = true, requestId = "p1", tool = "Bash", question = false, count = 2)
        val olderRouters = permission.copy(count = null)
        val nothing = LivePrompt(pending = false)
        // These prompts list no questions, so each is taken at its word, whatever the form.
        fun route(fresh: LivePrompt?, remembered: LivePrompt?) =
            LiveFeed.routeForQuestion(fresh, remembered, asked = listOf("Colour?"))

        assertEquals(QuestionRoute.Named("q1"), route(question, null))
        assertEquals(QuestionRoute.Named("q1"), route(question, earlier))
        assertEquals("under a newer permission prompt", QuestionRoute.Named("q0"), route(permission, earlier))
        assertEquals(
            "an older router answers the newest prompt whatever it is told",
            QuestionRoute.NotYet, route(olderRouters, earlier),
        )
        assertEquals(QuestionRoute.NotYet, route(permission, null))
        assertEquals(QuestionRoute.NotYet, route(olderRouters, null))
        assertEquals("gone, and the router will say so", QuestionRoute.Named("q0"), route(nothing, earlier))
        assertEquals(QuestionRoute.Unnamed, route(nothing, null))
        assertEquals("the router could not be asked", QuestionRoute.Unnamed, route(null, null))
        assertEquals(QuestionRoute.Named("q0"), route(null, earlier))
    }

    /**
     * A form stays live in the feed until this phone answers it, so one answered in the
     * editor is still there when the model asks its next question. Named onto that next one,
     * its answers would be written into it and approve it — dismissing a question nobody had
     * read. The web view refuses that submit; so must this app: a form names only the
     * question it was drawn for, the same questions in the same order.
     */
    @Test
    fun `a form's answers go only to the question it was drawn for`() {
        fun question(id: String, vararg texts: String): LivePrompt = routerJson.decodeFromString(
            """{"pending":true,"request_id":"$id","tool":"AskUserQuestion","question":true,"suggestions":[],"count":1,
                "input":{"questions":[${texts.joinToString(",") { """{"question":"$it","header":"H","multiSelect":false,"options":[{"label":"A"}]}""" }}]}}"""
        )
        val framework = question("req-Q1", "Which framework?")
        val database = question("req-Q2", "Which database?")
        val pair = question("req-Q3", "Colour?", "Size?")
        val permission = LivePrompt(pending = true, requestId = "p1", tool = "Bash", question = false, count = 2)
        val nothing = LivePrompt(pending = false)
        fun route(fresh: LivePrompt?, remembered: LivePrompt?, vararg asked: String) =
            LiveFeed.routeForQuestion(fresh, remembered, asked.toList())

        assertEquals(listOf("Which database?"), database.questionTexts)
        assertEquals(
            "the review's case: Q1's form while Q2 waits",
            QuestionRoute.Different, route(database, null, "Which framework?"),
        )
        assertEquals(QuestionRoute.Different, route(database, framework, "Which framework?"))
        assertEquals(QuestionRoute.Named("req-Q2"), route(database, null, "Which database?"))
        assertEquals(QuestionRoute.Named("req-Q3"), route(pair, null, "Colour?", "Size?"))
        assertEquals("the same questions, in the same order", QuestionRoute.Different, route(pair, null, "Size?", "Colour?"))
        assertEquals(QuestionRoute.Different, route(pair, null, "Colour?"))
        // Under a newer permission request, a remembered question is named only if it is this form's.
        assertEquals(QuestionRoute.Named("req-Q1"), route(permission, framework, "Which framework?"))
        assertEquals(
            "the question beneath is another: answer the permission request, then compare again",
            QuestionRoute.NotYet, route(permission, database, "Which framework?"),
        )
        // Nothing waiting: the router will say so, but never by naming another question.
        assertEquals(QuestionRoute.Named("req-Q1"), route(nothing, framework, "Which framework?"))
        assertEquals(QuestionRoute.Unnamed, route(nothing, database, "Which framework?"))
        // The router could not be asked: the last question seen waiting decides.
        assertEquals(QuestionRoute.Named("req-Q1"), route(null, framework, "Which framework?"))
        assertEquals(QuestionRoute.Different, route(null, database, "Which framework?"))
    }

    @Test
    fun `a prompt's input is read only for its questions, and never fails the prompt`() {
        val bash: LivePrompt = routerJson.decodeFromString(
            """{"pending":true,"request_id":"r1","tool":"Bash","question":false,"input":{"command":"ls","questions":"not a list"},"count":1}"""
        )
        assertTrue("a tool's own input, whatever its shape, still decodes", bash.isPermission)
        assertNull(bash.questionTexts)
        val odd: LivePrompt = routerJson.decodeFromString(
            """{"pending":true,"request_id":"q9","tool":"AskUserQuestion","question":true,"input":{"questions":[{"header":"no text"},7]}}"""
        )
        assertEquals(listOf(null, null), odd.questionTexts)
        assertFalse(odd.asks(listOf("Which?", "Why?")))
        val bare: LivePrompt = routerJson.decodeFromString("""{"pending":true,"request_id":"q8","tool":"AskUserQuestion","question":true}""")
        assertNull("a router that sends no input", bare.questionTexts)
        assertTrue("is taken at its word", bare.asks(listOf("Anything?")))
        val nullInput: LivePrompt = routerJson.decodeFromString("""{"pending":true,"question":true,"input":null}""")
        assertTrue(nullInput.asks(listOf("Anything?")))
    }

    @Test
    fun `the header shows a status only while the stream vouches for it`() {
        val running = status("""{"state":"tool","detail":"Running Bash","quiet_ms":12000,"activity":"tool","tone":"busy","queued":0}""")
        assertEquals("Running Bash · 12s", StatusText.header(running, connected = true, fresh = true, before = "proj"))
        assertEquals(
            "a dropped stream does not keep its last word",
            "reconnecting…", StatusText.header(running, connected = false, fresh = false, before = "proj"),
        )
        assertEquals(
            "connected, but silent past the heartbeat",
            "reconnecting…", StatusText.header(running, connected = true, fresh = false, before = "proj"),
        )
        assertEquals("connecting…", StatusText.header(null, connected = false, fresh = false, before = "proj"))
        assertEquals("proj", StatusText.header(null, connected = true, fresh = false, before = "proj"))
    }

    @Test
    fun `the header line says how long and how many are queued`() {
        // An older router's statuses, as the iPhone has always shown them.
        assertEquals("in Bash", StatusText.line(status("""{"state":"tool","detail":"in Bash","quiet_ms":1200}""")))
        assertEquals("Working · 5s", StatusText.line(status("""{"state":"working","detail":"Working","quiet_ms":5200}""")))
        assertEquals("Working · 2 queued", StatusText.line(status("""{"state":"working","detail":"Working","quiet_ms":10,"queued":2}""")))
        assertEquals("idle", StatusText.line(status("""{"state":"idle","detail":"","quiet_ms":0}""")))
        // And a newer one's.
        assertEquals(
            "Waiting for your answer · 3m 07s",
            StatusText.line(
                status(
                    """{"state":"ask","detail":"Waiting for your answer","quiet_ms":187000,"activity":"waiting",
                        "tone":"attention","since_ms":1000,"as_of_ms":188000,"open":true,"queued":0,
                        "waiting_on":{"kind":"question","tool":"AskUserQuestion","request_id":"q1","tool_use_id":"toolu_9"}}"""
                )
            ),
        )
        assertEquals(
            "No progress · 12m 00s",
            StatusText.line(status("""{"state":"working","detail":"No progress","quiet_ms":720000,"activity":"stalled","tone":"warn"}""")),
        )
    }

    /**
     * The row line policy all three clients follow: the router's words and how long, for
     * every session — idle included, since how long one has sat waiting for you is worth a
     * glance too — and for an ended one, how long ago it ended. The iPhone and this app had
     * shown nothing for idle and ended sessions while the web showed both.
     */
    @Test
    fun `every row carries the router's line, idle and ended included`() {
        val hour = 3_600_000L
        fun at(activity: String, detail: String) = legacyRow().copy(
            activity = activity, detail = detail, sinceMs = 1_000, asOfMs = 3_000,
        )
        assertEquals("under the four seconds a counter is worth", "Thinking", StatusText.row(at("generating", "Thinking"), now = 0))
        assertEquals(
            "API error: rate limited · 3h 12m",
            StatusText.row(at("failed", "API error: rate limited").copy(asOfMs = 1_000 + (3 * 60 + 12) * 60_000L), now = 0),
        )
        assertEquals("1 background agent running", StatusText.row(at("background", "1 background agent running"), now = 0))
        assertEquals(
            "Waiting for you · 1h 00m",
            StatusText.row(at("idle", "Waiting for you").copy(asOfMs = 1_000 + hour), now = 0),
        )
        assertEquals(
            "Waiting for you · background shell running · 10h 00m",
            StatusText.row(at("idle", "Waiting for you · background shell running").copy(asOfMs = 1_000 + 10 * hour), now = 0),
        )
        val ended = at("closed", "Session ended").copy(open = false, tone = "closed", endedMs = 5_000, asOfMs = 5_000 + 3 * hour)
        assertEquals("Session ended · 3h 00m ago", StatusText.row(ended, now = 0))
        assertEquals(
            "measured on the router's clock, whatever the phone's says",
            "Session ended · 3h 00m ago", StatusText.row(ended, now = 99 * hour),
        )
        assertEquals("no end known, no time", "Session ended", StatusText.row(ended.copy(endedMs = null), now = 0))
        assertEquals("no start given, no duration", "Running Bash", StatusText.row(at("tool", "Running Bash").copy(sinceMs = null)))
        assertNull("nothing said, nothing shown", StatusText.row(at("idle", "")))
        assertNull("an older router's rows look as they always did", StatusText.row(legacyRow(state = "idle", active = false)))
    }

    /**
     * The open tab shows what the header shows: the session's own stream, which is current
     * to the second, rather than its row in the list, which can be five seconds old — a
     * tab still gray while the header beside it says the turn is running.
     */
    @Test
    fun `the open tab follows its own stream, not the list row`() {
        val open = legacyRow(state = "idle", active = false).copy(sessionId = "open0000", activity = "idle", tone = "idle")
        val other = legacyRow().copy(sessionId = "othr0000", activity = "generating", tone = "busy")
        val running = status("""{"state":"working","detail":"Thinking","quiet_ms":1,"activity":"generating","tone":"busy","open":true}""")
        val ended = status("""{"state":"idle","detail":"Session ended","quiet_ms":0,"activity":"closed","tone":"closed","open":false}""")
        fun tab(s: LiveSession, st: LiveStatus?, fresh: Boolean) = DotLook.tab(s, "open0000", st, fresh)

        assertEquals("the stream, not the idle row", DotLook(StateColor.GREEN), tab(open, running, fresh = true))
        assertEquals("the same colour as the header's dot", DotLook.stream(running, true, open).color, tab(open, running, true).color)
        assertEquals(DotLook(StateColor.GRAY, hollow = true), tab(open, ended, fresh = true))
        assertEquals("a stream it cannot vouch for is neutral, as in the header", DotLook(StateColor.GRAY), tab(open, running, fresh = false))
        assertEquals("before the first status, the row", DotLook.of(open, active = false), tab(open, null, fresh = false))
        assertEquals("every other tab keeps its row", DotLook(StateColor.GREEN), tab(other, ended, fresh = true))
        // The header's own dot wears the halo; tabs never do.
        assertEquals(DotLook(StateColor.GREEN, active = true), DotLook.stream(running, true, open))
        assertEquals(DotLook(StateColor.GRAY), DotLook.stream(running, false, open))
    }

    /**
     * "4 working · 3 waiting", as the web view's header and the iPhone's router row say it:
     * "7 active" could not tell a session blocked on you from one at work. The fixture is the
     * review's own — nine sessions, one of each kind and some twice.
     */
    @Test
    fun `the router says how many sessions work and how many wait`() {
        fun at(activity: String, active: Boolean) = legacyRow(active = active).copy(activity = activity)
        val nine = listOf(
            at("generating", true), at("generating", true), at("background", true), at("stalled", true),
            at("waiting", true), at("waiting", true), at("waiting", true), at("idle", false),
            at("closed", false).copy(open = false),
        )
        assertEquals(SessionCounts.Line("4 working · 3 waiting", StateColor.ORANGE), SessionCounts.routerLine(nine))
        assertEquals("4 working · 3 waiting · 9 this week", SessionCounts.weekLine(nine))
        assertEquals(
            "at work only is green",
            SessionCounts.Line("2 working", StateColor.GREEN), SessionCounts.routerLine(nine.take(2)),
        )
        assertEquals(
            "waiting on you is the one worth colouring",
            SessionCounts.Line("1 waiting", StateColor.ORANGE), SessionCounts.routerLine(listOf(at("waiting", true), at("idle", false))),
        )
        val quiet = listOf(at("idle", false), at("closed", false), at("failed", false))
        assertNull("nothing at work or waiting: nothing to say", SessionCounts.routerLine(quiet))
        assertEquals("3 this week", SessionCounts.weekLine(quiet))
        // An older router: `active` is all it says, and the words are what they always were.
        val older = listOf(legacyRow(), legacyRow(state = "ask"), legacyRow(), legacyRow(state = "idle", active = false))
        assertEquals(SessionCounts.Line("3 active sessions", StateColor.GREEN), SessionCounts.routerLine(older))
        assertEquals(SessionCounts.Line("1 active session", StateColor.GREEN), SessionCounts.routerLine(older.take(1)))
        assertEquals("3 active · 4 this week", SessionCounts.weekLine(older))
        assertNull(SessionCounts.routerLine(older.takeLast(1)))
        assertEquals("1 this week", SessionCounts.weekLine(older.takeLast(1)))
    }

    /**
     * Ten seconds of nothing on the stream — no item, no status — and the feed stops
     * vouching for what it last heard and reconnects. The same limit on all three clients:
     * the router sends a status every second, so ten is ten missed heartbeats.
     */
    @Test
    fun `a silent stream is given up on after ten seconds, as on every client`() {
        assertEquals(10_000L, LiveFeed.SILENT_MS)
    }

    @Test
    fun `elapsed time is spelled the way the iPhone spells it`() {
        assertEquals("0s", Fmt.elapsed(-5))
        assertEquals("0s", Fmt.elapsed(0))
        assertEquals("45s", Fmt.elapsed(45_000))
        assertEquals("3m 07s", Fmt.elapsed(187_000))
        assertEquals("3h 12m", Fmt.elapsed((3 * 60 + 12) * 60_000L))
        assertEquals("1d 0h", Fmt.elapsed(24 * 3_600_000L))
    }
}

/**
 * How a feed row is classified. Also found by reading a real feed: a tool result is written
 * into the transcript as a *user* message, so dispatching on the role rendered every
 * command's output as something the person had typed.
 */
class FeedRowKindTest {

    /** The same decision the composable makes, as a value so it can be asserted on. */
    private fun classify(kind: String, role: String): String = when (kind) {
        "ask" -> "ask"
        "tool_use" -> "tool"
        "tool_result" -> "result"
        "thinking" -> "thinking"
        "answer" -> "answer"
        "interrupt", "command" -> "marker"
        else -> if (role == "user") "user" else "prose"
    }

    @Test
    fun `a tool result is not the person talking`() {
        assertEquals("result", classify("tool_result", "user"))
        assertEquals("user", classify("text", "user"))
        assertEquals("prose", classify("text", "assistant"))
        assertEquals("tool", classify("tool_use", "assistant"))
        assertEquals("thinking", classify("thinking", "assistant"))
        assertEquals("answer", classify("answer", "user"))
        assertEquals("marker", classify("interrupt", "user"))
        assertEquals("marker", classify("command", "user"))
        assertEquals("ask", classify("ask", "assistant"))
        // An unknown kind falls back on the role, which is the only thing left to go on.
        assertEquals("prose", classify("other", "system"))
    }
}

/** The styling inside a line, which the feed showed literally until this existed. */
class InlineTest {
    private fun runs(s: String) = ca.whnet.agentrouter.data.Inline.parse(s)
    private fun R(text: String, bold: Boolean = false, italic: Boolean = false, code: Boolean = false, url: String? = null) =
        ca.whnet.agentrouter.data.Inline.Run(text, bold, italic, code, url)

    @Test
    fun `bold italic code and links become runs`() {
        assertEquals(
            listOf(R("Archiving "), R("was", italic = true), R(" recording "), R("correctly", bold = true), R(" — see "), R("832daf9", code = true), R(".")),
            runs("Archiving *was* recording **correctly** — see `832daf9`."),
        )
        assertEquals(
            listOf(R("see "), R("the docs", url = "https://x.test/d"), R(" now")),
            runs("see [the docs](https://x.test/d) now"),
        )
        // A bare address is text, as on the web view (SE-C92).
        assertEquals(listOf(R("at https://x.test/a, then")), runs("at https://x.test/a, then"))
    }

    @Test
    fun `what is not markdown is left as typed`() {
        // A glob, a snake_case name, an unclosed emphasis, an unclosed backtick.
        assertEquals(listOf(R("rm -rf *.rs")), runs("rm -rf *.rs"))
        assertEquals(listOf(R("set snake_case_name here")), runs("set snake_case_name here"))
        assertEquals(listOf(R("a * b")), runs("a * b"))
        assertEquals(listOf(R("this `is not closed")), runs("this `is not closed"))
        assertEquals(listOf(R("2 ** 8")), runs("2 ** 8"))
    }

    @Test
    fun `code inside emphasis and emphasis inside links keep their styles`() {
        assertEquals(
            listOf(R("run ", bold = true), R("cargo test", bold = true, code = true), R(" first", bold = true)),
            runs("**run `cargo test` first**"),
        )
    }
}
