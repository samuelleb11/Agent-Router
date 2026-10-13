import SwiftUI

/// Every session the router can see, grouped the way the web view groups them — machine by
/// machine when a hosted router merges more than one, with a tab for each machine over them.
struct SessionsView: View {
    let state: RouterState
    /// The older and archived folds that are open, by machine: one of each, as there always
    /// was, when the list names no machines. A machine's folds are the same on All and on
    /// its own tab.
    @State private var unfolded: Set<String> = []
    @State private var notice: String?
    /// Said when the chosen machine left the list and the list went back to All.
    @State private var tabNotice: String?
    @State private var starting = false

    var body: some View {
        let all = SessionGroups.byMachine(state.sessions)
        let tabs = SessionTabs.tabs(all)
        let machines = SessionTabs.parts(all, showing: state.sessionsTab)
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
            if let tabNotice {
                Section {
                    Text(tabNotice)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("machine-tab-notice")
                }
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
        // Inline: a large title over the pinned machine tabs left a tall empty band above them.
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button(SessionStart.title, systemImage: "plus") { starting = true }
                    .accessibilityIdentifier("start-session")
            }
        }
        .sheet(isPresented: $starting) { StartSessionSheet(state: state) }
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
                    // running the agent, and until one connects it lists nothing. How to connect
                    // one is on the Machines screen, the commands' one home (SE-C187).
                    ContentUnavailableView {
                        Label("No sessions", systemImage: "bubble.left.and.bubble.right")
                    } description: {
                        Text(Machines.agentless)
                    } actions: {
                        NavigationLink(Machines.connect, value: MachinesRoute(routerId: state.router.id))
                            .accessibilityIdentifier("connect-a-machine")
                    }
                } else {
                    ContentUnavailableView("No sessions", systemImage: "bubble.left.and.bubble.right",
                                           description: Text("Nothing has run on this Mac in the last week."))
                }
            }
        }
        // Over a list of several machines, a tab for each: pinned above the list, and only
        // there when there is more than one machine to choose between.
        .safeAreaInset(edge: .top, spacing: 0) {
            if !tabs.isEmpty {
                MachineTabStrip(tabs: tabs, selected: state.sessionsTab) { machine in
                    tabNotice = nil
                    state.sessionsTab = machine
                }
            }
        }
        // A chosen machine that has left the list — gone offline, say — gives way to All,
        // and the list says so once rather than show nothing.
        .onChange(of: SessionTabs.isUnlisted(state.sessionsTab, rows: state.sessions), initial: true) { _, gone in
            guard gone else { return }
            state.sessionsTab = nil
            tabNotice = SessionTabs.unlisted
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
}

/// The tabs over a list that holds more than one machine's sessions: All, then each machine,
/// each with what is at work there and what waits on you, and the attention dot when
/// something does. One row that scrolls sideways rather than squeeze, as a session's tabs do.
private struct MachineTabStrip: View {
    let tabs: [SessionTabs.Tab]
    /// The chosen machine's part; nil is All.
    let selected: String?
    let choose: (String?) -> Void

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(tabs) { t in
                    let on = t.machine == selected
                    Button {
                        if !on { choose(t.machine) }
                    } label: {
                        HStack(spacing: 5) {
                            if t.waiting {
                                Circle()
                                    .fill(StateColor.of(tone: .attention).color)
                                    .frame(width: 7, height: 7)
                            }
                            // A label is whatever the machine's agent was given: a long one is
                            // cut short, so one tab never takes the whole row.
                            Text(t.label)
                                .lineLimit(1)
                                .frame(maxWidth: 160)
                            if !t.counts.isEmpty {
                                Text(t.counts)
                                    .foregroundStyle(t.waiting ? Color.orange : Color.secondary)
                                    .lineLimit(1)
                            }
                        }
                        .font(.caption)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 5)
                        .background(Capsule().fill(on ? Color.accentColor.opacity(0.18) : Color(.secondarySystemBackground)))
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(t.spoken)
                    .accessibilityAddTraits(on ? .isSelected : [])
                    .accessibilityIdentifier("machine-tab")
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
        }
        .background(.bar)
        .accessibilityLabel("Machines")
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
                    if let name = session.clientName {
                        if let p = session.markProvider { ProviderMark(provider: p, form: .words(name)) } else { Text(name) }
                        Text("·")
                    }
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
            Text(session.canControl ? "live" : "view")
                .font(.caption2.weight(.medium))
                .padding(.horizontal, 7)
                .padding(.vertical, 2)
                .background(Capsule().fill(session.canControl ? Color.green.opacity(0.15) : Color.secondary.opacity(0.12)))
                .foregroundStyle(session.canControl ? .green : .secondary)
                .accessibilityLabel(session.canControl ? "Can take a message" : "Read only")
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
