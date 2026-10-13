import Charts
import SwiftUI

/// The dashboard's page, for one router and one scope: totals, the activity chart, the
/// cache and context splits, models, threads, accounts — and the lists beneath.
struct UsageView: View {
    let usage: UsageState

    var body: some View {
        List {
            Section {
                Picker("Range", selection: Bindable(usage).range) {
                    ForEach(UsageRange.allCases) { r in Text(r.label).tag(r) }
                }
                .pickerStyle(.segmented)
                .listRowInsets(EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16))
                .listRowBackground(Color.clear)
                .accessibilityIdentifier("usage-range")
                // The provider every figure follows, as the console's scope offers it: shown
                // only while there are two to choose between.
                if let choices = usage.providerChoices {
                    Picker("Provider", selection: Bindable(usage).provider) {
                        ForEach(choices, id: \.self) { p in Text(p.map(Provider.name) ?? "All providers").tag(p) }
                    }
                    .pickerStyle(.segmented)
                    .listRowInsets(EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16))
                    .listRowBackground(Color.clear)
                    .accessibilityIdentifier("usage-provider")
                }
            }
            if let f = usage.failure {
                Section { Label(f, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange) }
            }
            if let s = usage.summary {
                tiles(s)
                if !usage.buckets.isEmpty {
                    Section("Activity over time") {
                        ActivityChart(buckets: usage.buckets, bucketMs: s.timeline.bucketMs)
                            .listRowInsets(EdgeInsets(top: 10, leading: 12, bottom: 10, trailing: 12))
                    }
                }
                ForEach(s.providers.filter { !$0.history.isEmpty }) { block in
                    Section {
                        QuotaHistoryChart(block: block)
                            .listRowInsets(EdgeInsets(top: 10, leading: 12, bottom: 10, trailing: 12))
                    } header: {
                        ProviderMark(provider: block.provider, form: .words("\(Provider.name(block.provider)) quota over this range"))
                    } footer: {
                        Text("Percent of each window used, for \(Provider.name(block.provider))'s account serving now.")
                    }
                }
                readSection(s)
                contextSection(s)
                modelsSection(s)
                threadsSection(s)
                accountsSection(s)
            } else if usage.failure == nil {
                Section { HStack { ProgressView(); Text("Reading…").foregroundStyle(.secondary) } }
            }
            detailsSection
        }
        .navigationTitle(usage.scope.title)
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await usage.refresh() }
        .task { await usage.poll() }
    }

    // MARK: totals

    @ViewBuilder private func tiles(_ s: Summary) -> some View {
        let t = s.totals
        let b = usage.buckets
        let hit = t.promptTokens > 0 ? t.cacheReadTokens / t.promptTokens : 0
        let spend = Fmt.costSum(t.cost, unpriced: t.unpriced, estimated: t.estimated)
        let split = { (total: ProviderSplit.Total) in ProviderSplit.parts(t.byProvider, total) }
        Section {
            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 10) {
                StatTile("Prompt tokens read", Fmt.compact(t.promptTokens),
                         t.firstSeen != nil ? "\(Fmt.when(Fmt.date(ms: t.firstSeen)!)) → \(Fmt.when(Fmt.date(ms: t.lastSeen)!))" : "nothing recorded in this range") {
                    if b.count > 1 { Sparkline(values: b.map(\.prompt)) }
                    if let parts = split(.prompt) { ProviderSplitView(parts: parts) }
                }
                StatTile("Spend", t.cost > 0 || (t.unpriced ?? 0) > 0 ? spend.text : "—",
                         t.cost > 0 ? "\(Fmt.compact(t.cacheReadTokens)) cached reads · \(Fmt.compact(t.outputTokens)) out" : "nothing priced in scope") {
                    if let parts = split(.spend) { ProviderSplitView(parts: parts) }
                }
                StatTile("Requests", Fmt.count(t.requests),
                         "\(Fmt.count(t.sessions)) sessions · \(Fmt.count(t.projects)) projects · \(Fmt.count(t.subagentRequests)) by subagents") {
                    if let parts = split(.requests) { ProviderSplitView(parts: parts) }
                }
                StatTile("Output tokens", Fmt.compact(t.outputTokens), "\(Fmt.compact(t.thinkingTokens)) of it thinking") {
                    if b.count > 1 { Sparkline(values: b.map(\.output)) }
                    if let parts = split(.output) { ProviderSplitView(parts: parts) }
                }
                StatTile("Served from cache", Fmt.share(hit), "cache reads ÷ everything read") {
                    Gauge(value: hit, in: 0...1) { EmptyView() }
                        .gaugeStyle(.accessoryLinearCapacity)
                        .tint(.teal)
                    if let parts = split(.cache) { ProviderSplitView(parts: parts) }
                }
                StatTile("Cache writes", Fmt.compact(t.cacheWriteTokens),
                         "\(Fmt.compact(t.cacheWrite5mTokens)) for 5 min · \(Fmt.compact(t.cacheWrite1hTokens)) for an hour") {
                    if let parts = split(.writes) { ProviderSplitView(parts: parts) }
                }
                StatTile("Time to first token", Fmt.ms(s.latency?.ttfbP50),
                         "p95 \(Fmt.ms(s.latency?.ttfbP95)) · slowest turn \(Fmt.ms(s.latency?.durationMax))")
                StatTile("Errors", Fmt.count(t.errors),
                         t.requests > 0 ? "\(Fmt.share(t.errors / t.requests)) of requests" : "—", bad: t.errors > 0)
            }
            .listRowInsets(EdgeInsets())
            .listRowBackground(Color.clear)
        } header: {
            Text("Totals in scope")
        } footer: {
            if let note = Fmt.costNote([spend]) { Text(note) }
        }
    }

    // MARK: splits

    @ViewBuilder private func readSection(_ s: Summary) -> some View {
        let t = s.totals
        Section {
            ShareBar(segments: [
                .init(key: "input", label: "Uncached input", value: t.inputTokens, color: .orange),
                .init(key: "w5", label: "Cache writes, 5 min", value: t.cacheWrite5mTokens, color: .yellow),
                .init(key: "w1", label: "Cache writes, 1 hour", value: t.cacheWrite1hTokens, color: .brown),
                .init(key: "read", label: "Cache reads", value: t.cacheReadTokens, color: .teal),
            ])
        } header: {
            Text("What the model read")
        } footer: {
            Text(t.promptTokens > 0
                 ? "Hit rate = cache reads ÷ everything read = \(Fmt.share(t.cacheReadTokens / t.promptTokens)). Cached tokens are far cheaper against the quota than tokens paid in full."
                 : "Nothing read yet.")
        }
    }

    private static let contextColors: [String: Color] = [
        "system": .indigo, "tools": .cyan, "history": .yellow, "tool_results": .pink,
    ]

    @ViewBuilder private func contextSection(_ s: Summary) -> some View {
        if !s.context.isEmpty {
            Section {
                ShareBar(segments: s.context.map {
                    .init(key: $0.component, label: $0.component.replacingOccurrences(of: "_", with: " "),
                          value: $0.estTokens, color: Self.contextColors[$0.component] ?? .gray)
                })
                ForEach(s.context) { c in
                    LabeledContent(c.component.replacingOccurrences(of: "_", with: " ")) {
                        Text("\(Fmt.compact(c.estTokens)) · \(Fmt.usd(c.estCostOnce)) if paid once")
                            .font(.caption)
                    }
                }
            } header: {
                Text("What fills the context window")
            } footer: {
                Text("Estimate, by character share of each request. Ratios are reliable; the absolutes are not.")
            }
        }
    }

    @ViewBuilder private func modelsSection(_ s: Summary) -> some View {
        if !s.models.isEmpty {
            let max = s.models.map(\.promptTokens).max() ?? 1
            let costs = s.models.map { Fmt.costSum($0.cost, unpriced: $0.unpriced, estimated: $0.estimated) }
            Section {
                ForEach(Array(s.models.enumerated()), id: \.element.id) { i, m in
                    BarRow(value: m.promptTokens, max: max, color: FamilyColor.of(Bucket.family(m.model)),
                           text: "\(costs[i].text) · \(Fmt.count(m.requests)) req · \(Fmt.compact(m.outputTokens)) out") {
                        HStack(spacing: 6) {
                            ProviderMark(provider: m.provider, form: .alone).font(.caption2).foregroundStyle(.secondary)
                            Text(Fmt.shortModel(m.model)).font(.subheadline)
                        }
                    }
                }
            } header: {
                Text("By model")
            } footer: {
                if let note = Fmt.costNote(costs) { Text(note) }
            }
        }
    }

    @ViewBuilder private func threadsSection(_ s: Summary) -> some View {
        if s.threads.count > 1 {
            let max = s.threads.map(\.promptTokens).max() ?? 1
            Section {
                ForEach(s.threads) { t in
                    BarRow(value: t.promptTokens, max: max, color: t.thread == "subagent" ? .pink : .accentColor,
                           text: "\(Fmt.costSum(t.cost, unpriced: t.unpriced, estimated: t.estimated).text) · \(Fmt.count(t.requests)) req" + (t.thread == "subagent" ? " · \(Fmt.count(t.agents)) agents" : "")) {
                        Label(t.thread == "subagent" ? "Subagents" : "Main thread",
                              systemImage: t.thread == "subagent" ? "person.2" : "person")
                            .font(.subheadline)
                    }
                }
            } header: {
                Text("Main thread vs subagents")
            } footer: {
                Text("Share of prompt tokens.")
            }
        }
    }

    @ViewBuilder private func accountsSection(_ s: Summary) -> some View {
        if s.byAccount.count > 1, usage.scope.account == nil {
            let max = s.byAccount.map(\.promptTokens).max() ?? 1
            Section {
                ForEach(s.byAccount) { a in
                    NavigationLink(value: UsageRoute(routerId: usage.router.router.id, scope: usage.narrowed(account: a.id))) {
                        BarRow(value: a.promptTokens, max: max, color: .accentColor,
                               text: "\(Fmt.costSum(a.cost, unpriced: a.unpriced, estimated: a.estimated).text) · \(Fmt.count(a.requests)) req · \(Fmt.count(a.sessions)) sessions") {
                            HStack(spacing: 6) {
                                ProviderMark(provider: a.provider, form: .alone).font(.caption2).foregroundStyle(.secondary)
                                Text(a.label).font(.subheadline)
                                if a.active == 1 { Text("active for \(Provider.name(a.provider))").font(.caption2).foregroundStyle(.green) }
                            }
                        }
                    }
                }
            } header: {
                Text("Accounts")
            } footer: {
                Text("Tap one to narrow the page to it.")
            }
        }
    }

    @ViewBuilder private var detailsSection: some View {
        let id = usage.router.router.id
        Section("In detail") {
            NavigationLink(value: UsageListRoute(routerId: id, scope: usage.scope, kind: .projects)) {
                LabeledContent { Text(Fmt.count(Double(usage.projects.count))) } label: { Label("Projects", systemImage: "folder") }
            }
            NavigationLink(value: UsageListRoute(routerId: id, scope: usage.scope, kind: .sessions)) {
                LabeledContent { Text(Fmt.count(Double(usage.sessions.count))) } label: { Label("Sessions", systemImage: "rectangle.stack") }
            }
            NavigationLink(value: UsageListRoute(routerId: id, scope: usage.scope, kind: .tools)) {
                LabeledContent { Text(Fmt.count(Double(usage.tools?.called.count ?? 0))) } label: { Label("Tools", systemImage: "wrench.and.screwdriver") }
            }
            NavigationLink(value: UsageListRoute(routerId: id, scope: usage.scope, kind: .requests)) {
                LabeledContent { Text(Fmt.count(Double(usage.requests.count))) } label: { Label("Requests", systemImage: "list.bullet") }
            }
        }
        .accessibilityIdentifier("usage-details")
    }
}

