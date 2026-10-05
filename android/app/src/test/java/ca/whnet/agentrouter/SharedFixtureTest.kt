package ca.whnet.agentrouter

import ca.whnet.agentrouter.api.LiveSession
import ca.whnet.agentrouter.api.routerJson
import ca.whnet.agentrouter.data.Inline
import ca.whnet.agentrouter.data.Markdown
import ca.whnet.agentrouter.data.Markdown.Block
import ca.whnet.agentrouter.data.SessionGroups
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.Parameterized

/**
 * The fixtures the web view and both phones share, fixtures/live/markdown-blocks.json and
 * fixtures/live/session-groups.json, read from the repository and held against this app's own
 * Markdown blocks and session list, one case at a time. Their `expected` is generated from the
 * web's model, the reference for both phones (docs/session-liveness.md, "Client presentation
 * policy"); each file's `about` says what the shape means. A case marked `pending` is one where
 * matching the web would lose something here: it is skipped, by name, until it is decided.
 */
@RunWith(Parameterized::class)
class SharedFixtureTest(private val title: String, private val file: String, private val case: JsonObject) {

    companion object {
        private const val MARKDOWN = "markdown-blocks.json"
        private const val GROUPS = "session-groups.json"

        @JvmStatic
        @Parameterized.Parameters(name = "{0}")
        fun cases(): List<Array<Any>> = listOf(MARKDOWN, GROUPS).flatMap { file ->
            fixture(file)["cases"]!!.jsonArray.map { c ->
                arrayOf<Any>("$file: ${c.jsonObject.string("name")}", file, c.jsonObject)
            }
        }

        /** The repository's own copy: the tests run in android/app, two folders below its root. */
        private fun fixture(name: String): JsonObject {
            val dir = generateSequence(File("").absoluteFile) { it.parentFile }
                .map { File(it, "fixtures/live") }
                .first { File(it, name).isFile }
            return Json.parseToJsonElement(File(dir, name).readText()).jsonObject
        }

        private fun JsonObject.string(key: String): String = this[key]!!.jsonPrimitive.content
    }

    @Test
    fun `matches the web view`() {
        val pending = case["pending"]?.jsonPrimitive?.contentOrNull
        assumeTrue("pending, and skipped: $title ($pending)", pending == null)
        when (file) {
            MARKDOWN -> assertEquals(title, case["expected"], blocks(Markdown.blocks(case.string("input"))))
            GROUPS -> assertEquals(title, case["expected"], groups(case))
        }
    }

    // ---------- this app's values, in the fixtures' shape ----------

    /** A line's runs, as the screen draws them: adjacent runs in the same styles are one. */
    private fun runs(s: String): JsonArray {
        val merged = mutableListOf<Inline.Run>()
        for (r in Inline.parse(s)) {
            if (r.text.isEmpty()) continue
            val last = merged.lastOrNull()
            if (last != null && last.copy(text = r.text) == r) merged[merged.size - 1] = last.copy(text = last.text + r.text)
            else merged.add(r)
        }
        return JsonArray(
            merged.map { r ->
                buildJsonObject {
                    put("text", r.text)
                    if (r.bold) put("bold", true)
                    if (r.italic) put("italic", true)
                    if (r.code) put("code", true)
                    r.url?.let { put("href", it) }
                }
            },
        )
    }

    private fun block(type: String, vararg fields: Pair<String, JsonElement>) =
        JsonObject(mapOf("type" to JsonPrimitive(type)) + fields)

    private fun blocks(blocks: List<Block>): JsonArray = JsonArray(
        blocks.map { b ->
            when (b) {
                is Block.Paragraph -> block("paragraph", "runs" to runs(b.text))
                is Block.Heading -> block("heading", "level" to JsonPrimitive(b.level), "runs" to runs(b.text))
                is Block.Bullets -> block("list", "ordered" to JsonPrimitive(false), "items" to JsonArray(b.items.map(::runs)))
                is Block.Numbered -> block("list", "ordered" to JsonPrimitive(true), "items" to JsonArray(b.items.map(::runs)))
                is Block.Quote -> block("quote", "runs" to runs(b.text))
                // The language tag is dropped, as on the web (SE-C93).
                is Block.Code -> block("code", "lang" to JsonNull, "text" to JsonPrimitive(b.text))
                is Block.Table -> block(
                    "table",
                    "head" to JsonArray(b.rows.first().map(::runs)),
                    "rows" to JsonArray(b.rows.drop(1).map { row -> JsonArray(row.map(::runs)) }),
                )
            }
        },
    )

    /** What the list screen draws: one part under no machine's heading, then its groups. */
    private fun groups(c: JsonObject): JsonObject {
        val rows = c["sessions"]!!.jsonArray.map { routerJson.decodeFromJsonElement<LiveSession>(it) }
        val parts = SessionGroups.byMachine(rows, c["now"]!!.jsonPrimitive.long)
        assertEquals("$title: one list, under no machine's heading", listOf<String?>(null), parts.map { it.label })
        val g = parts.single().groups
        fun project(p: SessionGroups.Project) = buildJsonObject {
            put("project", p.name)
            put("tone", p.tone)
            put("halo", p.dot?.active ?: false)
            put("sessions", JsonArray(p.sessions.map { JsonPrimitive(it.sessionId) }))
        }
        return buildJsonObject {
            put("recent", JsonArray(g.recent.map(::project)))
            put("older", JsonArray(g.older.map(::project)))
            put("archived", JsonArray(g.archived.map { JsonPrimitive(it.sessionId) }))
        }
    }
}
