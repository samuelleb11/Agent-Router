import SwiftUI

/// Search everything this router has recorded.
///
/// Per router, because the index belongs to one: each router has its own store, and
/// there is no federated view to ask.
struct SearchView: View {
    let state: RouterState

    @State private var query = ""
    @State private var kind: Kind = .all
    @State private var results: SearchResults?
    @State private var failure: String?
    @State private var searching = false

    /// The facets the router indexes. `all` sends no `kind` at all, which is what the
    /// endpoint wants for "don't narrow".
    enum Kind: String, CaseIterable, Identifiable {
        case all, text, toolUse, toolResult, thinking
        var id: Self { self }
        var label: String {
            switch self {
            case .all: "All"
            case .text: "Messages"
            case .toolUse: "Tool calls"
            case .toolResult: "Results"
            case .thinking: "Thinking"
            }
        }
        var param: String? {
            switch self {
            case .all: nil
            case .text: "text"
            case .toolUse: "tool_use"
            case .toolResult: "tool_result"
            case .thinking: "thinking"
            }
        }
    }

    var body: some View {
        List {
            Section {
                Picker("Kind", selection: $kind) {
                    ForEach(Kind.allCases) { Text($0.label).tag($0) }
                }
                .pickerStyle(.segmented)
                .accessibilityIdentifier("search-kind")
            }

            if let failure {
                Section {
                    Label(failure, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .accessibilityIdentifier("search-error")
                }
            }

            // Only worth saying when the router ran something other than what was typed.
            if let e = results?.expression, !e.isEmpty, e != trimmedQuery {
                Section {
                    Text("Searched for \(e)").font(.caption).foregroundStyle(.secondary)
                }
            }

            if let sessions = results?.sessions, !sessions.isEmpty {
                Section("Sessions that mention it") {
                    ForEach(sessions) { s in
                        NavigationLink(value: SessionByIdRoute(routerId: state.router.id, sessionId: s.sessionId)) {
                            sessionRow(s)
                        }
                    }
                }
            }

            if let hits = results?.hits, !hits.isEmpty {
                Section(hits.count == 1 ? "1 match" : "\(hits.count) matches") {
                    ForEach(hits) { hit in
                        NavigationLink(value: SessionByIdRoute(routerId: state.router.id, sessionId: hit.sessionId)) {
                            hitRow(hit)
                        }
                    }
                }
            }
        }
        .navigationTitle("Search")
        .searchable(text: $query, prompt: "A word, a phrase, an identifier")
        .autocorrectionDisabled()
        .textInputAutocapitalization(.never)
        .overlay { emptyState }
        // SwiftUI cancels and restarts this on every keystroke and on a change of facet,
        // so the sleep at the top of it *is* the debounce — no timer, no sequence guard.
        .task(id: "\(query)|\(kind.rawValue)") {
            let q = trimmedQuery
            guard !q.isEmpty else {
                results = nil
                failure = nil
                return
            }
            do { try await Task.sleep(for: .milliseconds(220)) } catch { return }
            await run(q)
        }
    }

    private var trimmedQuery: String {
        query.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    @ViewBuilder private var emptyState: some View {
        if trimmedQuery.isEmpty {
            ContentUnavailableView(
                "Search the transcripts",
                systemImage: "magnifyingglass",
                description: Text("Every message, tool call and result this router has recorded.")
            )
        } else if searching && results == nil {
            ProgressView()
        } else if let r = results, r.hits.isEmpty, failure == nil {
            ContentUnavailableView.search(text: trimmedQuery)
        }
    }

    @ViewBuilder private func sessionRow(_ s: SearchSessionRow) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Text(s.sessionLabel ?? s.shortId).font(.headline).lineLimit(1)
                Spacer()
                Text(Fmt.count(s.hits)).font(.subheadline).monospacedDigit().foregroundStyle(.secondary)
            }
            Text(s.projectName).font(.caption).foregroundStyle(.secondary)
            if let d = Fmt.date(ms: s.lastAt) {
                Text("\(Fmt.count(s.blocks)) messages · \(Fmt.ago(d))")
                    .font(.caption2).foregroundStyle(.tertiary)
            }
        }
        .padding(.vertical, 2)
    }

    @ViewBuilder private func hitRow(_ hit: SearchHit) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Text(hit.kindName)
                    .font(.caption2)
                    .padding(.horizontal, 6).padding(.vertical, 1)
                    .background(Capsule().fill(Color.secondary.opacity(0.15)))
                Text(hit.role).font(.caption2).foregroundStyle(.secondary)
                Spacer()
                if let d = Fmt.date(ms: hit.at) {
                    Text(Fmt.ago(d)).font(.caption2).foregroundStyle(.tertiary)
                }
            }
            Text(highlighted(hit.snippet)).font(.caption).lineLimit(4)
            Text("\(hit.projectName) · \(hit.shortSession)")
                .font(.caption2).foregroundStyle(.tertiary).lineLimit(1)
        }
        .padding(.vertical, 3)
    }

    /// The router marks each match with \u{2039}…\u{203A}.
    ///
    /// Split on the pair rather than substituting into it: a stray marker in the text
    /// itself can then only shift which run is emphasised, where substitution could
    /// leave a run open and take the rest of the snippet with it.
    private func highlighted(_ snippet: String) -> AttributedString {
        var out = AttributedString()
        let parts = snippet.components(separatedBy: CharacterSet(charactersIn: "\u{2039}\u{203A}"))
        for (i, part) in parts.enumerated() {
            var piece = AttributedString(part)
            if i % 2 == 1 {
                piece.inlinePresentationIntent = .stronglyEmphasized
                piece.foregroundColor = .accentColor
            }
            out.append(piece)
        }
        return out
    }

    private func run(_ q: String) async {
        guard let client = state.liveClient else {
            failure = "This router has no usable address."
            return
        }
        searching = true
        defer { searching = false }
        do {
            // Smaller than the dashboard's sixty: this is a phone, often on cellular,
            // and the rows below the first screenful are rarely the ones wanted.
            let r: SearchResults = try await client.get(
                "/api/search",
                query: ["q": q, "kind": kind.param, "limit": "25"]
            )
            results = r
            failure = nil
        } catch {
            // A cancelled request is the next keystroke arriving, not a failure.
            if error is CancellationError || Task.isCancelled { return }
            failure = LiveFeed.describe(error)
        }
    }
}
