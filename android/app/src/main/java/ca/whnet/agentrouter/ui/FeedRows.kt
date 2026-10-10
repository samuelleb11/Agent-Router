// One feed row, by kind — the same distinctions the web view and the iPhone draw.

package ca.whnet.agentrouter.ui

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Build
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Terminal
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import ca.whnet.agentrouter.api.AskQuestion
import ca.whnet.agentrouter.data.AskAnswers
import ca.whnet.agentrouter.data.Inline
import ca.whnet.agentrouter.data.Markdown
import ca.whnet.agentrouter.feed.LiveFeed

private val RowPad = Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 5.dp)

@Composable
fun FeedRow(row: LiveFeed.Row, onAnswer: (Map<String, String>, List<String>) -> Unit) {
    val item = row.item
    val who = if (item.sidechain) "subagent" else item.role
    // Under 400 ms is not worth a caption; it reads as noise on every row.
    val dur = item.durMs?.takeIf { it > 400 }?.let { Fmt.duration(it) }

    // Dispatch on `kind` before `role`. A tool result is recorded in the transcript as a
    // user message — that is how the CLI writes it — so testing the role first renders
    // every command's output as something the person typed.
    when (item.kind) {
        "ask" -> item.ask?.let {
            Box(RowPad) { AskForm(it.questions, row.answered, onAnswer) }
        }
        "tool_use" -> Column(RowPad, verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Header(item.name ?: "tool", dur, icon = if (item.name == "Bash") Icons.Default.Terminal else Icons.Default.Build)
            CodeBox(item.text.orEmpty(), prompt = item.name == "Bash")
        }
        // A result with no text — an image, or a tool that says nothing on success — is
        // not worth an empty box.
        "tool_result" -> if (!item.text.isNullOrBlank()) Box(RowPad) { CodeBox(item.text, label = "OUT") }
        "thinking" -> Box(RowPad) { Thinking(item.text.orEmpty(), dur) }
        "answer" -> Column(RowPad, verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Header("answered", dur, check = true)
            Selectable { Text(item.text.orEmpty(), style = MaterialTheme.typography.bodySmall) }
        }
        "command" -> Column(RowPad, verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Header("command", dur)
            Selectable {
                Text(item.text.orEmpty(), style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace)
            }
        }
        "interrupt" -> Header("interrupted", dur, modifier = RowPad, tint = MaterialTheme.colorScheme.error)
        "aside" -> Column(
            RowPad.then(Modifier.clip(RoundedCornerShape(12.dp)).background(Tint.teal.copy(alpha = 0.08f)).padding(10.dp)),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Header("side answer", dur)
            RichText(item.text.orEmpty())
        }
        else -> when (item.role) {
            "user" -> UserBubble(item.text.orEmpty(), row.pending)
            "assistant" -> Column(RowPad, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Header(who, dur)
                RichText(item.text.orEmpty())
            }
            else -> Column(RowPad, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Header(who, dur)
                Selectable {
                    Text(
                        item.text.orEmpty(),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}

/** The small line above a row saying who or what it is, and how long it took. */
@Composable
private fun Header(
    label: String,
    dur: String?,
    modifier: Modifier = Modifier,
    icon: androidx.compose.ui.graphics.vector.ImageVector? = null,
    check: Boolean = false,
    tint: Color = MaterialTheme.colorScheme.onSurfaceVariant,
) {
    Row(modifier, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
        if (check) Icon(Icons.Default.Check, null, Modifier.size(13.dp), tint = tint)
        if (icon != null) Icon(icon, null, Modifier.size(13.dp), tint = tint)
        Text(label, style = MaterialTheme.typography.labelMedium, color = tint, fontWeight = FontWeight.Medium)
        if (dur != null) Text(dur, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.outline)
    }
}

/** What the person typed: on the right, tinted, and dimmer while it is only an echo. */
@Composable
private fun UserBubble(text: String, pending: Boolean) {
    Row(RowPad, horizontalArrangement = Arrangement.End) {
        Spacer(Modifier.width(40.dp))
        Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(3.dp)) {
            if (pending) {
                Text("you · queued", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Selectable {
                Text(
                    text,
                    Modifier
                        .clip(RoundedCornerShape(16.dp))
                        .background(MaterialTheme.colorScheme.primary.copy(alpha = if (pending) 0.10f else 0.16f))
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        }
    }
}

/** Thinking is folded away: it is there when wanted, and not in the way otherwise. */
@Composable
private fun Thinking(text: String, dur: String?) {
    var open by rememberSaveable { mutableStateOf(false) }
    Column(Modifier.animateContentSize()) {
        Row(
            Modifier.clickable { open = !open }.padding(vertical = 2.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Icon(
                if (open) Icons.Default.KeyboardArrowDown else Icons.Default.KeyboardArrowRight,
                null, Modifier.size(16.dp), tint = MaterialTheme.colorScheme.outline,
            )
            Text(
                "Thought" + (dur?.let { " for $it" } ?: ""),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (open && text.isNotBlank()) {
            Selectable {
                Text(
                    text,
                    Modifier.padding(start = 20.dp, top = 2.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

/**
 * Monospace, unwrapped, and folded behind a toggle rather than cut off: a long result is
 * ten lines and a "Show 140 lines" button, not a wall.
 */
@Composable
fun CodeBox(text: String, label: String? = null, prompt: Boolean = false) {
    val fold = 10
    var expanded by rememberSaveable { mutableStateOf(false) }
    val lines = remember(text) { text.split('\n') }
    val shown = if (expanded || lines.size <= fold) text else lines.take(fold).joinToString("\n")
    Column(
        Modifier.fillMaxWidth()
            .clip(RoundedCornerShape(10.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .padding(10.dp)
            .animateContentSize(),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        if (label != null) {
            Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.outline, fontWeight = FontWeight.SemiBold)
        }
        Selectable {
            Text(
                (if (prompt) "$ " else "") + shown,
                Modifier.horizontalScroll(rememberScrollState()),
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
            )
        }
        if (lines.size > fold) {
            TextButton(onClick = { expanded = !expanded }, contentPadding = androidx.compose.foundation.layout.PaddingValues(0.dp)) {
                Text(if (expanded) "Show less" else "Show ${lines.size} lines", style = MaterialTheme.typography.labelMedium)
            }
        }
    }
}

/** Assistant prose, block by block: what the web view's `md()` draws. */
@Composable
fun RichText(text: String) {
    if (text.isBlank()) return
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Markdown.blocks(text).forEach { block ->
            when (block) {
                is Markdown.Block.Paragraph -> Selectable { Text(inline(block.text), style = MaterialTheme.typography.bodyMedium) }
                // The level the web draws, `#` the third and `##` the fourth (SE-C98).
                is Markdown.Block.Heading -> Text(
                    inline(block.text),
                    style = if (block.level <= 4) MaterialTheme.typography.titleMedium else MaterialTheme.typography.titleSmall,
                    modifier = Modifier.padding(top = 2.dp),
                )
                is Markdown.Block.Bullets -> Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    block.items.forEach { item ->
                        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                            Text("•", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            Selectable { Text(inline(item), style = MaterialTheme.typography.bodyMedium) }
                        }
                    }
                }
                is Markdown.Block.Numbered -> Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    block.items.forEachIndexed { i, item ->
                        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                            Text("${i + 1}.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            Selectable { Text(inline(item), style = MaterialTheme.typography.bodyMedium) }
                        }
                    }
                }
                is Markdown.Block.Quote -> Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(
                        Modifier.width(3.dp).height(20.dp).clip(RoundedCornerShape(2.dp))
                            .background(MaterialTheme.colorScheme.primary.copy(alpha = 0.5f)),
                    )
                    Selectable {
                        Text(inline(block.text), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
                is Markdown.Block.Code -> CodeBox(block.text)
                is Markdown.Block.Table -> Column(
                    Modifier.horizontalScroll(rememberScrollState())
                        .clip(RoundedCornerShape(8.dp))
                        .background(MaterialTheme.colorScheme.surfaceContainerHigh)
                        .padding(8.dp),
                    verticalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    block.rows.forEachIndexed { r, cells ->
                        Row(horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                            cells.forEach {
                                Text(
                                    inline(it),
                                    style = MaterialTheme.typography.bodySmall,
                                    fontWeight = if (r == 0) FontWeight.SemiBold else FontWeight.Normal,
                                    modifier = Modifier.widthIn(min = 40.dp),
                                )
                            }
                        }
                        if (r == 0) HorizontalDivider()
                    }
                }
            }
        }
    }
}

/** Long-press to select and copy, which a feed of commands and paths needs constantly. */
@Composable
private fun Selectable(content: @Composable () -> Unit) = SelectionContainer { content() }

/** The runs the parser produced, as styled spans. Links open in the browser on tap. */
@Composable
private fun inline(s: String): AnnotatedString {
    val link = MaterialTheme.colorScheme.primary
    val codeBg = MaterialTheme.colorScheme.surfaceContainerHigh
    return remember(s, link, codeBg) {
        buildAnnotatedString {
            for (run in Inline.parse(s)) {
                val style = SpanStyle(
                    fontWeight = if (run.bold) FontWeight.Bold else null,
                    fontStyle = if (run.italic) FontStyle.Italic else null,
                    fontFamily = if (run.code) FontFamily.Monospace else null,
                    background = if (run.code) codeBg else Color.Unspecified,
                )
                if (run.url != null) {
                    withLink(
                        LinkAnnotation.Url(
                            run.url,
                            TextLinkStyles(style = style.copy(color = link, textDecoration = TextDecoration.Underline)),
                        ),
                    ) { append(run.text) }
                } else {
                    withStyle(style) { append(run.text) }
                }
            }
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

    Column(
        Modifier.fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            // Orange is the colour of "waiting for you" everywhere else in the app.
            .background(Tint.orange.copy(alpha = if (answered) 0.06f else 0.12f))
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
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
                            if (q.isMulti) Modifier.toggleable(value = picked, enabled = !answered) { choose() }
                            else Modifier.selectable(selected = picked, enabled = !answered) { choose() }
                        ).padding(vertical = 2.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        if (q.isMulti) Checkbox(picked, null, enabled = !answered)
                        else RadioButton(picked, null, enabled = !answered)
                        Column(Modifier.weight(1f)) {
                            Text(option.label, style = MaterialTheme.typography.bodyMedium)
                            if (!option.description.isNullOrBlank()) {
                                Text(option.description!!, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
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
