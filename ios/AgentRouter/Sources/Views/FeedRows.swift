import SwiftUI

/// One feed row, by kind — the same distinctions the web view draws.
struct FeedRowView: View {
    let row: LiveFeed.Row
    let feed: LiveFeed

    private var item: LiveItem { row.item }
    private var who: String { item.sidechain ? "subagent" : item.role }
    private var dur: String? { item.durMs.flatMap { $0 > 400 ? Fmt.duration(TimeInterval($0) / 1000) : nil } }

    var body: some View {
        switch item.kind {
        case "ask" where item.ask != nil:
            AskFormView(questions: item.ask!.questions, answered: row.answered) { answers, parts in
                await feed.answer(answers, fallback: parts, row: row.id)
            }
            .padding(12)
            .background(RoundedRectangle(cornerRadius: 12).fill(Color.orange.opacity(row.answered ? 0.06 : 0.12)))
        case "tool_use":
            VStack(alignment: .leading, spacing: 4) {
                header(Label(item.name ?? "tool", systemImage: item.name == "Bash" ? "terminal" : "wrench.and.screwdriver"))
                CodeBox(text: item.text ?? "", prompt: item.name == "Bash")
            }
        case "tool_result":
            CodeBox(text: item.text ?? "", label: "OUT")
        case "thinking":
            DisclosureGroup {
                Text(item.text ?? "")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            } label: {
                Text("Thought" + (dur.map { " for \($0)" } ?? ""))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        case "text" where item.role == "user":
            HStack {
                Spacer(minLength: 40)
                VStack(alignment: .trailing, spacing: 3) {
                    if row.pending {
                        Text("you · queued").font(.caption2).foregroundStyle(.secondary)
                    }
                    Text(item.text ?? "")
                        .textSelection(.enabled)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(RoundedRectangle(cornerRadius: 16).fill(Color.accentColor.opacity(0.16)))
                }
                .opacity(row.pending ? 0.7 : 1)
            }
        case "text" where item.role == "assistant":
            VStack(alignment: .leading, spacing: 4) {
                header(Text(who))
                RichText(text: item.text ?? "")
            }
        case "answer":
            VStack(alignment: .leading, spacing: 4) {
                header(Label("answered", systemImage: "checkmark"))
                Text(item.text ?? "").font(.footnote).textSelection(.enabled)
            }
        case "command":
            VStack(alignment: .leading, spacing: 4) {
                header(Label("command", systemImage: "command"))
                Text(item.text ?? "").font(.system(.footnote, design: .monospaced)).textSelection(.enabled)
            }
        case "aside":
            VStack(alignment: .leading, spacing: 4) {
                header(Label("side answer", systemImage: "text.bubble"))
                RichText(text: item.text ?? "")
            }
            .padding(10)
            .background(RoundedRectangle(cornerRadius: 12).fill(Color.teal.opacity(0.08)))
        default:
            VStack(alignment: .leading, spacing: 4) {
                header(Text(who))
                Text(item.text ?? "").font(.footnote).foregroundStyle(.secondary).textSelection(.enabled)
            }
        }
    }

    private func header<L: View>(_ label: L) -> some View {
        HStack(spacing: 6) {
            label.font(.caption.weight(.medium)).foregroundStyle(.secondary)
            if let dur { Text(dur).font(.caption2).foregroundStyle(.tertiary) }
        }
    }
}

/// Monospace, unwrapped, folded behind a toggle rather than cut off.
struct CodeBox: View {
    let text: String
    var label: String? = nil
    var prompt = false
    @State private var expanded = false
    private let fold = 10

    private var lines: Int { text.split(separator: "\n", omittingEmptySubsequences: false).count }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let label {
                Text(label).font(.caption2.weight(.semibold)).foregroundStyle(.tertiary)
            }
            ScrollView(.horizontal, showsIndicators: false) {
                Text((prompt ? "$ " : "") + text)
                    .font(.system(.footnote, design: .monospaced))
                    .lineLimit(expanded ? nil : fold)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if lines > fold {
                Button(expanded ? "Show less" : "Show \(lines) lines") { expanded.toggle() }
                    .font(.caption)
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color(.secondarySystemBackground)))
    }
}

