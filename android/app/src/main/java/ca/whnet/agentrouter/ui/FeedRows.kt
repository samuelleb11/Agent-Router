package ca.whnet.agentrouter.ui

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.api.AskQuestion
import ca.whnet.agentrouter.data.AskAnswers
import ca.whnet.agentrouter.data.Markdown
import ca.whnet.agentrouter.feed.LiveFeed

@Composable
fun FeedRow(row: LiveFeed.Row, onAnswer: (Map<String, String>, List<String>) -> Unit) {
    val item = row.item
    // Dispatch on `kind` before `role`. A tool result is recorded in the transcript as a
    // user message — that is how the CLI writes it — so testing the role first renders
    // every command's output as something the person typed.
    when (item.kind) {
        "ask" -> item.ask?.let { AskForm(it.questions, row.answered, onAnswer) }
        "tool_use" -> ToolRow(item.name ?: "tool", item.text, item.durMs)
        "tool_result" -> ToolResultRow(item.text.orEmpty())
        "thinking" -> ThinkingRow(item.text.orEmpty(), item.durMs)
        "answer" -> AnswerRow(item.text.orEmpty())
        "interrupt", "command" -> MarkerRow(item.text.orEmpty(), item.kind)
        "text" -> if (item.role == "user") UserRow(item.text.orEmpty(), row.pending)
        else ProseRow(item.text.orEmpty())
        else -> if (item.role == "user") UserRow(item.text.orEmpty(), row.pending)
        else ProseRow(item.text.orEmpty())
    }
}

/** What a tool sent back. Monospace and clipped: these are long and rarely read in full. */
@Composable
private fun ToolResultRow(text: String) {
    if (text.isBlank()) return
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
        Card(
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
            ),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(
                text,
                Modifier.padding(10.dp),
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 12,
            )
        }
    }
}

/** The tool result that carries an answer to a question, which reads as the person's reply. */
@Composable
private fun AnswerRow(text: String) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            "answered",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.tertiary,
        )
        Text(text, style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(1f))
    }
}

/** A cancellation or a slash command — neither is prose, and neither is the person talking. */
@Composable
private fun MarkerRow(text: String, kind: String) {
    Text(
        if (text.isBlank()) kind else text,
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
        style = MaterialTheme.typography.labelMedium,
        color = if (kind == "interrupt") MaterialTheme.colorScheme.error
        else MaterialTheme.colorScheme.outline,
    )
}

@Composable
private fun UserRow(text: String, pending: Boolean) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)) {
        Card(
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.primaryContainer.copy(
                    alpha = if (pending) 0.55f else 1f
                ),
            ),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(
                text,
                Modifier.padding(12.dp),
                color = MaterialTheme.colorScheme.onPrimaryContainer,
                style = MaterialTheme.typography.bodyMedium,
            )
        }
        if (pending) {
            Text(
                "you · queued",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = 2.dp),
            )
        }
    }
}

@Composable
private fun ProseRow(text: String) {
    if (text.isBlank()) return
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Markdown.blocks(text).forEach { MarkdownBlock(it) }
    }
}

@Composable
private fun MarkdownBlock(block: Markdown.Block) {
    when (block) {
        is Markdown.Block.Heading -> Text(
            block.text,
            style = when (block.level) {
                1 -> MaterialTheme.typography.titleLarge
                2 -> MaterialTheme.typography.titleMedium
                else -> MaterialTheme.typography.titleSmall
            },
        )
        is Markdown.Block.Paragraph -> Text(block.text, style = MaterialTheme.typography.bodyMedium)
        is Markdown.Block.Bullets -> Column {
            block.items.forEach { Text("•  $it", style = MaterialTheme.typography.bodyMedium) }
        }
        is Markdown.Block.Numbered -> Column {
            block.items.forEachIndexed { i, s ->
                Text("${i + 1}.  $s", style = MaterialTheme.typography.bodyMedium)
            }
        }
        is Markdown.Block.Quote -> Text(
            block.text,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(start = 10.dp),
        )
        is Markdown.Block.Code -> CodeBox(block.text)
        is Markdown.Block.Table -> Column(Modifier.horizontalScroll(rememberScrollState())) {
            block.rows.forEachIndexed { i, cells ->
                Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                    cells.forEach {
                        Text(
                            it,
                            style = MaterialTheme.typography.bodySmall,
                            fontWeight = if (i == 0) FontWeight.Bold else FontWeight.Normal,
                        )
                    }
                }
            }
        }
    }
}

