package ca.whnet.agentrouter.api

/** One server-sent event. [event] is null for an unnamed `data:` message. */
data class SseEvent(val event: String?, val data: String)

/**
 * Bytes into lines — every line, including the empty ones.
 *
 * This is not a convenience over a line-reading helper, it is the whole point: an SSE frame
 * is terminated *by* a blank line, so any reader that quietly drops empty lines (which most
 * do) means no frame ever ends and the feed stays blank forever. The iOS client hit exactly
 * this and hand-rolled the same splitter; the comment there is worth keeping in both.
 */
class SseLineSplitter {
    // Bytes, not characters. A line is decoded as UTF-8 only once it is whole: building a
    // String a byte at a time turns every multi-byte character into mojibake, so the "…"
    // the router appends to clipped text arrives as "â¦".
    private var buf = ByteArray(256)
    private var len = 0

    /** The completed line, or null if this byte did not finish one. */
    fun feed(byte: Int): String? {
        if (byte != '\n'.code) {
            if (len == buf.size) buf = buf.copyOf(buf.size * 2)
            buf[len++] = byte.toByte()
            return null
        }
        if (len > 0 && buf[len - 1] == '\r'.code.toByte()) len--
        val line = String(buf, 0, len, Charsets.UTF_8)
        len = 0
        return line
    }
}

/**
 * The text/event-stream framing, one line at a time: `event:` and `data:` accumulate, a blank
 * line delivers what has accumulated, and a line starting `:` is a keep-alive to be ignored.
 * Axum sends those every few seconds while a session is quiet.
 */
class SseParser {
    private var event: String? = null
    private val data = mutableListOf<String>()

    /** The event this line completed, or null. */
    fun feed(line: String): SseEvent? {
        if (line.isEmpty()) {
            val out = if (data.isEmpty()) null else SseEvent(event, data.joinToString("\n"))
            event = null
            data.clear()
            return out
        }
        if (line.startsWith(":")) return null
        val colon = line.indexOf(':')
        val field: String
        var value: String
        if (colon >= 0) {
            field = line.substring(0, colon)
            value = line.substring(colon + 1)
            if (value.startsWith(" ")) value = value.substring(1)
        } else {
            field = line
            value = ""
        }
        when (field) {
            "event" -> event = value
            "data" -> data.add(value)
        }
        return null
    }
}
