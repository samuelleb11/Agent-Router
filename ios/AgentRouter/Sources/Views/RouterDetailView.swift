import SwiftUI

/// One router: each provider's quota block — its serving account, how much is left and when it
/// runs out — every account it holds by provider, and the switch between them.
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
            // The links first, as before the blocks came: a thumb reaches the sessions without
            // scrolling past every provider's quota.
            sessionsSection
            quotaSections
            accountsSections
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

    /// One block per provider with an account, stacked, as the console shows them at phone
    /// width; none, and where to add one, when the router holds no account.
    @ViewBuilder private var quotaSections: some View {
        let blocks = state.blocks
        if blocks.isEmpty {
            Section("Quota headroom") {
                if state.forecast == nil {
                    Text(state.failure == nil ? "Connecting…" : "No data").foregroundStyle(.secondary)
                } else {
                    NavigationLink(value: AccountsRoute(routerId: state.router.id)) {
                        Text("No account yet — add one in Accounts.")
                    }
                }
            }
        }
        ForEach(blocks) { block in
            QuotaBlockSection(block: block, routerId: state.router.id)
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

    /// The accounts by provider, each provider with its own serving account: a tap serves that
    /// provider's requests with it, and leaves the others' as they are.
    @ViewBuilder private var accountsSections: some View {
        let groups = state.accountsByProvider
        ForEach(groups, id: \.provider) { group in
            Section {
                ForEach(group.accounts) { a in
                    Button {
                        Task { await state.activate(a.id) }
                    } label: {
                        AccountRow(account: a, credential: state.credential(for: a.id),
                                   switching: state.switching == a.id)
                    }
                    .buttonStyle(.plain)
                    .disabled(a.isActive || state.switching != nil)
                    .accessibilityHint(a.isActive ? "Serving now" : "Serve \(Provider.name(group.provider))'s requests with this account")
                }
                if group.provider == groups.last?.provider {
                    NavigationLink(value: AccountsRoute(routerId: state.router.id)) {
                        Label("Manage accounts", systemImage: "person.crop.circle.badge.plus")
                            .accessibilityIdentifier("manage-accounts")
                    }
                }
            } header: {
                ProviderMark(provider: group.provider, form: .words("\(Provider.name(group.provider)) accounts"))
            } footer: {
                if group.provider == groups.last?.provider {
                    Text("Tap an account to serve its provider's requests with it. Each provider has its own; the switch takes effect on the next request, mid-session.")
                }
            }
        }
    }

    @ViewBuilder private var totalsSection: some View {
        if let f = state.forecast {
            let g = f.global
            let spend = Fmt.costSum(g.cost, unpriced: f.accounts.compactMap(\.usage.unpriced).reduce(0, +),
                                    estimated: f.accounts.compactMap(\.usage.estimated).reduce(0, +))
            let pace = Fmt.costSum(g.pace30d.cost, unpriced: g.pace30d.unpriced, estimated: g.pace30d.estimated)
            Section {
                LabeledContent {
                    VStack(alignment: .trailing) {
                        Text(spend.text)
                        Text("across \(g.accounts) account\(g.accounts == 1 ? "" : "s"), at API rates")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                } label: { Text("Spend, every account") }
                LabeledContent("Tokens read", value: Fmt.compact(g.promptTokens))
                LabeledContent("Requests", value: Fmt.count(g.requests))
                LabeledContent {
                    VStack(alignment: .trailing) {
                        Text(pace.text)
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
                Text(["Spend, tokens and requests add up across accounts. Quota does not — each percentage is of its own limit — so each provider's headroom is in its own block. Pace is from \(g.pace30d.basis).",
                      Fmt.costNote([spend, pace])].compactMap { $0 }.joined(separator: " "))
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
                LabeledContent("Authentication", value: auth.credentialStatus)
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

    /// The switch, a picker per provider: choosing one serves that provider alone.
    private var accountMenu: some View {
        Menu {
            ForEach(state.accountsByProvider, id: \.provider) { group in
                Picker(Provider.name(group.provider), selection: Binding(
                    get: { group.accounts.first(where: \.isActive)?.id ?? "" },
                    set: { id in Task { await state.activate(id) } }
                )) {
                    ForEach(group.accounts) { a in
                        Text(a.title).tag(a.id)
                    }
                }
                .pickerStyle(.inline)
            }
        } label: {
            Label("Switch account", systemImage: "person.2.circle")
        }
        .disabled(state.accountsByProvider.isEmpty)
    }
}

/// One provider's quota block: its mark, the account serving it, its own headroom and first to
/// run out, its windows by their own names, where the reading came from, and its burn-downs.
private struct QuotaBlockSection: View {
    let block: QuotaBlock
    let routerId: UUID

    var body: some View {
        Section {
            NavigationLink(value: AccountsRoute(routerId: routerId)) {
                LabeledContent("Serving now", value: block.serving)
            }
            if let room = block.room {
                VStack(alignment: .leading, spacing: 4) {
                    LabeledContent("Headroom left", value: room.value)
                    Text(room.sub)
                        .font(.caption)
                        .foregroundStyle(room.tone == .bad ? .red : room.tone == .warn ? .orange : .secondary)
                    if let first = room.first {
                        Text(first).font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            if let meters = block.meters {
                ForEach(meters) { QuotaRow(meter: $0) }
                HStack(spacing: 8) {
                    Text("Status")
                    if let st = block.status { StatusTag(status: st) } else { Text("—") }
                    Spacer()
                    if let source = block.source {
                        Text(source).font(.caption).foregroundStyle(.secondary)
                    }
                }
            } else {
                Text("No quota reading yet for \(block.name)'s serving account.")
                    .foregroundStyle(.secondary)
            }
            ForEach(burnDowns, id: \.0) { name, w in
                VStack(alignment: .leading, spacing: 4) {
                    Text("\(name) burn-down")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    BurnDownChart(window: w)
                }
                .padding(.vertical, 4)
            }
        } header: {
            ProviderMark(provider: block.provider)
        } footer: {
            if !burnDowns.isEmpty {
                Text("Solid is what has been used; dashed is where the current rate of work leads. Burn is measured per hour of work, not per hour on the clock.")
            }
        }
    }

    private var burnDowns: [(String, Forecast.Window)] {
        (block.forecast?.windows ?? [])
            .compactMap { k, w in w.flatMap { $0.isExpired || $0.windowStart == nil ? nil : (k.title, $0) } }
    }
}

/// A block's status, in the word its provider gave and the colour that word means.
private struct StatusTag: View {
    let status: QuotaStatus

    var body: some View {
        Label {
            Text(status.word)
        } icon: {
            if status.tone == .bad { Image(systemName: "exclamationmark.octagon.fill") }
        }
        .font(.caption.weight(.medium))
        .padding(.horizontal, 8)
        .padding(.vertical, 2)
        .background(color.opacity(0.15), in: Capsule())
        .foregroundStyle(color)
    }

    private var color: Color {
        switch status.tone {
        case .ok: .green
        case .warn: .orange
        case .bad: .red
        case .neutral: .secondary
        }
    }
}

private struct AccountRow: View {
    let account: Forecast.ForecastAccount
    let credential: Account?
    let switching: Bool

    var body: some View {
        let cost = Fmt.costSum(account.week.cost, unpriced: account.week.unpriced, estimated: account.week.estimated)
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 3) {
                Text(account.title)
                    .font(.body)
                    .foregroundStyle(.primary)
                HStack(spacing: 6) {
                    if let plan = account.plan { Text(plan) }
                    Text(cost.text + " this week")
                    if let w = account.second, !w.isExpired, let used = w.used {
                        Text(account.secondKey.short + " " + Fmt.percent(used))
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                if account.second?.spent == true {
                    Text("its \(account.secondKey.name) window is spent until it resets")
                        .font(.caption2)
                        .foregroundStyle(.red)
                }
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
                    .accessibilityLabel("Serving \(Provider.name(account.providerId))")
            }
        }
        .contentShape(Rectangle())
    }
}
