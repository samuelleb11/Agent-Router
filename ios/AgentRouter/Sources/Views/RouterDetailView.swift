import SwiftUI

/// One router: what it is serving with, how much of the quota is left and when it
/// runs out, every account it holds, and the switch between them.
struct RouterDetailView: View {
    let state: RouterState
    @Environment(RouterStore.self) private var store
    @State private var editing = false

    var body: some View {
        List {
            if let failure = state.failure {
                Section {
                    Label(failure.errorDescription ?? "Error", systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .accessibilityIdentifier("router-error")
                    if failure == .unauthorized || failure == .noPasswordSet {
                        Button("Edit router") { editing = true }
                    }
                }
            }
            activeSection
            sessionsSection
            if let a = state.activeAccount {
                Section {
                    QuotaRow(title: "5-hour window", window: a.fiveHour)
                    QuotaRow(title: "7-day window", window: a.sevenDay)
                } header: {
                    Text("Quota — \(a.title)")
                } footer: {
                    if let q = state.quota, let at = Fmt.date(ms: q.seenAt) {
                        // Say where the number came from: a response header is only as fresh as
                        // the last request; the usage endpoint is current even on an idle Mac.
                        Text(q.source == "response-headers" ? "From the last request, \(Fmt.ago(at))." : "Checked \(Fmt.ago(at)).")
                    }
                }
                burnDownSection(a)
            }
            accountsSection
            totalsSection
            routerSection
        }
        .navigationTitle(state.router.name)
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                accountMenu
                Button("Edit", systemImage: "pencil") { editing = true }
            }
        }
        .refreshable { await state.refresh(includeAccounts: true) }
        .task { await state.poll() }
        .sheet(isPresented: $editing) { RouterFormView(mode: .edit(state.router)) }
    }

    // MARK: sections

    @ViewBuilder private var activeSection: some View {
        Section("Serving with") {
            if let a = state.activeAccount {
                LabeledContent {
                    Text(a.plan ?? "")
                        .foregroundStyle(.secondary)
                } label: {
                    Label(a.title, systemImage: "person.crop.circle.fill")
                }
            } else if state.forecast == nil {
                Text(state.failure == nil ? "Connecting…" : "No data")
                    .foregroundStyle(.secondary)
            } else {
                Text("No active account").foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder private var sessionsSection: some View {
        Section {
            NavigationLink(value: SessionsRoute(routerId: state.router.id)) {
                LabeledContent {
                    Text(sessionsSummary).foregroundStyle(.secondary)
                } label: {
                    Label("Live sessions", systemImage: "bubble.left.and.bubble.right")
                        .accessibilityIdentifier("live-sessions")
                }
            }
            // A hosted router's sessions come from the machines connected to it; a local
            // router takes none.
            if state.auth?.isHosted == true {
                NavigationLink(value: MachinesRoute(routerId: state.router.id)) {
                    Label("Machines", systemImage: "desktopcomputer")
                        .accessibilityIdentifier("machines")
                }
            }
            NavigationLink(value: UsageRoute(routerId: state.router.id, scope: UsageScope())) {
                LabeledContent {
                    Text(state.forecast.map { Fmt.usd($0.global.cost) } ?? "").foregroundStyle(.secondary)
                } label: {
                    Label("Usage", systemImage: "chart.bar.xaxis")
                        .accessibilityIdentifier("usage")
                }
            }
            NavigationLink(value: SearchRoute(routerId: state.router.id)) {
                Label("Search", systemImage: "magnifyingglass")
                    .accessibilityIdentifier("search")
            }
        }
    }

    private var sessionsSummary: String {
        let n = state.sessions.count
        if n == 0 { return state.forecast == nil ? "" : "none this week" }
        return state.liveSummary.map { "\($0) · \(n) this week" } ?? "\(n) this week"
    }

    @ViewBuilder private func burnDownSection(_ a: Forecast.ForecastAccount) -> some View {
        let windows: [(String, Forecast.Window)] = [("5-hour", a.fiveHour), ("7-day", a.sevenDay)]
            .compactMap { name, w in w.flatMap { $0.isExpired || $0.windowStart == nil ? nil : (name, $0) } }
        if !windows.isEmpty {
            Section {
                ForEach(windows, id: \.0) { name, w in
                    VStack(alignment: .leading, spacing: 4) {
                        Text("\(name) burn-down")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        BurnDownChart(window: w)
                    }
                    .padding(.vertical, 4)
                }
            } footer: {
                Text("Solid is what has been used; dashed is where the current rate of work leads. Burn is measured per hour of work, not per hour on the clock.")
            }
        }
    }

    @ViewBuilder private var accountsSection: some View {
        let list = state.switchable
        if !list.isEmpty {
            Section {
                ForEach(list) { a in
                    Button {
                        Task { await state.activate(a.id) }
                    } label: {
                        AccountRow(account: a, credential: state.credential(for: a.id),
                                   switching: state.switching == a.id)
                    }
                    .buttonStyle(.plain)
                    .disabled(a.isActive || state.switching != nil)
                    .accessibilityHint(a.isActive ? "Serving now" : "Switch the router to this account")
                }
                NavigationLink(value: AccountsRoute(routerId: state.router.id)) {
                    Label("Manage accounts", systemImage: "person.crop.circle.badge.plus")
                        .accessibilityIdentifier("manage-accounts")
                }
            } header: {
                Text("Accounts")
            } footer: {
                Text("Tap an account to serve with it. The switch takes effect on the next request, mid-session.")
            }
        }
    }

    @ViewBuilder private var totalsSection: some View {
        if let g = state.forecast?.global {
            Section {
                LabeledContent("Spend", value: Fmt.usd(g.cost))
                LabeledContent("Tokens read", value: Fmt.compact(g.promptTokens))
                LabeledContent("Requests", value: Fmt.count(g.requests))
                LabeledContent {
                    VStack(alignment: .trailing) {
                        Text(Fmt.hours(g.combinedHeadroom.activeHoursLeft))
                        if let f = g.combinedHeadroom.firstToExhaust, let at = Fmt.date(ms: f.at) {
                            Text("first to go: \(f.accountId) \(Fmt.relative(at))")
                                .font(.caption)
                                .foregroundStyle(.red)
                        }
                    }
                } label: {
                    Text("Headroom")
                }
                LabeledContent {
                    VStack(alignment: .trailing) {
                        Text(Fmt.usd(g.pace30d.cost))
                        Text(Fmt.compact(g.pace30d.promptTokens) + " tokens")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                } label: {
                    Text("30-day pace")
                }
            } header: {
                Text("All accounts")
            } footer: {
                Text("Spend, tokens and requests add up across accounts. Quota does not — each percentage is of its own limit — so headroom is hours of work left. Pace is from the last 7 days of recorded spend.")
            }
        }
    }

    @ViewBuilder private var routerSection: some View {
        Section("Router") {
            LabeledContent("Address", value: state.router.address)
            if let h = state.health {
                LabeledContent("Version", value: "\(h.version) · pid \(h.pid)")
            }
            if let auth = state.auth {
                LabeledContent("Password", value: auth.configured ? "set" : "not set")
            }
            if let t = state.lastRefresh {
                LabeledContent("Updated", value: Fmt.ago(t))
            }
            NavigationLink(value: SettingsRoute(routerId: state.router.id)) {
                Label("Settings", systemImage: "gearshape")
                    .accessibilityIdentifier("settings")
            }
        }
    }

    // MARK: toolbar

    private var accountMenu: some View {
        Menu {
            Picker("Account", selection: Binding(
                get: { state.activeAccount?.id ?? "" },
                set: { id in Task { await state.activate(id) } }
            )) {
                ForEach(state.switchable) { a in
                    Text(a.title).tag(a.id)
                }
            }
        } label: {
            Label("Switch account", systemImage: "person.2.circle")
        }
        .disabled(state.switchable.isEmpty)
    }
}

private struct AccountRow: View {
    let account: Forecast.ForecastAccount
    let credential: Account?
    let switching: Bool

    var body: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 3) {
                Text(account.title)
                    .font(.body)
                    .foregroundStyle(.primary)
                HStack(spacing: 6) {
                    if let plan = account.plan { Text(plan) }
                    Text(Fmt.usd(account.week.cost) + " this week")
                    if let w = account.sevenDay, !w.isExpired, let used = w.used {
                        Text("7d " + Fmt.percent(used))
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                if let c = credential, c.ready == false {
                    Label(c.detail ?? "credential unavailable", systemImage: "exclamationmark.triangle")
                        .font(.caption2)
                        .foregroundStyle(.orange)
                }
            }
            Spacer()
            if switching {
                ProgressView()
            } else if account.isActive {
                Image(systemName: "checkmark")
                    .foregroundStyle(.tint)
                    .fontWeight(.semibold)
                    .accessibilityLabel("Active")
            }
        }
        .contentShape(Rectangle())
    }
}
