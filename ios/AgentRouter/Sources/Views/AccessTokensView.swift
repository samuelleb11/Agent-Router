import SwiftUI

/// Settings → Access tokens: every router credential with its state, a new one made and shown
/// once, and one revoked.
struct AccessTokensView: View {
    let state: RouterState
    @State private var model: AccessTokensState?
    @State private var creating = false
    @State private var created: CreatedToken?
    @State private var revoking: AccessToken?

    var body: some View {
        List {
            Section {
                Text(AccessTokens.explain)
                Text(AccessTokens.howSent).foregroundStyle(.secondary)
            }
            .font(.footnote)
            if let f = model?.failure { Section { Label(f, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange) } }
            if let d = model?.done { Section { Text(d).font(.footnote).foregroundStyle(.green) } }
            if let tokens = model?.tokens {
                Section("Tokens") {
                    if tokens.isEmpty { Text("No access token yet.").foregroundStyle(.secondary) }
                    ForEach(tokens) { t in
                        TokenRow(token: t)
                            .swipeActions(edge: .trailing) {
                                if t.revokedAt == nil {
                                    Button("Revoke", systemImage: "xmark.circle", role: .destructive) { revoking = t }
                                }
                            }
                    }
                }
            }
        }
        .navigationTitle("Access tokens")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Create a token", systemImage: "plus") { creating = true }
                    .disabled(model?.tokens == nil)
                    .accessibilityIdentifier("create-token")
            }
        }
        .overlay { if model?.tokens == nil, model?.failure == nil { ProgressView() } }
        .refreshable { await model?.load() }
        .sheet(isPresented: $creating) {
            if let model { CreateTokenSheet(model: model) { created = $0 } }
        }
        .sheet(item: $created) { CreatedTokenSheet(token: $0) }
        .confirmationDialog("Revoke \(revoking?.name ?? "")?", isPresented: Binding(get: { revoking != nil }, set: { if !$0 { revoking = nil } }), titleVisibility: .visible) {
            Button("Revoke token", role: .destructive) {
                if let t = revoking { Task { await model?.revoke(t) } }
            }
        } message: {
            Text(AccessTokens.revokeWhat)
        }
        .task {
            let m = AccessTokensState(router: state)
            model = m
            await m.load()
        }
    }
}

private struct TokenRow: View {
    let token: AccessToken

    var body: some View {
        let status = AccessTokens.status(token)
        VStack(alignment: .leading, spacing: 3) {
            HStack {
                Text(token.name).font(.body)
                Spacer()
                Text(status).font(.caption2.weight(.medium))
                    .foregroundStyle(status == "Active" ? Color.green : Color.secondary)
            }
            Text("\(token.prefix)… · \(AccessTokens.scopeLabel(token.scope))")
                .font(.caption.monospaced()).foregroundStyle(.secondary)
            Text(["Created \(time(token.createdAt))",
                  token.lastUsedAt.map { "last used \(time($0))" } ?? "never used",
                  token.expiresAt.map { "expires \(time($0))" } ?? "never expires",
                  token.revokedAt.map { "revoked \(time($0))" }].compactMap { $0 }.joined(separator: " · "))
                .font(.caption).foregroundStyle(.secondary)
        }
        .padding(.vertical, 2)
    }

    private func time(_ ms: Int) -> String { Fmt.date(ms: ms).map(Fmt.when) ?? "—" }
}

/// A new token: its name, its permissions and its lifetime.
private struct CreateTokenSheet: View {
    let model: AccessTokensState
    let made: (CreatedToken) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var scope = "client"
    @State private var days: Int? = 90
    @State private var working = false
    @State private var message: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $name, prompt: Text("Codex on my laptop"))
                        .accessibilityIdentifier("token-name")
                } footer: { Text("The client or machine using it; up to 64 characters.") }
                Section {
                    Picker("Permissions", selection: $scope) {
                        ForEach(AccessTokens.scopes) { Text($0.label).tag($0.value) }
                    }
                    Picker("Expires after", selection: $days) {
                        ForEach(AccessTokens.lifetimes, id: \.label) { Text($0.label).tag($0.days) }
                    }
                } footer: { Text(AccessTokens.scopes.first { $0.value == scope }?.description ?? "") }
                if let message { Section { Text(message).font(.footnote).foregroundStyle(.red) } }
            }
            .navigationTitle("Create an access token")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() }.disabled(working) }
                ToolbarItem(placement: .confirmationAction) {
                    Button(working ? "Creating…" : "Create token") {
                        Task {
                            working = true
                            defer { working = false }
                            switch await model.create(name: name, scope: scope, days: days) {
                            case .success(let t): dismiss(); made(t)
                            case .failure(let e): message = e.errorDescription
                            }
                        }
                    }
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || working)
                }
            }
        }
    }
}

/// The token just made, in full, this once.
private struct CreatedTokenSheet: View {
    let token: CreatedToken
    @Environment(\.dismiss) private var dismiss
    @State private var copied = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text(AccessTokens.shownOnce)
                    Text(token.token).font(.body.monospaced()).textSelection(.enabled)
                        .accessibilityIdentifier("created-token")
                    Button(copied ? "Copied" : "Copy token", systemImage: "doc.on.doc") {
                        UIPasteboard.general.string = token.token
                        copied = true
                    }
                    ShareLink(item: token.token) { Label("Share", systemImage: "square.and.arrow.up") }
                } footer: { Text(AccessTokens.createdWords(token)) }
            }
            .navigationTitle("Save your access token")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("I have saved the token") { dismiss() } }
            }
            .interactiveDismissDisabled()
        }
    }
}
