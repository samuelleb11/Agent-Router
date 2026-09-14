import SwiftUI

/// One session: its feed as it happens, and a way to type into it.
struct SessionView: View {
    let state: RouterState
    /// The session this page shows now — switchable in place from the tab strip, as
    /// the web view switches between a project's sessions.
    @State private var session: LiveSession
    @State private var feed: LiveFeed?
    @State private var draft = ""
    @State private var following = true
    @State private var notice: String?
    @State private var askModel = false
    @State private var customModel = ""
    @State private var askAside = false
    @State private var asideText = ""
    @FocusState private var composing: Bool

    init(state: RouterState, session: LiveSession) {
        self.state = state
        _session = State(initialValue: session)
    }

    var body: some View {
        // A ZStack, not a Group: a `.task` on a conditional child restarts when the
        // branch flips, which here would start the stream twice.
        ZStack {
            if let feed { content(feed) } else { ProgressView() }
        }
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                VStack(spacing: 1) {
                    Text(session.projectName).font(.headline)
                    if let st = feed?.status {
                        HStack(spacing: 5) {
                            StateDot(state: st.state, active: st.isBusy)
                            Text(statusLine(st))
                        }
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    } else if feed?.connected == false {
                        Text("connecting…").font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    if let feed {
                        let opts = feed.options
                        let model = (feed.stats?.model ?? "").replacingOccurrences(of: "claude-", with: "")
                        Menu {
                            ForEach(opts?.models ?? [], id: \.self) { m in
                                Button { Task { await feed.setModel(m) } } label: {
                                    if m == model { Label(m, systemImage: "checkmark") } else { Text(m) }
                                }
                            }
                            Button("Other…") { customModel = ""; askModel = true }
                        } label: {
                            Label("Model" + (model.isEmpty ? "" : " · \(model)"), systemImage: "cpu")
                        }
                        .disabled(!session.wrapped || feed.switching)
                        Menu {
                            ForEach(opts?.efforts ?? ["low", "medium", "high", "xhigh", "max"], id: \.self) { e in
                                Button { Task { await feed.setEffort(e) } } label: {
                                    if e == feed.stats?.effort { Label(e, systemImage: "checkmark") } else { Text(e) }
                                }
                            }
                        } label: {
                            Label("Effort" + (feed.stats?.effort.map { " · \($0)" } ?? ""), systemImage: "gauge.with.needle")
                        }
                        .disabled(!session.wrapped || feed.switching)
                        if !session.wrapped {
                            Text("Read-only session: model and effort cannot be changed from here.")
                        }
                        Button("Ask on the side…", systemImage: "text.bubble") { asideText = draft; askAside = true }
                            .disabled(feed.switching)
                        Divider()
                    }
                    Button(session.archived ? "Restore session" : "Archive session",
                           systemImage: session.archived ? "tray.and.arrow.up" : "archivebox") {
                        Task { await putAway(!session.archived) }
                    }
                    Button("Copy session id", systemImage: "doc.on.doc") {
                        UIPasteboard.general.string = session.sessionId
                    }
                } label: {
                    Label("More", systemImage: "ellipsis.circle")
                }
            }
        }
        .alert("Side question", isPresented: $askAside) {
            TextField("What do you want to know?", text: $asideText)
            Button("Ask") {
                let q = asideText.trimmingCharacters(in: .whitespacesAndNewlines)
                if !q.isEmpty, let feed { draft = ""; following = true; Task { await feed.aside(q) } }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Answered from this session's context without interrupting it — what /btw does. The answer appears here only.")
        }
        .alert("Model", isPresented: $askModel) {
            TextField("opus, sonnet[1m], or a full id", text: $customModel)
                .textInputAutocapitalization(.never).autocorrectionDisabled()
            Button("Switch") {
                let m = customModel.trimmingCharacters(in: .whitespaces)
                if !m.isEmpty, let feed { Task { await feed.setModel(m) } }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("An alias the CLI knows, or a full model id. Takes effect on the next turn.")
        }
        .task(id: session.sessionId) {
            let f = LiveFeed(session: session, client: state.liveClient)
            feed = f
            async let facts: Void = f.loadFacts()
            await f.run()
            _ = await facts
        }
    }

    /// Every session of this project that is current, plus this one — creation order,
    /// so a tab keeps its place for the life of the session.
    private var siblings: [LiveSession] {
        let now = Date()
        return state.sessions
            .filter { $0.projectName == session.projectName }
            .filter { $0.sessionId == session.sessionId || (SessionGroups.isCurrent($0, now: now) && !$0.archived) }
            .sorted { ($0.createdMs ?? $0.lastActivityMs) < ($1.createdMs ?? $1.lastActivityMs) }
    }

    /// Put this session away, or bring it back, and say so — it comes back on its own
    /// the moment it shows life again.
    private func putAway(_ archived: Bool) async {
        if let why = await state.archive(session, archived) {
            feed?.notice = LiveFeed.Notice(text: why, ok: false)
        } else if let updated = state.sessions.first(where: { $0.sessionId == session.sessionId }) {
            session = updated
            feed?.notice = LiveFeed.Notice(text: archived ? "Put away. It comes back when it shows life." : "Back in the list.", ok: true)
        }
    }

    private func statusLine(_ st: LiveStatus) -> String {
        var line = st.quietMs > 4000 ? "\(st.detail) · \(Fmt.duration(TimeInterval(st.quietMs) / 1000))" : st.detail
        if let q = st.queued, q > 0 { line += " · \(q) queued" }
        return line
    }

    @ViewBuilder private func content(_ feed: LiveFeed) -> some View {
        VStack(spacing: 0) {
            let tabs = siblings
            if tabs.count > 1 {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(tabs) { s in
                            let on = s.sessionId == session.sessionId
                            Button {
                                if !on { draft = ""; session = s }
                            } label: {
                                HStack(spacing: 5) {
                                    StateDot(state: s.state, active: s.active)
                                    Text(s.label).lineLimit(1)
                                }
                                .font(.caption)
                                .padding(.horizontal, 10)
                                .padding(.vertical, 5)
                                .background(Capsule().fill(on ? Color.accentColor.opacity(0.18) : Color(.secondarySystemBackground)))
                            }
                            .buttonStyle(.plain)
                            .accessibilityAddTraits(on ? .isSelected : [])
                        }
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                }
                .background(.bar)
                .accessibilityLabel("Sessions in this project")
            }
            if let t = feed.stats { FactsStrip(stats: t, project: feed.project) }
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 10) {
                        ForEach(feed.rows) { row in
                            FeedRowView(row: row, feed: feed)
                                .id(row.id)
                        }
                        Color.clear.frame(height: 1).id("bottom")
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                }
                .scrollDismissesKeyboard(.interactively)
                // Open on the newest message, not the oldest.
                .defaultScrollAnchor(.bottom, for: .initialOffset)
                // Following is a state, not a per-append test: once the reader scrolls
                // up to read something, new rows must not yank them back down. Only a
                // scroll that has *settled* decides it — the backfill arrives as a burst
                // that grows the content under a zero offset, and judging "at the
                // bottom" during that growth stopped the feed following before the
                // first pin ever happened.
                .onScrollPhaseChange { _, phase, context in
                    if phase == .idle {
                        let g = context.geometry
                        following = g.contentOffset.y + g.containerSize.height >= g.contentSize.height - 140
                    }
                }
                .onChange(of: feed.rows.count) {
                    if following { proxy.scrollTo("bottom", anchor: .bottom) }
                }
                .onChange(of: composing) {
                    // The keyboard takes half the screen; keep the newest row in view.
                    if composing, following { proxy.scrollTo("bottom", anchor: .bottom) }
                }
            }
            if let n = feed.notice {
                HStack {
                    Text(n.text).font(.footnote)
                    Spacer()
                    Button("Dismiss", systemImage: "xmark") { feed.notice = nil }
                        .labelStyle(.iconOnly)
                        .font(.caption)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(n.ok ? Color.green.opacity(0.12) : Color.orange.opacity(0.15))
            }
            composer(feed)
        }
    }

    /// What this person already said in this session, newest first — the web view's
    /// ArrowUp history, as a menu.
    private func recent(_ feed: LiveFeed) -> [String] {
        var seen = Set<String>()
        return feed.rows.reversed().compactMap { r -> String? in
            guard r.item.role == "user", r.item.kind == "text", !r.item.sidechain, !r.pending,
                  let t = r.item.text, !t.isEmpty, seen.insert(t).inserted else { return nil }
            return t
        }
        .prefix(8).map { $0 }
    }

    private func composer(_ feed: LiveFeed) -> some View {
        HStack(alignment: .bottom, spacing: 8) {
            let history = recent(feed)
            if !history.isEmpty {
                Menu {
                    ForEach(history, id: \.self) { t in
                        Button(String(t.prefix(60)) + (t.count > 60 ? "…" : "")) { draft = t }
                    }
                } label: {
                    Image(systemName: "clock.arrow.circlepath").font(.title3)
                }
                .accessibilityLabel("Recent messages")
            }
            TextField(session.wrapped ? "Message" : "Message (read-only session)", text: $draft, axis: .vertical)
                .lineLimit(1...6)
                .focused($composing)
                .accessibilityIdentifier("composer")
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(RoundedRectangle(cornerRadius: 18).fill(Color(.secondarySystemBackground)))
                .onSubmit { submit(feed) }
            // Stop beside Send, never instead of it: a message sent while a turn runs is
            // queued by the session and taken up when the turn ends.
            if feed.canStop {
                Button {
                    Task { await feed.interrupt() }
                } label: {
                    Image(systemName: "stop.circle.fill").font(.system(size: 30))
                }
                .tint(.red)
                .disabled(feed.stopping)
                .accessibilityLabel("Stop")
            }
            Button {
                submit(feed)
            } label: {
                Image(systemName: "arrow.up.circle.fill").font(.system(size: 30))
            }
            .disabled(feed.sending || draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            .accessibilityLabel(feed.canStop ? "Queue" : "Send")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.bar)
    }

    private func submit(_ feed: LiveFeed) {
        let text = draft
        draft = ""
        following = true
        Task { await feed.send(text) }
    }
}

private struct FactsStrip: View {
    let stats: SessionStats
    let project: ProjectStats?
    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                Text((stats.model ?? "unknown").replacingOccurrences(of: "claude-", with: "")).bold()
                if let e = stats.effort { Text("·"); Text("effort \(e)") }
                Text("·"); Text("\(Fmt.compact(Double(stats.total))) tokens")
                Text("·"); Text("\(stats.turns) turn\(stats.turns == 1 ? "" : "s")")
                if let c = stats.cost { Text("·"); Text(Fmt.usd(c)).bold() }
                if let p = project, p.sessions > 1 {
                    Text("·")
                    Text("project \(p.sessions) sessions \(Fmt.compact(Double(p.total))) tokens" + (p.cost.map { " \(Fmt.usd($0))" } ?? ""))
                        .foregroundStyle(.secondary)
                }
            }
            .font(.caption)
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
        }
        .background(.bar)
    }
}
