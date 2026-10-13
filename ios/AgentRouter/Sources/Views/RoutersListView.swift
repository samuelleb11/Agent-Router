import SwiftUI

/// The root: every router this phone knows, each with its pulse.
struct RoutersListView: View {
    @Environment(RouterStore.self) private var store
    @State private var adding = false
    @State private var path = NavigationPath()

    var body: some View {
        NavigationStack(path: $path) {
            List {
                ForEach(store.routers) { router in
                    NavigationLink(value: router) {
                        RouterRow(state: store.state(for: router))
                    }
                }
                .onDelete { store.remove(at: $0) }
            }
            .navigationTitle("Routers")
            .navigationDestination(for: Router.self) { router in
                RouterDetailView(state: store.state(for: router))
            }
            .navigationDestination(for: SessionsRoute.self) { route in
                if let state = store.state(id: route.routerId) { SessionsView(state: state) }
            }
            .navigationDestination(for: MachinesRoute.self) { route in
                if let state = store.state(id: route.routerId) { MachinesView(state: state) }
            }
            .navigationDestination(for: SessionRoute.self) { route in
                if let state = store.state(id: route.routerId) { SessionView(state: state, session: route.session) }
            }
            .navigationDestination(for: UsageRoute.self) { route in
                if let usage = store.usage(id: route.routerId, scope: route.scope) { UsageView(usage: usage) }
            }
            .navigationDestination(for: UsageListRoute.self) { route in
                if let usage = store.usage(id: route.routerId, scope: route.scope) {
                    switch route.kind {
                    case .projects: ProjectsView(usage: usage)
                    case .sessions: ProxySessionsView(usage: usage)
                    case .tools: ToolsView(usage: usage)
                    case .requests: RequestsView(usage: usage)
                    }
                }
            }
            .navigationDestination(for: RequestRoute.self) { route in
                RequestDetailView(r: route.request, accounts: store.state(id: route.routerId)?.accounts ?? [])
            }
            .navigationDestination(for: SearchRoute.self) { route in
                if let state = store.state(id: route.routerId) { SearchView(state: state) }
            }
            .navigationDestination(for: AccountsRoute.self) { route in
                if let state = store.state(id: route.routerId) { AccountsView(state: state) }
            }
            .navigationDestination(for: SettingsRoute.self) { route in
                if let state = store.state(id: route.routerId) { RouterSettingsView(state: state) }
            }
            .navigationDestination(for: LibraryRoute.self) { route in
                if let state = store.state(id: route.routerId) { LibraryView(state: state) }
            }
            .navigationDestination(for: LibraryItemRoute.self) { route in
                if let state = store.state(id: route.routerId) { LibraryItemView(state: state, kind: route.kind, name: route.name) }
            }
            .navigationDestination(for: SettingsPageRoute.self) { route in
                if let state = store.state(id: route.routerId) { SettingsPageView(state: state, page: route.page) }
            }
            .navigationDestination(for: SessionByIdRoute.self) { route in
                if let state = store.state(id: route.routerId) { SessionByIdView(state: state, sessionId: route.sessionId) }
            }
            // A tapped notification lands on the session it came from. The router is the
            // one whose id the phone registered with; if that no longer matches — the router
            // was removed and added again, say — and there is only one, it is that one.
            .onChange(of: store.pendingOpen, initial: true) { _, open in
                guard let open else { return }
                let router = store.routers.first { $0.id == open.routerId }
                    ?? (store.routers.count == 1 ? store.routers.first : nil)
                guard let router else { store.pendingOpen = nil; return }
                adding = false
                var p = NavigationPath()
                p.append(router)
                p.append(SessionsRoute(routerId: router.id))
                p.append(SessionByIdRoute(routerId: router.id, sessionId: open.sessionId))
                path = p
                store.pendingOpen = nil
            }
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button("Add router", systemImage: "plus") { adding = true }
                }
            }
            .overlay {
                if store.routers.isEmpty {
                    ContentUnavailableView {
                        Label("No routers", systemImage: "server.rack")
                    } description: {
                        Text("Add the Agent Router running on your Mac to see its quota and switch accounts.")
                    } actions: {
                        Button("Add router") { adding = true }
                            .buttonStyle(.borderedProminent)
                    }
                }
            }
            .sheet(isPresented: $adding) { RouterFormView(mode: .add) }
            .refreshable { await refreshAll() }
            .task(id: store.routers.map(\.id)) {
                while !Task.isCancelled {
                    await refreshAll()
                    do { try await Task.sleep(for: .seconds(15)) } catch { return }
                }
            }
        }
    }

    /// Every router at once. `RouterState` is main-actor isolated, so each child task
    /// simply hops there for its await; nothing here needs to be marked.
    private func refreshAll() async {
        let states = store.routers.map { store.state(for: $0) }
        await withTaskGroup(of: Void.self) { group in
            for s in states {
                group.addTask { await s.refresh(includeAccounts: false) }
            }
        }
    }
}

