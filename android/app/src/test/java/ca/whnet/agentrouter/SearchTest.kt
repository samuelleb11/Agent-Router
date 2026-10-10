package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.SearchReply
import ca.whnet.agentrouter.api.routerJson
import ca.whnet.agentrouter.data.Snippet
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SearchTest {
    /** The reply as the router spells it: counts are whole, rank is a real. */
    @Test
    fun `the search reply decodes`() {
        val json = """
        {"query":"body_cap","expression":"body_cap",
         "sessions":[{"session_id":"2b3c4d5e-0000-4000-8000-00000000000b","project":"/code/agent-router",
                      "session_label":null,"git_branch":"dev","started_at":1790000000000,
                      "blocks":3,"hits":4,"first_at":1790000000000,"last_at":1790000009000}],
         "hits":[{"block_id":41,"kind":"tool_use","chars":120,
                  "session_id":"2b3c4d5e-0000-4000-8000-00000000000b","role":"assistant","ordinal":7,
                  "request_id":48761,"at":1790000009000,"project":"/code/agent-router",
                  "session_label":null,"snippet":"why is ‹body_cap› 64 KB","rank":-8.04}]}
        """.trimIndent()
        val r = routerJson.decodeFromString<SearchReply>(json)

        assertEquals("2b3c4d5e", r.sessions[0].shortId)
        assertEquals("agent-router", r.sessions[0].projectName)
        assertEquals(4L, r.sessions[0].hits)

        val hit = r.hits[0]
        assertEquals("tool call", hit.kindName)
        // A block appears at every place it was replayed, so the block alone is not an
        // identity — and a LazyColumn given a duplicate key crashes rather than coping.
        assertEquals("2b3c4d5e-0000-4000-8000-00000000000b-41-7", hit.key)
        assertEquals(-8.04, hit.rank, 0.001)
    }

    /** A nullable project must not fail the reply: the join behind it is a LEFT JOIN. */
    @Test
    fun `a hit whose project is null still decodes`() {
        val json = """{"query":"x","expression":"x","sessions":[],"hits":[
            {"block_id":1,"kind":"text","chars":3,"session_id":"s","role":"user","ordinal":0,
             "request_id":2,"at":1790000000000,"project":null,"session_label":null,
             "snippet":"x","rank":-1.0}]}"""
        val r = routerJson.decodeFromString<SearchReply>(json)
        assertEquals("—", r.hits[0].projectName)
        assertTrue(r.sessions.isEmpty())
    }

    @Test
    fun `a snippet splits into plain and matched runs`() {
        val runs = Snippet.parse("why is ‹body_cap› 64 KB")
        assertEquals(listOf("why is ", "body_cap", " 64 KB"), runs.map { it.text })
        assertEquals(listOf(false, true, false), runs.map { it.match })
    }

    /**
     * A marker the conversation's own text happened to contain can only shift which run
     * is emphasised. It must never drop text or run off the end looking for a close.
     */
    @Test
    fun `a stray marker cannot swallow the rest`() {
        val runs = Snippet.parse("a ‹b› c ‹d")
        assertEquals("a b c d", runs.joinToString("") { it.text })
        assertEquals(listOf(false, true, false, true), runs.map { it.match })
        assertEquals(emptyList<Snippet.Run>(), Snippet.parse(""))
    }
}
