import SwiftUI

/// Every session the router can see, grouped the way the web view groups them.
struct SessionsView: View {
    let state: RouterState
    @State private var showOlder = false
    @State private var showArchived = false
    @State private var notice: String?

    var body: some View {
        let groups = SessionGroups.build(state.sessions)
        List {
            if let t = state.shim?.newestUnwrapped {
                Section {
                    Label("\(t.ext) is not wrapped — sessions it starts are view-only until the wrapper is installed from the dashboard on the Mac.",
                          systemImage: "exclamationmark.triangle")
                        .font(.footnote)
                        .foregroundStyle(.orange)
                }
            }
            if let notice {
                Section { Text(notice).font(.footnote).foregroundStyle(.red) }
            }
            ForEach(groups.recent) { p in
                Section {
                    ForEach(p.sessions) { row($0) }
                } header: {
                    ProjectHeader(project: p)
                }
            }
            if !groups.older.isEmpty {
                Section {
                    DisclosureGroup(isExpanded: $showOlder) {
                        ForEach(groups.older) { p in
                            ForEach(p.sessions) { row($0) }
                        }
                    } label: {
                        let n = groups.older.reduce(0) { $0 + $1.sessions.count }
                        Text("\(n) older session\(n == 1 ? "" : "s")")
                            .foregroundStyle(.secondary)
                    }
                }
            }
            if !groups.archived.isEmpty {
                Section {
                    DisclosureGroup(isExpanded: $showArchived) {
                        ForEach(groups.archived) { row($0) }
                    } label: {
                        Text("\(groups.archived.count) archived").foregroundStyle(.secondary)
                    }
                }
            }
        }
        .navigationTitle("Sessions")
        .overlay {
            if state.sessions.isEmpty {
                if let why = state.sessionsFailure {
                    ContentUnavailableView {
                        Label("Couldn't list sessions", systemImage: "exclamationmark.triangle")
                    } description: {
                        Text(why)
                    }
                    .accessibilityIdentifier("sessions-error")
                } else {
                    ContentUnavailableView("No sessions", systemImage: "bubble.left.and.bubble.right",
                                           description: Text("Nothing has run on this Mac in the last week."))
                }
            }
        }
        .refreshable { await state.refreshSessions() }
        .task {
            while !Task.isCancelled {
                await state.refreshSessions()
                do { try await Task.sleep(for: .seconds(5)) } catch { return }
            }
        }
    }

    private func row(_ s: LiveSession) -> some View {
        NavigationLink(value: SessionRoute(routerId: state.router.id, session: s)) { SessionRowView(session: s) }
            .swipeActions(edge: .trailing) {
                Button(s.archived ? "Restore" : "Archive",
                       systemImage: s.archived ? "tray.and.arrow.up" : "archivebox") {
                    Task { notice = await state.archive(s, !s.archived) }
                }
                .tint(s.archived ? .blue : .orange)
            }
    }
}

private struct ProjectHeader: View {
    let project: SessionGroups.Project
    var body: some View {
        HStack {
            StateDot(state: project.live ? "working" : "idle", active: project.live)
            Text(project.name)
            Spacer()
            Text(Fmt.ago(project.newest))
        }
    }
}

struct SessionRowView: View {
    let session: LiveSession
    var body: some View {
        HStack(spacing: 10) {
            StateDot(state: session.state, active: session.active)
            VStack(alignment: .leading, spacing: 2) {
                Text(session.label)
                    .lineLimit(2)
                HStack(spacing: 5) {
                    Text(session.shortId).monospaced()
                    if let b = session.gitBranch { Text("·"); Text(b) }
                    Text("·")
                    Text(Fmt.ago(session.lastActivity))
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            }
            Spacer(minLength: 6)
            Text(session.wrapped ? "live" : "view")
                .font(.caption2.weight(.medium))
                .padding(.horizontal, 7)
                .padding(.vertical, 2)
                .background(Capsule().fill(session.wrapped ? Color.green.opacity(0.15) : Color.secondary.opacity(0.12)))
                .foregroundStyle(session.wrapped ? .green : .secondary)
                .accessibilityLabel(session.wrapped ? "Can take a message" : "Read only")
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("session-row")
    }
}

struct StateDot: View {
    let state: String?
    var active = false
    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 9, height: 9)
            .overlay { if active { Circle().stroke(color.opacity(0.35), lineWidth: 3) } }
            .accessibilityLabel(state ?? "idle")
    }
    private var color: Color {
        switch StateColor.of(state) {
        case .green: .green
        case .orange: .orange
        case .yellow: .yellow
        case .red: .red
        case .purple: .purple
        case .gray: .gray
        }
    }
}


/// A session known only by id — from a notification — looked up, then shown.
struct SessionByIdView: View {
    let state: RouterState
    let sessionId: String
    @State private var found: LiveSession?

    var body: some View {
        ZStack {
            if let found {
                SessionView(state: state, session: found)
            } else {
                ContentUnavailableView {
                    Label("Finding the session", systemImage: "magnifyingglass")
                } description: {
                    Text(String(sessionId.prefix(8)))
                }
            }
        }
        .task {
            while found == nil, !Task.isCancelled {
                await state.refreshSessions()
                found = state.sessions.first { $0.sessionId == sessionId }
                if found == nil { do { try await Task.sleep(for: .seconds(2)) } catch { return } }
            }
        }
    }
}
