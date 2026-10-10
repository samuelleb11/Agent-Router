// Splitting a search snippet into plain and matched runs.
//
// Outside Compose so it can be tested on the JVM, for the same reason Inline is.

package ca.whnet.agentrouter.data

object Snippet {
    data class Run(val text: String, val match: Boolean)

    private const val OPEN = '‹'
    private const val CLOSE = '›'

    /// The router wraps each match in ‹…›.
    ///
    /// Split on either marker and alternate, rather than pairing an open with a close:
    /// a stray marker in the conversation's own text can then only shift which run is
    /// emphasised, where pairing could swallow the rest of the snippet looking for a
    /// close that never comes.
    fun parse(src: String): List<Run> {
        if (src.isEmpty()) return emptyList()
        val out = mutableListOf<Run>()
        val piece = StringBuilder()
        var match = false
        for (c in src) {
            if (c == OPEN || c == CLOSE) {
                if (piece.isNotEmpty()) {
                    out.add(Run(piece.toString(), match))
                    piece.clear()
                }
                match = !match
            } else {
                piece.append(c)
            }
        }
        if (piece.isNotEmpty()) out.add(Run(piece.toString(), match))
        return out
    }
}