/// One provider's two windows over the range, each by its own name, on the activity chart's
/// axis: a bucket with no reading for a window is left off that window's line, never 0%.
struct QuotaHistoryChart: View {
    let block: ProviderQuota

    /// The block's second window, by the name its provider gives it.
    private var second: WindowKey {
        block.windows.keys.contains("month") || block.history.contains(where: { $0.month != nil }) ? .month : .sevenDay
    }

    var body: some View {
        let second = second
        VStack(alignment: .leading, spacing: 6) {
            Chart {
                ForEach(block.history, id: \.t) { p in
                    if let v = p.fiveHour {
                        LineMark(x: .value("Time", p.date), y: .value("Used", min(1, v)), series: .value("Window", WindowKey.fiveHour.name))
                            .foregroundStyle(Color.accentColor)
                    }
                    if let v = p.value(second) {
                        LineMark(x: .value("Time", p.date), y: .value("Used", min(1, v)), series: .value("Window", second.name))
                            .foregroundStyle(Color.secondary)
                    }
                }
                RuleMark(y: .value("Limit", 1.0)).foregroundStyle(.quaternary)
            }
            .chartYScale(domain: 0...1)
            .chartYAxis { AxisMarks(values: [0, 0.5, 1]) { v in AxisGridLine(); AxisValueLabel { if let d = v.as(Double.self) { Text(Fmt.percent(d)) } } } }
            .chartXAxis { AxisMarks(values: .automatic(desiredCount: 3)) { _ in AxisGridLine(); AxisValueLabel(format: .dateTime.month(.abbreviated).day().hour()) } }
            .frame(height: 90)
            HStack(spacing: 12) {
                Label(WindowKey.fiveHour.name, systemImage: "minus").foregroundStyle(Color.accentColor)
                Label(second.name, systemImage: "minus").foregroundStyle(.secondary)
            }
            .font(.caption2)
        }
        .accessibilityLabel("\(Provider.name(block.provider)) quota over this range")
    }
}

/// A total's parts by provider, under its tile: a thin bar of the shares, then each provider's
/// mark, its figure and its share.
struct ProviderSplitView: View {
    let parts: [ProviderPart]

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            GeometryReader { g in
                HStack(spacing: 1) {
                    ForEach(parts) { p in
                        Rectangle().fill(FamilyColor.provider(p.provider))
                            .frame(width: max(0, g.size.width * (p.share ?? 0) - 1))
                    }
                }
            }
            .frame(height: 3)
            ForEach(parts) { p in
                HStack(spacing: 4) {
                    ProviderMark(provider: p.provider, form: .alone)
                    Text(p.figure).lineLimit(1).minimumScaleFactor(0.8)
                    Spacer(minLength: 2)
                    if let share = p.share { Text(Fmt.share(share)).foregroundStyle(.secondary) }
                }
                .font(.caption2)
            }
        }
        .padding(.top, 2)
        .accessibilityElement(children: .combine)
    }
}
