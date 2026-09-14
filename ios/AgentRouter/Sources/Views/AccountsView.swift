import SwiftUI

/// Every account the router holds — the dashboard's settings table, as a list.
struct AccountsView: View {
    let state: RouterState
    @State private var model: AccountsState?
    @State private var editing: Account?
    @State private var adding = false
    @State private var removing: Account?
    @State private var notice: String?

    var body: some View {
        List {
            if let f = model?.failure { Section { Label(f, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange) } }
            if let notice { Section { Text(notice).font(.footnote).foregroundStyle(.red) } }
            Section {
                ForEach(model?.accounts ?? []) { a in
                    Button { editing = a } label: { AccountRowView(account: a) }
                        .buttonStyle(.plain)
                        .swipeActions(edge: .leading) {
                            if !a.isActive {
                                Button("Serve with", systemImage: "checkmark.circle") {
                                    Task { notice = await model?.activate(a.id) }
                                }
                                .tint(.green)
                            }
                        }
                        .swipeActions(edge: .trailing) {
                            Button("Remove", systemImage: "trash", role: .destructive) { removing = a }
                                .disabled(a.isActive)
                        }
                }
            } footer: {
                Text("Tap an account to rename it, say who it is, rotate its credential or store a claude.ai session key. Swipe right to serve with it, left to remove it. The serving account cannot be removed.")
            }
        }
        .navigationTitle("Accounts")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Add account", systemImage: "plus") { adding = true }
            }
        }
        .overlay { if model?.accounts.isEmpty ?? true, model?.failure == nil { ProgressView() } }
        .refreshable { await model?.load(probe: true) }
        .sheet(item: $editing) { a in
            if let model { AccountEditSheet(model: model, account: a) }
        }
        .sheet(isPresented: $adding) {
            if let model { AddAccountSheet(model: model) }
        }
        .confirmationDialog("Remove \(removing?.title ?? "this account")?", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }), titleVisibility: .visible) {
            Button("Remove", role: .destructive) {
                if let a = removing { Task { notice = await model?.remove(a.id) } }
            }
        } message: {
            Text("The stored credential is deleted from the Mac. Everything recorded against this account stays.")
        }
        .task {
            let m = AccountsState(router: state)
            model = m
            await m.load(probe: true)
        }
    }
}

struct AccountRowView: View {
    let account: Account
    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Text(account.title).font(.body)
                if account.isActive { Text("serving").font(.caption2).foregroundStyle(.green) }
                Spacer()
                Circle().fill(account.ready == false ? Color.red : Color.green).frame(width: 8, height: 8)
                    .accessibilityLabel(account.ready == false ? "credential unavailable" : "ready")
            }
            Text([account.subscriptionType, account.orgName, account.source.map { "via \($0)" }].compactMap { $0 }.joined(separator: " · "))
                .font(.caption).foregroundStyle(.secondary)
            HStack(spacing: 6) {
                Text(account.detail ?? "—").lineLimit(1)
                if let e = account.expiry { Text("· expires \(Fmt.relative(e))") }
                if account.hasUsageKey { Text("· usage key").foregroundStyle(.teal) }
            }
            .font(.caption2).foregroundStyle(.tertiary)
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}

/// Rename, identify, rotate — the dashboard's edit box.
struct AccountEditSheet: View {
    let model: AccountsState
    let account: Account
    @Environment(\.dismiss) private var dismiss
    @State private var label = ""
    @State private var email = ""
    @State private var token = ""
    @State private var usage = ""
    @State private var saving = false
    @State private var message: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $label)
                    TextField("Identity", text: $email, prompt: Text("you@example.com"))
                        .keyboardType(.emailAddress).textInputAutocapitalization(.never).autocorrectionDisabled()
                } footer: {
                    Text("The id and the stored credential stay the same, so history keyed to this account stays intact.")
                }
                if account.source == "token" {
                    Section {
                        SecureField("New credential", text: $token, prompt: Text("sk-ant-oat01-…"))
                    } header: { Text("Credential") } footer: {
                        Text("A fresh `claude setup-token` value, if the current one was revoked or you want to rotate it.")
                    }
                }
                Section {
                    SecureField(account.hasUsageKey ? "Replace the stored key" : "Session key", text: $usage, prompt: Text("sk-ant-sid…"))
                    if account.hasUsageKey {
                        Button("Remove stored key", role: .destructive) {
                            Task { message = await model.usageKey(account.id, token: nil, clear: true) ?? "Removed." }
                        }
                    }
                } header: { Text("claude.ai session key") } footer: {
                    Text("Copied from your claude.ai browser cookie. Used only to read usage — never sent to api.anthropic.com, and kept in a separate Keychain entry on the Mac from the credential that signs requests.")
                }
                if let message { Section { Text(message).font(.footnote) } }
            }
            .navigationTitle(account.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) { Button("Save") { Task { await save() } }.disabled(saving) }
            }
            .onAppear { label = account.label; email = account.email ?? "" }
        }
    }

    private func save() async {
        saving = true
        defer { saving = false }
        var problems: [String] = []
        let l = label.trimmingCharacters(in: .whitespaces), e = email.trimmingCharacters(in: .whitespaces), t = token.trimmingCharacters(in: .whitespaces)
        if l != account.label || e != (account.email ?? "") || !t.isEmpty {
            if let p = await model.edit(AccountEditBody(id: account.id, label: l != account.label ? l : nil,
                                                          email: e != (account.email ?? "") ? e : nil, token: t.isEmpty ? nil : t)) { problems.append(p) }
        }
        let u = usage.trimmingCharacters(in: .whitespaces)
        if !u.isEmpty, let p = await model.usageKey(account.id, token: u, clear: false) { problems.append(p) }
        if problems.isEmpty { dismiss() } else { message = problems.joined(separator: " ") }
    }
}

