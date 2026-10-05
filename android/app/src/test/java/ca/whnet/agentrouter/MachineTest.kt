package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.RouterFailure
import ca.whnet.agentrouter.api.SessionMachine
import ca.whnet.agentrouter.api.routerJson
import ca.whnet.agentrouter.data.RouterState
import ca.whnet.agentrouter.data.SessionGroups
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * A hosted router merges the live lists of every machine running its agent, and names each
 * row's machine. A router on a Mac names none, and its list must look exactly as it did.
 */
class MachineTest {

    private val now = 1_790_000_000_000L

    private fun row(json: String): LiveSession = routerJson.decodeFromString(json)

    /** A row as a router sends it. [machine] adds the key only a hosted router sends. */
    private fun session(
        id: String, project: String, agoHours: Double, machine: String?, archived: Boolean = false,
    ): LiveSession {
        val named = machine?.let { ""","machine":{"id":"id-$it","label":"$it"}""" }.orEmpty()
        return row(
            """{"session_id":"$id","cwd":"/code/$project","project":"-code-$project","git_branch":null,"title":null,
                "created_ms":0,"last_activity_ms":${now - (agoHours * 3_600_000).toLong()},"active":false,
                "state":"idle","archived":$archived,"wrapped":true,"bytes":10$named}"""
        )
    }

    // ------------------------------------------------------------ decoding

    @Test
    fun `a row from a hosted router names its machine`() {
        val s = row(
            """{"session_id":"d6e4d7da-9d0e-47cb-86ca-37b4bc53b9c3","cwd":"/code/agent-router",
                "project":"-code-agent-router","git_branch":"dev","title":"Machine agent","created_ms":1790000000000,
                "last_activity_ms":1790000500000,"active":true,"state":"working","archived":false,"wrapped":true,
                "bytes":123,"activity":"generating","tone":"busy","source":"registry","as_of_ms":1790000520000,
                "machine":{"id":"5f0c8a7e-2b1d-4c3e-9a8f-1e2d3c4b5a69","label":"mac-studio","platform":"macos"}}"""
        )
        assertEquals(SessionMachine("5f0c8a7e-2b1d-4c3e-9a8f-1e2d3c4b5a69", "mac-studio"), s.machine)
        assertEquals("mac-studio", s.machineLabel)
        assertEquals("the rest of the row reads as it always did", "agent-router", s.projectName)
        assertEquals("generating", s.activity)
    }

    @Test
    fun `a row without a machine still decodes, and names none`() {
        // Exactly what a router on a Mac sends, and every router sent before hosting.
        val local = row(
            """{"session_id":"aaaa1111","cwd":"/code/x","project":"-code-x","git_branch":null,"title":null,
                "created_ms":0,"last_activity_ms":1790000000000,"active":true,"state":"working",
                "archived":false,"wrapped":false,"bytes":10}"""
        )
        assertNull(local.machine)
        assertNull(local.machineLabel)
        assertEquals("x", local.projectName)
        assertNull(row("""{"session_id":"bare0000","machine":null}""").machine)

        // A machine whose name is missing or blank has no name to show, but is still a machine.
        val unnamed = row("""{"session_id":"x","machine":{"id":"m1"}}""")
        assertEquals(SessionMachine(id = "m1"), unnamed.machine)
        assertNull(unnamed.machineLabel)
        assertNull(row("""{"session_id":"x","machine":{"id":"m1","label":"  "}}""").machineLabel)
        assertEquals("win-box", row("""{"session_id":"x","machine":{"id":"m2","label":" win-box "}}""").machineLabel)
    }

    // ------------------------------------------------------------ grouping

    @Test
    fun `one machine, or none, is the list exactly as it always was`() {
        for (machine in listOf(null, "mac-studio")) {
            val rows = listOf(
                session("aaaa0001", "project-b", 0.5, machine),
                session("aaaa0002", "project-a", 2.0, machine),
                session("aaaa0003", "project-a", 30.0, machine),
                session("aaaa0004", "project-a", 1.0, machine, archived = true),
            )
            assertEquals(
                "machine $machine: one entry, unlabelled, holding the list as build arranges it",
                listOf(SessionGroups.Machine(null, SessionGroups.build(rows, now))),
                SessionGroups.byMachine(rows, now),
            )
        }
        assertEquals(
            "nothing this week is one empty entry, so the page still says so",
            listOf(SessionGroups.Machine(null, SessionGroups.build(emptyList(), now))),
            SessionGroups.byMachine(emptyList(), now),
        )
    }

    @Test
    fun `two machines are grouped by machine, then as before`() {
        val rows = listOf(
            session("win00001", "agent-router", 0.2, "Win-Box"),
            session("mac00001", "agent-router", 1.0, "mac-studio"),
            session("mac00002", "northstar", 3.0, "mac-studio"),
            session("win00002", "agent-router", 20.0, "Win-Box"),
            session("mac00003", "agent-router", 0.1, "mac-studio"),
            session("mac00004", "northstar", 2.0, "mac-studio", archived = true),
        )
        val split = SessionGroups.byMachine(rows, now)
        assertEquals("machines by name, whatever their case", listOf("mac-studio", "Win-Box"), split.map { it.label })
        val (mac, win) = split

        assertEquals(
            "each machine arranged exactly as a whole list is",
            SessionGroups.build(rows.filter { it.machineLabel == "mac-studio" }, now), mac.groups,
        )
        assertEquals(listOf("agent-router", "northstar"), mac.groups.recent.map { it.name })
        assertEquals("newest first within a project", listOf("mac00003", "mac00001"), mac.groups.recent[0].sessions.map { it.shortId })
        assertEquals(emptyList<SessionGroups.Project>(), mac.groups.older)
        assertEquals(listOf("mac00004"), mac.groups.archived.map { it.shortId })

        // The same project on the other machine is another project, with its own older fold.
        assertEquals(listOf("agent-router"), win.groups.recent.map { it.name })
        assertEquals(listOf("win00001"), win.groups.recent.single().sessions.map { it.shortId })
        assertEquals(listOf("win00002"), win.groups.older.flatMap { p -> p.sessions.map { it.shortId } })
        assertEquals(emptyList<LiveSession>(), win.groups.archived)
    }

    @Test
    fun `rows that name no machine come last, and names differing only in case stay two machines`() {
        val rows = listOf(
            session("none0001", "p", 1.0, null),
            session("zeta0001", "p", 1.0, "zeta"),
            session("alfa0001", "p", 1.0, "alpha"),
            session("ALFA0002", "p", 1.0, "ALPHA"),
        )
        val split = SessionGroups.byMachine(rows, now)
        assertEquals(listOf("alpha", "ALPHA", "zeta", null), split.map { it.label })
        assertEquals("not one session lost", rows.size, split.sumOf { m -> m.groups.recent.sumOf { it.sessions.size } })
    }

    // ------------------------------------------------------------ copy

    @Test
    fun `a missing password is the deployment's to set on a hosted router`() {
        assertEquals(
            "a router on a Mac reads exactly as before",
            "This router has no password yet — set one on its dashboard, on the Mac",
            RouterState.describe(RouterFailure.NoPasswordSet),
        )
        assertEquals(
            "This router has no password yet — a hosted router's password is set by its deployment",
            RouterState.describe(RouterFailure.NoPasswordSet, hosted = true),
        )
        assertEquals(
            "every other failure reads the same either way",
            "Wrong password for this router", RouterState.describe(RouterFailure.Unauthorized, hosted = true),
        )
    }
}