/** Code is set in monospace and left unwrapped — it scrolls rather than reflows. */
@Composable
fun CodeBox(text: String) {
    Card(
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerHighest),
        shape = RoundedCornerShape(8.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(
            text,
            Modifier.horizontalScroll(rememberScrollState()).padding(10.dp),
            fontFamily = FontFamily.Monospace,
            style = MaterialTheme.typography.bodySmall,
        )
    }
}

@Composable
private fun ToolRow(name: String, detail: String?, durMs: Long?) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Text(
            name,
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.primary,
        )
        Column(Modifier.weight(1f)) {
            if (!detail.isNullOrBlank()) {
                Text(
                    detail,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (durMs != null && durMs > 0) {
                Text(
                    Fmt.duration(durMs),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.outline,
                )
            }
        }
    }
}

@Composable
private fun ThinkingRow(text: String, durMs: Long?) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
        Text(
            if (durMs != null && durMs > 0) "Thought for ${Fmt.duration(durMs)}" else "Thinking",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.outline,
        )
        if (text.isNotBlank()) {
            Text(
                text,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/**
 * The answer form: every question at once.
 *
 * The desktop client asks them one at a time; on a phone you are usually answering from a
 * notification and want the whole thing in front of you. "Other" is always offered, and
 * typing in it is choosing it — which is why picking an option clears it and vice versa on
 * a single-select question.
 */
@Composable
fun AskForm(
    questions: List<AskQuestion>,
    answered: Boolean,
    onAnswer: (Map<String, String>, List<String>) -> Unit,
) {
    var selections by remember { mutableStateOf(mapOf<String, AskAnswers.Selection>()) }
    var complaint by remember { mutableStateOf<String?>(null) }

    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.tertiaryContainer.copy(alpha = if (answered) 0.4f else 1f),
        ),
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
    ) {
        Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            questions.forEach { q ->
                val sel = selections[q.question] ?: AskAnswers.Selection()
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(q.title, style = MaterialTheme.typography.titleSmall)
                    if (q.header != null && q.header != q.question) {
                        Text(q.question, style = MaterialTheme.typography.bodySmall)
                    }
                    (q.options ?: emptyList()).forEach { option ->
                        val picked = option.label in sel.chosen
                        val choose = {
                            val next = if (q.isMulti) {
                                val c = sel.chosen.toMutableSet()
                                if (picked) c.remove(option.label) else c.add(option.label)
                                sel.copy(chosen = c)
                            } else {
                                // Single-select: choosing an option clears what was typed.
                                AskAnswers.Selection(chosen = setOf(option.label), other = "")
                            }
                            selections = selections + (q.question to next)
                            complaint = null
                        }
                        Row(
                            Modifier.fillMaxWidth().then(
                                if (q.isMulti) {
                                    Modifier.toggleable(value = picked, enabled = !answered) { choose() }
                                } else {
                                    Modifier.selectable(selected = picked, enabled = !answered) { choose() }
                                }
                            ).padding(vertical = 2.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            if (q.isMulti) Checkbox(picked, null, enabled = !answered)
                            else RadioButton(picked, null, enabled = !answered)
                            Column(Modifier.weight(1f)) {
                                Text(option.label, style = MaterialTheme.typography.bodyMedium)
                                if (!option.description.isNullOrBlank()) {
                                    Text(
                                        option.description!!,
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                            }
                        }
                    }
                    OutlinedTextField(
                        value = sel.other,
                        onValueChange = { typed ->
                            // Typing is choosing: on a single-select that clears the options.
                            val next = if (q.isMulti) sel.copy(other = typed)
                            else sel.copy(other = typed, chosen = if (typed.isEmpty()) sel.chosen else emptySet())
                            selections = selections + (q.question to next)
                            complaint = null
                        },
                        enabled = !answered,
                        label = { Text("Other") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }
            if (complaint != null) {
                Text(complaint!!, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            }
            Button(
                enabled = !answered,
                onClick = {
                    val (answers, parts) = AskAnswers.compose(questions, selections)
                    if (answers.isEmpty()) complaint = "Pick at least one answer first."
                    else onAnswer(answers, parts)
                },
                modifier = Modifier.fillMaxWidth(),
            ) { Text(if (answered) "Answered" else "Submit") }
        }
    }
}
