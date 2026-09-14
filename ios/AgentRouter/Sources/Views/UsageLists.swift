import SwiftUI

/// Projects the router has served, in this scope. Tap one to narrow the page to it.
struct ProjectsView: View {
    let usage: UsageState
    var body: some View {
        List(usage.projects) { p in
            NavigationLink(value: UsageRoute(routerId: usage.router.router.id, scope: UsageScope(account: usage.scope.account, project: p.project))) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(p.name).font(.headline)
                    Text(p.project).font(.caption2).foregroundStyle(.tertiary).lineLimit(1).truncationMode(.head)
                    Text("\(Fmt.count(p.sessions)) sessions · \(Fmt.count(p.requests)) req · \(Fmt.compact(p.promptTokens)) read · \(Fmt.usd(p.cost))")
                        .font(.caption).foregroundStyle(.secondary)
                    HStack(spacing: 6) {
                        ForEach((p.models ?? "").split(separator: ",").map(String.init), id: \.self) { m in
                            Text(Fmt.shortModel(m)).font(.caption2).padding(.horizontal, 6).padding(.vertical, 1)
                                .background(Capsule().fill(FamilyColor.of(Bucket.family(m)).opacity(0.15)))
                        }
                        if p.errors > 0 { Text("\(Fmt.count(p.errors)) errors").font(.caption2).foregroundStyle(.red) }
                        Spacer()
                        if let d = Fmt.date(ms: p.lastSeen) { Text(Fmt.ago(d)).font(.caption2).foregroundStyle(.secondary) }
                    }
                }
                .padding(.vertical, 2)
            }
        }
        .navigationTitle("Projects")
        .overlay { if usage.projects.isEmpty { ContentUnavailableView("No projects", systemImage: "folder", description: Text("Nothing routed in this range.")) } }
        .refreshable { await usage.refresh() }
    }
}

/// Sessions the proxy served, in this scope — with a way into the live view when the
/// transcript is still around.
struct ProxySessionsView: View {
    let usage: UsageState
    var body: some View {
        List(usage.sessions) { s in
            NavigationLink(value: UsageRoute(routerId: usage.router.router.id, scope: UsageScope(account: usage.scope.account, project: usage.scope.project, session: s.id))) {
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 6) {
                        Text(s.shortId).monospaced().font(.subheadline.weight(.medium))
                        if let b = s.gitBranch { Label(b, systemImage: "arrow.triangle.branch").font(.caption).foregroundStyle(.secondary).lineLimit(1) }
                        if !s.client.isEmpty { Text(s.client).font(.caption2).foregroundStyle(.tertiary) }
                        Spacer()
                        Text(Fmt.ago(Date(timeIntervalSince1970: TimeInterval(s.lastSeenAt) / 1000))).font(.caption2).foregroundStyle(.secondary)
                    }
                    Text(s.projectName).font(.caption).foregroundStyle(.secondary)
                    Text("\(Fmt.count(s.requests)) req · \(Fmt.compact(s.promptTokens)) read · \(Fmt.compact(s.outputTokens)) out · \(Fmt.usd(s.cost))"
                         + (s.subagents > 0 ? " · \(Fmt.count(s.subagents)) subagents" : "")
                         + (s.errors > 0 ? " · \(Fmt.count(s.errors)) errors" : ""))
                        .font(.caption).foregroundStyle(.secondary)
                    if let live = usage.router.sessions.first(where: { $0.sessionId == s.id }) {
                        NavigationLink(value: SessionRoute(routerId: usage.router.router.id, session: live)) {
                            Label("Open live", systemImage: "bubble.left.and.bubble.right").font(.caption)
                        }
                        .buttonStyle(.borderless)
                    }
                }
                .padding(.vertical, 2)
            }
        }
        .navigationTitle("Sessions")
        .overlay { if usage.sessions.isEmpty { ContentUnavailableView("No sessions", systemImage: "rectangle.stack", description: Text("Nothing routed in this range.")) } }
        .refreshable { await usage.refresh() }
    }
}

/// What the assistant called, and what came back — two lists, as on the dashboard.
struct ToolsView: View {
    let usage: UsageState
    var body: some View {
        let t = usage.tools
        List {
            if let called = t?.called, !called.isEmpty {
                let max = called.map(\.calls).max() ?? 1
                Section {
                    ForEach(called) { c in
                        BarRow(value: c.calls, max: max, color: .accentColor,
                               text: "\(Fmt.count(c.calls)) calls · \(Fmt.count(c.requests)) requests" + (c.subagentCalls > 0 ? " · \(Fmt.count(c.subagentCalls)) by subagents" : "")) {
                            Text(c.name ?? "?").font(.subheadline)
                        }
                    }
                } header: { Text("Called") } footer: { Text("Tool calls the assistant made, by name.") }
            }
            if let produced = t?.produced, !produced.isEmpty {
                let max = produced.map(\.estTokens).max() ?? 1
                Section {
                    ForEach(produced) { p in
                        BarRow(value: p.estTokens, max: max, color: .pink,
                               text: "\(Fmt.count(p.results)) results · \(Fmt.compact(p.chars)) chars · largest \(Fmt.compact(p.maxTokens)) · \(Fmt.usd(p.estCostOnce)) if paid once" + (p.errors > 0 ? " · \(Fmt.count(p.errors)) failed" : "")) {
                            Text(p.name ?? "?").font(.subheadline)
                        }
                    }
                } header: { Text("Results") } footer: { Text("What came back, by estimated tokens. Each result is counted once, however many turns replayed it.") }
            }
        }
        .navigationTitle("Tools")
        .overlay { if (t?.called.isEmpty ?? true) && (t?.produced.isEmpty ?? true) { ContentUnavailableView("No tool use", systemImage: "wrench.and.screwdriver", description: Text("Nothing in this range.")) } }
        .refreshable { await usage.refresh() }
    }
}