private struct RouterRow: View {
    let state: RouterState

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Circle()
                    .fill(color)
                    .frame(width: 8, height: 8)
                    .accessibilityLabel(statusText)
                Text(state.router.name)
                    .font(.headline)
                Spacer()
                Text(state.router.address)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .monospaced()
                    // A hosted router's URL is long: one line, keeping its scheme and port.
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            if let failure = state.failure {
                Text(failure.errorDescription ?? "Error")
                    .font(.caption)
                    .foregroundStyle(.red)
                    .lineLimit(2)
            } else if !state.activeAccounts.isEmpty {
                // Each provider's serving account, by its mark, with its own two windows.
                ForEach(state.activeAccounts) { a in
                    HStack(spacing: 6) {
                        ProviderMark(provider: a.providerId, form: .alone)
                        Text(a.title)
                            .lineLimit(1)
                    }
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    HStack(spacing: 12) {
                        MiniGauge(label: WindowKey.fiveHour.short, used: a.fiveHour)
                        MiniGauge(label: a.secondKey.short, used: a.second)
                    }
                }
                if let c = state.sessionCounts {
                    // Waiting on you is the one worth colouring: it is the one that needs
                    // a thumb.
                    if let live = state.liveSummary {
                        Label(live, systemImage: "bubble.left.and.bubble.right")
                            .font(.caption)
                            .foregroundStyle(c.waiting > 0 ? .orange : .green)
                    }
                } else if state.activeSessions > 0 {
                    Label("\(state.activeSessions) active session\(state.activeSessions == 1 ? "" : "s")",
                          systemImage: "bubble.left.and.bubble.right")
                        .font(.caption)
                        .foregroundStyle(.green)
                }
            } else {
                Text(statusText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 2)
        .accessibilityIdentifier("router-row")
    }

    private var color: Color {
        if state.failure != nil { return .red }
        return state.isReachable ? .green : .gray
    }

    private var statusText: String {
        if state.failure != nil { return "unreachable" }
        return state.isReachable ? "connected" : "checking…"
    }
}

/// A window's fill at a glance: the same 70/90 thresholds as the dashboard.
private struct MiniGauge: View {
    let label: String
    let used: Forecast.Window?

    var body: some View {
        let live = used.map { !$0.isExpired } ?? false
        let value = live ? (used?.used ?? 0) : 0
        Gauge(value: value, in: 0...1) {
            Text(label)
        } currentValueLabel: {
            Text(live ? Fmt.percent(value) : "—")
        }
        .gaugeStyle(.accessoryLinearCapacity)
        .tint(QuotaColor.tint(value, live: live))
        .font(.caption2)
        .frame(maxWidth: 140)
        .accessibilityLabel("\(label) window")
        .accessibilityValue(live ? Fmt.percent(value) : "no reading")
    }
}

enum QuotaColor {
    static func tint(_ fraction: Double, live: Bool) -> Color {
        guard live else { return .gray }
        if fraction >= 0.9 { return .red }
        if fraction >= 0.7 { return .orange }
        return .accentColor
    }
}
