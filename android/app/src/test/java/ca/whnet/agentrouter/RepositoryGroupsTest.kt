package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.ProjectRow
import ca.whnet.agentrouter.api.ProxySessionRow
import ca.whnet.agentrouter.api.decodeReply
import ca.whnet.agentrouter.data.SessionGroups
import ca.whnet.agentrouter.feed.LiveFeed
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * A project is its git repository, not its directory. The router names each live session's
 * repository — its directory's `origin` remote, else its first, spelled canonically — so a
 * repository's clones and lane worktrees, once several projects named after their directories,
 * are one project named after the repository, as on the web console and the iPhone. A row the
 * router found no repository for is named by its directory as it always was; and the project's
 * totals are asked by its key, the repository, else the directory.
 */
class RepositoryGroupsTest {

    private val repository = "gitlab.example.com/acme/ui-kit-web"

    private fun row(id: String, cwd: String?, repository: String? = null, ms: Long = 1_000) = LiveSession(
        sessionId = id,
        cwd = cwd,
        repository = repository,
        lastActivityMs = ms,
        active = true,
    )

    @Test
    fun `a repository's worktrees are one project named after it`() {
        val rows = listOf(
            row("a", "/Users/sample/code/ui-kit-web", repository, ms = 2_000),
            row("b", "/Users/sample/code/ui-kit-web-lane-2", repository, ms = 3_000),
            row("c", "/Users/sample/code/scratch"),
        )
        val groups = SessionGroups.build(rows, now = 4_000)

        assertEquals(listOf("scratch", "ui-kit-web"), groups.recent.map { it.name })
        val project = groups.recent.single { it.name == "ui-kit-web" }
        assertEquals(listOf("b", "a"), project.sessions.map { it.sessionId })
        // Without a repository, the directory's last segment, as before.
        assertEquals(listOf("c"), groups.recent.single { it.name == "scratch" }.sessions.map { it.sessionId })
        assertEquals("ui-kit-web-lane-2", row("d", "/Users/sample/code/ui-kit-web-lane-2").projectName)
        // An empty repository is no repository.
        assertEquals("scratch", row("e", "/Users/sample/code/scratch", "").projectName)
    }

    @Test
    fun `a live row decodes with its repository and without one`() {
        val rows = decodeReply<List<LiveSession>>(
            """[{"session_id":"a","cwd":"/Users/sample/code/ui-kit-web-lane-2","project":"ui-kit-web-lane-2",
                 "last_activity_ms":1,"active":true,"state":"working","repository":"$repository"},
                {"session_id":"b","cwd":"/Users/sample/code/scratch","project":"scratch",
                 "last_activity_ms":1,"active":false,"state":"idle"},
                {"session_id":"c","cwd":"/Users/sample/code/other","project":"other",
                 "last_activity_ms":1,"repository":null}]""",
        )
        assertEquals(listOf(repository, null, null), rows.map { it.repository })
        assertEquals(listOf("ui-kit-web", "scratch", "other"), rows.map { it.projectName })
    }

    @Test
    fun `the project's totals are asked by its repository, else its directory`() {
        assertEquals(
            "/api/live/project?project=gitlab.example.com%2Facme%2Fui-kit-web",
            LiveFeed.projectPath(row("a", "/Users/sample/code/ui-kit-web-lane-2", repository)),
        )
        assertEquals(
            "/api/live/project?project=%2FUsers%2Fsample%2Fcode%2Fscratch",
            LiveFeed.projectPath(row("b", "/Users/sample/code/scratch")),
        )
        assertEquals(
            "/api/live/project?project=%2FUsers%2Fsample%2Fcode%2Fscratch",
            LiveFeed.projectPath(row("c", "/Users/sample/code/scratch", "")),
        )
        assertNull(LiveFeed.projectPath(row("d", null)))
    }

    @Test
    fun `the dashboard's projects and sessions decode with the new fields and without them`() {
        val projects = decodeReply<List<ProjectRow>>(
            """[{"project":"$repository","repository":"$repository","sessions":2,"requests":10,"cost":1.5,
                 "directories":[
                   {"cwd":"/Users/sample/code/ui-kit-web","machine":{"id":"m1","label":"mac"},
                    "remotes":[{"name":"origin","url":"git@gitlab.example.com:acme/ui-kit-web.git",
                                "repository":"$repository"}]},
                   {"cwd":"/home/sample/ui-kit-web","remotes":[]}]},
                {"project":"/Users/sample/code/scratch","sessions":1,"requests":2,"cost":0.1}]""",
        )
        assertEquals(listOf("ui-kit-web", "scratch"), projects.map { it.name })
        assertEquals(repository, projects[0].repository)
        assertEquals(2, projects[0].directories.size)
        assertEquals("mac", projects[0].directories[0].machine?.label)
        assertEquals(repository, projects[0].directories[0].remotes.single().repository)
        assertNull(projects[0].directories[1].machine)
        assertNull(projects[1].repository)
        assertEquals(emptyList<Any>(), projects[1].directories)

        val sessions = decodeReply<List<ProxySessionRow>>(
            """[{"id":"2631369d","project":"$repository","cwd":"/Users/sample/code/ui-kit-web-lane-2",
                 "repository":"$repository","machine":{"id":"m1","label":"mac"},"started_at":1,"last_seen_at":2},
                {"id":"9fda36f0","project":"/Users/sample/code/scratch","started_at":1,"last_seen_at":2}]""",
        )
        assertEquals(listOf("ui-kit-web", "scratch"), sessions.map { it.projectName })
        assertEquals("/Users/sample/code/ui-kit-web-lane-2", sessions[0].cwd)
        assertEquals("m1", sessions[0].machine?.id)
        assertNull(sessions[1].cwd)
        assertNull(sessions[1].repository)
        assertNull(sessions[1].machine)
    }
}
