package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.ProjectPath
import ca.whnet.agentrouter.api.ProjectRow
import ca.whnet.agentrouter.api.decodeReply
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * `GET /api/projects` groups requests by their session's working directory, and a session that
 * never recorded one has none: the reply then holds one row whose `project` is null, for all such
 * sessions. On the iPhone that one row failed the whole reply (AGEN-0027). Here [ProjectRow]
 * declares the field a string defaulting to "", and the app's Json coerces a null into that
 * default (`coerceInputValues` in `routerJson`), so the reply reads whole and the row is named as a
 * missing path is everywhere on the phones. This holds it there.
 */
class ProjectRowDecodingTest {

    /**
     * A reply as the router writes it (src/api.rs, `projects_inner`): two projects with a
     * directory, a Windows machine's among them, and between them the sessions without one.
     */
    private val reply = """
        [{"project":"/Users/sample/code/project-a","sessions":4,"requests":120,"prompt_tokens":9000000,
          "cost":30.5,"output_tokens":90000,"cache_read_tokens":8500000,"subagent_tokens":400000,
          "errors":1,"models":"claude-opus-5","first_seen":1789200000000,"last_seen":1789300000000},
         {"project":null,"sessions":3,"requests":45,"prompt_tokens":2000000,"cost":6.25,
          "output_tokens":20000,"cache_read_tokens":1900000,"subagent_tokens":0,"errors":0,
          "models":"claude-opus-5,claude-haiku-4-5","first_seen":1789100000000,"last_seen":1789290000000},
         {"project":"C:\\Users\\sample\\code\\project-b","sessions":1,"requests":8,"prompt_tokens":300000,
          "cost":0.9,"output_tokens":3000,"cache_read_tokens":280000,"subagent_tokens":0,"errors":0,
          "models":"claude-sonnet-4-5","first_seen":1789250000000,"last_seen":1789260000000}]
    """

    @Test
    fun `a reply holding a row without a directory decodes whole`() {
        // What RouterClient.get does with a body: decodeReply, through the app's own routerJson.
        val rows = decodeReply<List<ProjectRow>>(reply)
        assertEquals(3, rows.size)
        assertEquals(
            listOf("/Users/sample/code/project-a", "", """C:\Users\sample\code\project-b"""),
            rows.map { it.project },
        )
        // A row like any other: its figures arrive with it.
        assertEquals(3.0, rows[1].sessions, 0.0)
        assertEquals(45.0, rows[1].requests, 0.0)
        assertEquals(6.25, rows[1].cost, 0.0)
        assertEquals("claude-opus-5,claude-haiku-4-5", rows[1].models)
        assertEquals(1789290000000L, rows[1].lastSeen)
    }

    @Test
    fun `that row is named with the dash`() {
        val rows = decodeReply<List<ProjectRow>>(reply)
        assertEquals(listOf("project-a", "—", "project-b"), rows.map { it.name })
        // The phones' name for a missing path, which every other list gives a session without one.
        assertEquals(ProjectPath.name(null), rows[1].name)
    }
}
