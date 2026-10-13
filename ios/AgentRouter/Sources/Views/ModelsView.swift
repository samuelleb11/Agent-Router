import SwiftUI

/// One page of a router's Settings, by its name.
struct SettingsPageView: View {
    let state: RouterState
    let page: SettingsPage

    var body: some View {
        switch page {
        case .accounts: AccountsView(state: state)
        case .models: ModelsView(state: state)
        case .claudeCode: ClaudeCodeSettingsView(state: state)
        case .network: NetworkSettingsView(state: state)
        case .accessTokens: AccessTokensView(state: state)
        case .password: PasswordSettingsView(state: state)
        case .notifications: NotificationSettingsView(state: state)
        case .router: RouterInfoView(state: state)
        case .clients: ClientsView(state: state)
        case .forges, .library: PendingSettingsPage(page: page)
        }
    }
}

/// Settings → Models: every model the router serves, the account its next request goes to,
/// and the choice of another account of its own provider.
struct ModelsView: View {
    let state: RouterState
    @State private var model: ModelsState?

    var body: some View {
        List {
            if let f = model?.failure {
                Section { Label(f, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange) }
            }
            Section {
                ForEach(model?.listed ?? []) { m in
                    if let model { ModelCard(model: model, row: m) }
                }
            } header: {
                Text("Models")
            } footer: {
                Text("A model given no account of its own goes to its provider’s active account.")
            }
            RegistrySections(state: state)
        }
        .navigationTitle("Models")
        .overlay { if model?.models == nil, model?.failure == nil { ProgressView() } }
        .refreshable { await model?.load() }
        .task {
            let m = ModelsState(router: state)
            model = m
            await m.load()
        }
    }
}

/// One model: its name and id, its provider, who serves it, and the account it goes to.
private struct ModelCard: View {
    let model: ModelsState
    let row: CatalogueModel

    var body: some View {
        let choices = ModelChoices.choices(row, accounts: model.accounts, hosted: model.hosted)
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                ProviderMark(provider: row.provider, form: .alone).font(.caption)
                Text(row.name).font(.body)
            }
            if row.name != row.id {
                Text(row.id).font(.caption.monospaced()).foregroundStyle(.secondary)
            }
            Text("Served by: \(ModelChoices.servedBy(row, accounts: model.accounts))")
                .font(.caption).foregroundStyle(.secondary)
            // Its label hidden, so the choice — "Anthropic’s active account (…)" — has the
            // row's width rather than half of it.
            Picker("Account", selection: Binding(
                get: { row.assigned ?? "" },
                set: { id in
                    if let c = choices.first(where: { $0.id == id }) { Task { await model.assign(row, to: c) } }
                }
            )) {
                ForEach(choices) { Text($0.label).tag($0.id) }
            }
            .labelsHidden()
            .frame(maxWidth: .infinity, alignment: .leading)
            .disabled(model.busy != nil)
            .accessibilityIdentifier("model-account-\(row.id)")
            if let n = model.notice[row.id] {
                Text(n.text).font(.caption).foregroundStyle(n.ok ? Color.secondary : Color.red)
            }
        }
        .padding(.vertical, 2)
    }
}