/// Assistant prose, block by block: what the web view's `md()` draws.
struct RichText: View {
    let text: String
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(Markdown.blocks(text).enumerated()), id: \.offset) { _, block in
                switch block {
                case .paragraph(let s):
                    Text(Markdown.inline(s)).textSelection(.enabled)
                case .heading(let level, let s):
                    Text(Markdown.inline(s))
                        .font(level <= 2 ? .headline : .subheadline.weight(.semibold))
                        .padding(.top, 2)
                case .bullets(let items):
                    VStack(alignment: .leading, spacing: 3) {
                        ForEach(Array(items.enumerated()), id: \.offset) { _, it in
                            HStack(alignment: .firstTextBaseline, spacing: 6) {
                                Text("•").foregroundStyle(.secondary)
                                Text(Markdown.inline(it)).textSelection(.enabled)
                            }
                        }
                    }
                case .numbered(let items):
                    VStack(alignment: .leading, spacing: 3) {
                        ForEach(Array(items.enumerated()), id: \.offset) { i, it in
                            HStack(alignment: .firstTextBaseline, spacing: 6) {
                                Text("\(i + 1).").foregroundStyle(.secondary).monospacedDigit()
                                Text(Markdown.inline(it)).textSelection(.enabled)
                            }
                        }
                    }
                case .quote(let s):
                    HStack(alignment: .top, spacing: 8) {
                        RoundedRectangle(cornerRadius: 2).fill(Color.accentColor.opacity(0.5)).frame(width: 3)
                        Text(Markdown.inline(s)).foregroundStyle(.secondary).textSelection(.enabled)
                    }
                case .code(let s):
                    CodeBox(text: s)
                case .table(let rows):
                    ScrollView(.horizontal, showsIndicators: false) {
                        Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 4) {
                            ForEach(Array(rows.enumerated()), id: \.offset) { r, row in
                                GridRow {
                                    ForEach(Array(row.enumerated()), id: \.offset) { _, cell in
                                        Text(Markdown.inline(cell)).font(r == 0 ? .caption.weight(.semibold) : .caption)
                                    }
                                }
                                if r == 0 { Divider() }
                            }
                        }
                        .padding(8)
                        .background(RoundedRectangle(cornerRadius: 8).fill(Color(.secondarySystemBackground)))
                    }
                }
            }
        }
    }
}

/// The editor shows one question at a time with a Submit that answers them together;
/// this shows them all, which on a phone reads better than tabs.
struct AskFormView: View {
    let questions: [AskQuestion]
    let answered: Bool
    let onSubmit: ([String: String], [String]) async -> Void
    @State private var selections: [String: AskAnswers.Selection] = [:]
    @State private var submitting = false
    @State private var hint = "or type a reply below"

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            ForEach(questions) { q in question(q) }
            HStack(spacing: 10) {
                Button(answered ? "Answered" : "Submit answers") { Task { await submit() } }
                    .buttonStyle(.borderedProminent)
                    .disabled(answered || submitting)
                Text(answered ? "" : hint).font(.caption).foregroundStyle(.secondary)
            }
        }
        .disabled(answered)
    }

    private func question(_ q: AskQuestion) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(q.title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            Text(q.question).font(.subheadline)
            ForEach(q.options ?? []) { o in
                Button {
                    toggle(q, o.label)
                } label: {
                    HStack(alignment: .top, spacing: 8) {
                        Image(systemName: isChosen(q, o.label)
                              ? (q.isMulti ? "checkmark.square.fill" : "checkmark.circle.fill")
                              : (q.isMulti ? "square" : "circle"))
                            .foregroundStyle(isChosen(q, o.label) ? Color.accentColor : Color.secondary)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(o.label).font(.subheadline.weight(.medium))
                            if let d = o.description, !d.isEmpty {
                                Text(d).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                        Spacer(minLength: 0)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            // Always offered, as in the editor: none of the options may fit.
            TextField("Other — type your own answer", text: binding(q).other)
                .textFieldStyle(.roundedBorder)
                .font(.subheadline)
                .onChange(of: selections[q.question]?.other ?? "") { _, new in
                    if !q.isMulti, !new.isEmpty { selections[q.question, default: .init()].chosen = [] }
                }
        }
    }

    private func isChosen(_ q: AskQuestion, _ label: String) -> Bool {
        selections[q.question]?.chosen.contains(label) ?? false
    }

    private func toggle(_ q: AskQuestion, _ label: String) {
        var sel = selections[q.question] ?? .init()
        if q.isMulti {
            if sel.chosen.contains(label) { sel.chosen.remove(label) } else { sel.chosen.insert(label) }
        } else {
            sel.chosen = sel.chosen.contains(label) ? [] : [label]
            sel.other = ""
        }
        selections[q.question] = sel
    }

    private func binding(_ q: AskQuestion) -> Binding<AskAnswers.Selection> {
        Binding(get: { selections[q.question] ?? .init() }, set: { selections[q.question] = $0 })
    }

    private func submit() async {
        let (answers, parts) = AskAnswers.compose(questions, selections)
        guard !parts.isEmpty else {
            hint = "Pick at least one answer first."
            return
        }
        submitting = true
        hint = "answering…"
        await onSubmit(answers, parts)
        submitting = false
        hint = "or type a reply below"
    }
}
