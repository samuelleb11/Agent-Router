import SwiftUI

/// Add or edit a router. The password is the one set on the router's dashboard, or by a
/// hosted router's deployment.
struct RouterFormView: View {
    enum Mode { case add, edit(Router) }

    let mode: Mode
    @Environment(RouterStore.self) private var store
    @Environment(\.dismiss) private var dismiss

    @State private var name = ""
    @State private var host = ""
    @State private var port = "8787"
    @State private var password = ""
    @State private var testing = false
    @State private var testResult: (ok: Bool, text: String)?
    /// What the router said about itself when this form last tested the address in it.
    @State private var tested: AuthStatus?

    private var editing: Router? {
        if case .edit(let r) = mode { return r }
        return nil
    }

    /// Whether the router in the form says it is hosted: from this form's own connection
    /// test, else — for a router being edited, still at its saved address — from its page.
    /// Unknown reads as local, which is what every router was before hosting.
    private var hosted: Bool {
        if let tested { return tested.isHosted }
        guard let r = editing, draft?.origin == r.origin else { return false }
        return store.state(id: r.id)?.auth?.isHosted == true
    }

    private var entry: Router.Entry { Router.entry(host: host, port: port) }
    /// A whole URL brings its own port, so the Port field steps aside.
    private var hostIsURL: Bool { host.contains("://") }
    /// The router as Save would store it, and what the connection test calls.
    private var draft: Router? {
        guard case .valid(let scheme, let validHost, let validPort) = entry else { return nil }
        var router = editing ?? Router(name: "", host: "")
        router.name = name.trimmingCharacters(in: .whitespaces)
        router.scheme = scheme
        router.host = validHost
        router.port = validPort
        return router
    }
    private var valid: Bool {
        !name.trimmingCharacters(in: .whitespaces).isEmpty && draft != nil
            && (editing != nil || !password.isEmpty)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $name)
                        .textContentType(.name)
                        .accessibilityIdentifier("form-name")
                    TextField("Host", text: $host, prompt: Text("10.0.0.5 or an https:// URL"))
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .accessibilityIdentifier("form-host")
                    if !hostIsURL {
                        TextField("Port", text: $port)
                            .keyboardType(.numberPad)
                            .accessibilityIdentifier("form-port")
                    }
                    switch entry {
                    case .empty:
                        EmptyView()
                    case .invalid(let why):
                        Label(why, systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                            .font(.callout)
                            .accessibilityIdentifier("form-address")
                    case .valid:
                        LabeledContent("Connects to", value: draft?.origin ?? "")
                            .font(.callout)
                            .accessibilityIdentifier("form-address")
                    }
                } header: {
                    Text("Router")
                } footer: {
                    Text("The Mac running Agent Router, as this phone can reach it. Turn on Listen on your network in the router's dashboard settings first. A hosted router is added by its https URL, such as `https://agent-router.example.com`.")
                }
                Section {
                    SecureField(editing == nil ? "Router credential" : "New credential (leave blank to keep)", text: $password)
                        .textContentType(.password)
                        .accessibilityIdentifier("form-password")
                } header: {
                    Text("Router credential")
                } footer: {
                    Text("Use an Administrator access token from Settings → Access tokens so this phone can read and control sessions. The router password also works. Kept in this phone's Keychain.")
                }
                Section {
                    Button {
                        Task { await test() }
                    } label: {
                        HStack {
                            Label("Test connection", systemImage: "antenna.radiowaves.left.and.right")
                            if testing { Spacer(); ProgressView() }
                        }
                    }
                    .disabled(testing || draft == nil)
                    if let testResult {
                        Label(testResult.text, systemImage: testResult.ok ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                            .foregroundStyle(testResult.ok ? .green : .orange)
                            .font(.callout)
                    }
                }
            }
            .navigationTitle(editing == nil ? "Add router" : "Edit router")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { save() }.disabled(!valid).accessibilityIdentifier("form-save")
                }
            }
            .onAppear {
                if let r = editing {
                    name = r.name
                    host = r.scheme == .https ? r.origin : r.host
                    port = String(r.port)
                }
            }
            // What a test said is about the address it tested.
            .onChange(of: entry) { tested = nil }
        }
    }

    private func save() {
        guard let router = draft else { return }
        if editing == nil { store.add(router, password: password) } else { store.update(router, password: password) }
        dismiss()
    }

    /// Health first — it needs no password, so it separates "wrong address" from
    /// "wrong password" — then the auth status with the credential.
    private func test() async {
        testing = true
        defer { testing = false }
        tested = nil
        guard let url = draft?.baseURL else {
            testResult = (false, "That is not a usable address.")
            return
        }
        let pw = password.isEmpty ? editing.flatMap { store.password(for: $0) } : password
        let client = RouterClient(baseURL: url, password: pw)
        do {
            let health: Health = try await client.get("/_router/health")
            let auth: AuthStatus = try await client.get("/api/auth")
            tested = auth
            testResult = (true, "Agent Router \(health.version) answered\(auth.fromLoopback ? "" : ", and the router credential was accepted").")
        } catch let e as RouterClient.Failure {
            testResult = (false, e.errorDescription ?? "Failed.")
        } catch {
            testResult = (false, error.localizedDescription)
        }
    }
}
