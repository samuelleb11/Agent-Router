package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.AskQuestion
import ca.whnet.agentrouter.api.LiveItem
import ca.whnet.agentrouter.api.LiveReply
import ca.whnet.agentrouter.api.LiveStatus
import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.SessionStats
import ca.whnet.agentrouter.api.SseEvent
import ca.whnet.agentrouter.api.SseLineSplitter
import ca.whnet.agentrouter.api.SseParser
import ca.whnet.agentrouter.api.routerJson
import ca.whnet.agentrouter.data.AskAnswers
import ca.whnet.agentrouter.data.Markdown
import ca.whnet.agentrouter.data.Markdown.Block
import ca.whnet.agentrouter.data.SessionGroups
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

    @Test
    fun `markdown blocks match the web view`() {
        val src = "# Title\nSome **bold** text\n\n- one\n- two\n\n1. first\n2) second\n\n> quoted\n\n" +
            "| a | b |\n|---|---|\n| 1 | 2 |\n\n```\ncode\n```\n| not | a table |"
        assertEquals(
            listOf(
                Block.Heading(1, "Title"),
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
        assertEquals(listOf(Block.Paragraph("plain\nlines")), Markdown.blocks("plain\nlines"))
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
        assertEquals(
            listOf(R("at "), R("https://x.test/a", url = "https://x.test/a"), R(", then")),
            runs("at https://x.test/a, then"),
        )
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