/// The dashboard's add form: sign in through the router, paste a token, or point at a
/// separate profile on the Mac.
struct AddAccountSheet: View {
    let model: AccountsState
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @State private var label = ""
    @State private var source = "signin"
    @State private var token = ""
    @State private var working = false
    @State private var login: LoginStatus?
    @State private var polling = false
    @State private var message: String?

    private var valid: Bool {
        !label.trimmingCharacters(in: .whitespaces).isEmpty && (source != "token" || !token.isEmpty)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $label, prompt: Text("work"))
                    Picker("How", selection: $source) {
                        Text("Sign in from here").tag("signin")
                        Text("Paste a token").tag("token")
                        Text("Separate profile").tag("profile")
                    }
                    if source == "token" {
                        SecureField("Token", text: $token, prompt: Text("sk-ant-oat01-…"))
                    }
                } footer: {
                    Text(help)
                }
                if let login {
                    Section("Sign-in") {
                        if let url = login.url.flatMap(URL.init(string:)) {
                            Button("Open the sign-in page", systemImage: "safari") { openURL(url) }
                            Button("Copy the link", systemImage: "doc.on.doc") { UIPasteboard.general.string = url.absoluteString }
                            Text("Open it in a browser signed into the account you want to add. Nothing opens on its own.")
                                .font(.footnote).foregroundStyle(.secondary)
                        }
                        LabeledContent("Status", value: phaseText(login))
                        if let e = login.error { Text(e).font(.footnote).foregroundStyle(.red) }
                        if login.phase == "failed", let t = login.tail, !t.isEmpty {
                            Text(t).font(.system(.caption2, design: .monospaced)).foregroundStyle(.secondary)
                        }
                    }
                }
                if let message { Section { Text(message).font(.footnote) } }
            }
            .navigationTitle("Add account")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(source == "signin" ? "Start sign-in" : "Add") { Task { await go() } }
                        .disabled(!valid || working || polling)
                }
            }
            .task(id: polling) {
                guard polling else { return }
                while !Task.isCancelled {
                    if let st = await model.loginStatus() {
                        login = st
                        if st.phase == "done" {
                            await model.load(probe: true)
                            await model.router.refresh(includeAccounts: true)
                            dismiss()
                            return
                        }
                        if st.phase == "failed" { polling = false; return }
                    }
                    do { try await Task.sleep(for: .seconds(2)) } catch { return }
                }
            }
        }
    }

    private var help: String {
        switch source {
        case "token": "A `claude setup-token` value from a terminal. The router keeps it in the Mac's Keychain."
        case "profile": "A separate Claude Code profile directory on the Mac. You sign in there once with a terminal; the router reads its credential."
        default: "The router runs `claude setup-token` on the Mac and shows you the link to open. Sign in there, and the account appears here."
        }
    }

    private func phaseText(_ l: LoginStatus) -> String {
        switch l.phase {
        case "starting": "starting…"
        case "awaiting_sign_in": "waiting for you to sign in"
        case "done": "signed in as \(l.email ?? l.label ?? "")"
        case "failed": "failed"
        default: l.error ?? "—"
        }
    }

    private func go() async {
        working = true
        defer { working = false }
        let name = label.trimmingCharacters(in: .whitespaces)
        if source == "signin" {
            login = await model.loginStart(label: name)
            if login?.error == nil { polling = true } else { message = login?.error }
            return
        }
        if let p = await model.add(label: name, source: source, token: source == "token" ? token.trimmingCharacters(in: .whitespaces) : nil) {
            message = p
            return
        }
        if source == "profile", let acc = model.accounts.first(where: { $0.label == name }), let pr = await model.probe(acc.id), pr.loggedIn == false {
            message = "Added, but not signed in yet: \(pr.detail ?? "sign in on the Mac with `claude` in that profile")."
            return
        }
        dismiss()
    }
}