/// Every request the proxy saw in scope, newest first.
struct RequestsView: View {
    let usage: UsageState
    var body: some View {
        List(usage.requests) { r in
            NavigationLink(value: RequestRoute(routerId: usage.router.router.id, request: r)) {
                RequestRowView(r: r)
            }
        }
        .navigationTitle("Requests")
        .overlay { if usage.requests.isEmpty { ContentUnavailableView("No requests", systemImage: "list.bullet", description: Text("Nothing in this range.")) } }
        .refreshable { await usage.refresh() }
    }
}

struct RequestRowView: View {
    let r: RequestRow
    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Text(Fmt.hm(Date(timeIntervalSince1970: TimeInterval(r.startedAt) / 1000))).monospacedDigit()
                Text(Fmt.shortModel(r.model)).foregroundStyle(.secondary)
                if r.isSubagent { Text("subagent").font(.caption2).foregroundStyle(.pink) }
                Spacer()
                if r.isError {
                    Text(r.status.map(String.init) ?? "error").font(.caption).foregroundStyle(.red)
                } else if let s = r.stopReason {
                    Text(s).font(.caption).foregroundStyle(.secondary)
                }
            }
            .font(.subheadline)
            Text("\(Fmt.compact(r.promptTokens)) read · \(Fmt.compact(r.outputTokens)) out · \(Fmt.usd(r.cost)) · ttfb \(Fmt.ms(r.ttfbMs.map(Double.init))) · \(Fmt.ms(r.durationMs.map(Double.init)))")
                .font(.caption).foregroundStyle(.secondary)
            if !r.toolList.isEmpty {
                Text(r.toolList.joined(separator: ", ")).font(.caption2).foregroundStyle(.tertiary).lineLimit(1)
            }
        }
        .padding(.vertical, 2)
    }
}

/// One request in full — the dashboard's expanded row.
struct RequestDetailView: View {
    let r: RequestRow
    var body: some View {
        List {
            Section {
                LabeledContent("When", value: Fmt.when(Date(timeIntervalSince1970: TimeInterval(r.startedAt) / 1000)))
                LabeledContent("Model", value: r.model ?? "—")
                LabeledContent("Status", value: r.status.map(String.init) ?? "—")
                if let e = r.error { LabeledContent("Error") { Text(e).foregroundStyle(.red) } }
                LabeledContent("Stop reason", value: r.stopReason ?? "—")
                LabeledContent("Thread", value: r.isSubagent ? "subagent \(r.agentId ?? "")" : "main")
                LabeledContent("Time to first token", value: Fmt.ms(r.ttfbMs.map(Double.init)))
                LabeledContent("Duration", value: Fmt.ms(r.durationMs.map(Double.init)))
            }
            Section("Tokens") {
                LabeledContent("Prompt (everything read)", value: Fmt.compact(r.promptTokens))
                LabeledContent("Uncached input", value: Fmt.compact(r.inputTokens))
                LabeledContent("Cache reads", value: Fmt.compact(r.cacheReadTokens))
                LabeledContent("Cache writes", value: "\(Fmt.compact(r.cacheWrite5mTokens)) for 5 min · \(Fmt.compact(r.cacheWrite1hTokens)) for an hour")
                LabeledContent("Output", value: Fmt.compact(r.outputTokens))
                LabeledContent("Thinking", value: Fmt.compact(r.thinkingTokens))
                LabeledContent("Cost", value: Fmt.usd(r.cost))
            }
            if !r.contextSplit.isEmpty {
                Section("Context window (estimate)") {
                    ShareBar(segments: r.contextSplit.map { name, v in
                        .init(key: name, label: name.replacingOccurrences(of: "_", with: " "), value: v,
                              color: ["system": Color.indigo, "tools": .cyan, "history": .yellow, "tool_results": .pink][name] ?? .gray)
                    })
                }
            }
            if !r.toolList.isEmpty {
                Section("Tools called, in order") {
                    ForEach(Array(r.toolList.enumerated()), id: \.offset) { i, name in
                        Label(name, systemImage: name == "Bash" ? "terminal" : "wrench.and.screwdriver")
                    }
                }
            }
            Section("Ids") {
                LabeledContent("Request", value: r.requestId ?? "—").font(.caption).textSelection(.enabled)
                LabeledContent("Session", value: r.sessionId ?? "—").font(.caption).textSelection(.enabled)
                if let p = r.project { LabeledContent("Project", value: p).font(.caption) }
            }
        }
        .navigationTitle("Request")
        .navigationBarTitleDisplayMode(.inline)
    }
}
