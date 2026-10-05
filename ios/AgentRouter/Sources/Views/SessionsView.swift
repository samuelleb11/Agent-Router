import SwiftUI

/// Every session the router can see, grouped the way the web view groups them — machine by
/// machine when a hosted router merges more than one.
struct SessionsView: View {
    let state: RouterState
    /// The older and archived folds that are open, by machine: one of each, as there always
    /// was, when the list names no machines.
    @State private var unfolded: Set<String> = []
    @State private var notice: String?

    var body: some View {
        let machines = SessionGroups.byMachine(state.sessions)
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
            // A listing that failed keeps the last one on screen, and its dots would go on
            // claiming "working" or "waiting" long after either stopped being true.
            if let why = state.sessionsFailure, !state.sessions.isEmpty {
                Section {
                    Label {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(why)
                            Text("The list below is the last one the router sent.")
                                .foregroundStyle(.secondary)
                        }
                    } icon: {
                        Image(systemName: "wifi.exclamationmark")
                    }
                    .font(.footnote)
                    .foregroundStyle(.orange)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("sessions-stale")
                }
            }
            ForEach(machines) { m in
                ForEach(m.groups.recent) { p in
                    Section {
                        ForEach(p.sessions) { row($0) }
                    } header: {
                        // A machine's heading rides on its first section, right over what it names.
                        if let name = m.name, p.id == m.groups.recent.first?.id {
                            VStack(alignment: .leading, spacing: 10) {
                                MachineHeader(name: name)
                                ProjectHeader(project: p)
                            }
                        } else {
                            ProjectHeader(project: p)
                        }
                    }
                }
                if !m.groups.older.isEmpty {
                    section(heading: m.groups.recent.isEmpty ? m.name : nil) {
                        DisclosureGroup(isExpanded: expanded("older", m)) {
                            ForEach(m.groups.older) { p in
                                ForEach(p.sessions) { row($0) }
                            }
                        } label: {
                            let n = m.groups.older.reduce(0) { $0 + $1.sessions.count }
                            Text("\(n) older session\(n == 1 ? "" : "s")")
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                if !m.groups.archived.isEmpty {
                    section(heading: m.groups.recent.isEmpty && m.groups.older.isEmpty ? m.name : nil) {
                        DisclosureGroup(isExpanded: expanded("archived", m)) {
                            ForEach(m.groups.archived) { row($0) }
                        } label: {
                            Text("\(m.groups.archived.count) archived").foregroundStyle(.secondary)
                        }
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
                } else if state.auth?.isHosted == true {
                    // A hosted router runs no sessions of its own: they come from the machines
                    // running the agent, and until one connects it lists nothing.
                    ContentUnavailableView("No sessions", systemImage: "bubble.left.and.bubble.right",
                                           description: Text("Sessions appear here once a machine runs the Agent Router agent:\n`agent-router agent install --server \(agentServer)`"))
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

    /// Whether one of a machine's folds is open.
    private func expanded(_ fold: String, _ machine: SessionGroups.Machine) -> Binding<Bool> {
        let key = fold + ":" + machine.id
        return Binding(get: { unfolded.contains(key) },
                       set: { open in if open { unfolded.insert(key) } else { unfolded.remove(key) } })
    }

    /// A fold's section: under the machine's heading when nothing above it carries that
    /// heading, and otherwise exactly the plain section the list always drew.
    @ViewBuilder
    private func section<Content: View>(heading: String?, @ViewBuilder _ content: () -> Content) -> some View {
        if let heading {
            Section { content() } header: { MachineHeader(name: heading) }
        } else {
            Section { content() }
        }
    }

    /// What a machine's agent connects to: the address the router gives for itself, or the
    /// one this phone reaches it at.
    private var agentServer: String {
        if let url = state.auth?.publicURL?.trimmingCharacters(in: .whitespaces), !url.isEmpty { return url }
        return state.router.origin
    }
}

/// The machine a part of the list runs on, over that part, when the sessions come from more
/// than one.
private struct MachineHeader: View {
    let name: String
    var body: some View {
        Label(name, systemImage: "desktopcomputer")
            .font(.title3.weight(.semibold))
            // The colour itself: `.primary` in a header is the header's own gray.
            .foregroundStyle(Color.primary)
            .textCase(nil)
            .accessibilityAddTraits(.isHeader)
            .accessibilityIdentifier("machine-header")
    }
}

private struct ProjectHeader: View {
    let project: SessionGroups.Project
    var body: some View {
        HStack {
            StateDot(look: project.look)
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
            StateDot(look: session.look)
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
                if let line = session.doing {
                    Text(line)
                        .font(.caption)
                        .foregroundStyle(session.needsYou ? .orange : session.activity == .failed ? .red : .secondary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 6)
            // An ended session is not wrapped any more, so it reads "view", as on the web view
            // and the other phone; its line already says that it ended, and when.
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
    let look: StateLook
    var body: some View {
        let color = look.color.color
        ZStack {
            if look.hollow {
                // Ended: a ring with nothing in it, so a closed session never reads as
                // one that is merely idle.
                Circle().strokeBorder(color, lineWidth: 1.5)
            } else {
                Circle().fill(color)
            }
        }
        .frame(width: 9, height: 9)
        .overlay { if look.halo { Circle().stroke(color.opacity(0.35), lineWidth: 3) } }
        .accessibilityLabel(look.label)
    }
}

extension StateColor.ColorName {
    var color: Color {
        switch self {
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
