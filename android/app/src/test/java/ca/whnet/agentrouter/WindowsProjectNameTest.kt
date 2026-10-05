package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.ProjectRow
import ca.whnet.agentrouter.api.ProxySessionRow
import ca.whnet.agentrouter.api.RequestRow
import ca.whnet.agentrouter.api.SearchHit
import ca.whnet.agentrouter.api.SearchReply
import ca.whnet.agentrouter.api.SearchSessionRow
import ca.whnet.agentrouter.api.routerJson
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * A hosted router hears from Windows machines, whose directories are `C:\Users\…`. Every list that
 * names a project from its directory — the dashboard's projects and sessions, a search's sessions
 * and hits, a request in the usage lists — names it by its last segment, split on `/` and `\`
 * alike, as the live list always has and as the web console does (SE-C222). Split on `/` alone,
 * such a project was named by its whole path.
 */
class WindowsProjectNameTest {

    private val windows = """C:\Users\sample\code\agent-router"""

    /** The same directory as the router writes it in JSON, each backslash escaped. */
    private val wire = windows.replace("\\", "\\\\")

    @Test
    fun `the project list names a Windows project by its last segment`() {
        val projects = routerJson.decodeFromString<List<ProjectRow>>(
            """[{"project":"$wire","sessions":1,"requests":4,"prompt_tokens":1000,"cost":0.1,
                 "output_tokens":10,"cache_read_tokens":900,"subagent_tokens":0,"errors":0,
                 "models":"claude-opus-5","last_seen":2}]""",
        )
        assertEquals("agent-router", projects[0].name)

        val sessions = routerJson.decodeFromString<List<ProxySessionRow>>(
            """[{"id":"2631369d-0980-4930-b3d1-3fd7396c4a20","project":"$wire","git_branch":"main",
                 "entrypoint":"claude-cli/2.1.269 (external, claude-vscode)","started_at":1,
                 "last_seen_at":2,"requests":4,"prompt_tokens":1000,"cost":0.1}]""",
        )
        assertEquals("agent-router", sessions[0].projectName)
    }

    @Test
    fun `a search hit names a Windows project by its last segment`() {
        val reply = routerJson.decodeFromString<SearchReply>(
            """{"query":"body_cap","expression":"body_cap",
                "sessions":[{"session_id":"9fda36f0-d6e8-4623-93b6-5cae13e706c8","project":"$wire",
                             "session_label":null,"git_branch":"dev","blocks":3,"hits":4,
                             "first_at":1790000000000,"last_at":1790000009000}],
                "hits":[{"block_id":41,"kind":"tool_use","chars":120,
                         "session_id":"9fda36f0-d6e8-4623-93b6-5cae13e706c8","role":"assistant",
                         "ordinal":7,"request_id":48761,"at":1790000009000,"project":"$wire",
                         "session_label":null,"snippet":"why is ‹body_cap› 64 KB","rank":-8.04}]}""",
        )
        assertEquals("agent-router", reply.sessions[0].projectName)
        assertEquals("agent-router", reply.hits[0].projectName)
    }

    @Test
    fun `a request in the usage lists names a Windows project by its last segment`() {
        val requests = routerJson.decodeFromString<List<RequestRow>>(
            """[{"id":7,"request_id":"req_1","session_id":"s","project":"$wire","model":"claude-opus-5",
                 "status":200,"started_at":1789200000000,"prompt_tokens":100000,"cost":0.06}]""",
        )
        assertEquals("agent-router", requests[0].projectName)
    }

    /** What each list calls a project, from the directory the router recorded for it. */
    private val lists: Map<String, (String?) -> String> = mapOf(
        // A project the router never resolved arrives as null, which decodes to "" here.
        "the project list" to { p -> ProjectRow(project = p ?: "").name },
        "the dashboard's sessions" to { p -> ProxySessionRow(project = p).projectName },
        "a search's sessions" to { p -> SearchSessionRow(project = p).projectName },
        "a search hit" to { p -> SearchHit(project = p).projectName },
        "a request" to { p -> RequestRow(project = p).projectName },
    )

    @Test
    fun `every list names a directory as the web console does, edge cases included`() {
        val cases = listOf(
            windows to "agent-router",
            // A trailing separator ends no segment.
            "$windows\\" to "agent-router",
            "/Users/sample/code/agent-router/" to "agent-router",
            // A path that mixes the two splits on both.
            """C:/Users/sample\code/agent-router""" to "agent-router",
            "agent-router" to "agent-router",
            """C:\""" to "C:",
            // Separators alone name no segment, and are named as written.
            "/" to "/",
            // No directory at all: the app's dash, where the web says "(unresolved)" — never a
            // blank name.
            "" to "—",
            null to "—",
        )
        for ((list, name) in lists) {
            for ((path, expected) in cases) assertEquals("$list, for ${path ?: "null"}", expected, name(path))
        }
    }

    @Test
    fun `the live list and every other list agree on a project's name`() {
        for (path in listOf(windows, "$windows\\", """C:/Users/sample\code/agent-router""", "/Users/sample/code/agent-router")) {
            val live = LiveSession(sessionId = "d6e4d7da", cwd = path, project = "C--Users-sample-code-agent-router")
            assertEquals(path, "agent-router", live.projectName)
            for ((list, name) in lists) assertEquals("$list, for $path", live.projectName, name(path))
        }
    }
}
