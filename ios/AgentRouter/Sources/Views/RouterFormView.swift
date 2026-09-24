import SwiftUI

/// Add or edit a router. The password is the one set on the router's dashboard.
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

    private var editing: Router? {
        if case .edit(let r) = mode { return r }
        return nil
    }

    private var portValue: Int? { Int(port.trimmingCharacters(in: .whitespaces)) }
    private var hostValue: String { host.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var valid: Bool {
        !name.trimmingCharacters(in: .whitespaces).isEmpty && !hostValue.isEmpty
            && (portValue.map { (1...65535).contains($0) } ?? false)
            && (editing != nil || !password.isEmpty)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $name)
                        .textContentType(.name)
                        .accessibilityIdentifier("form-name")
                    TextField("Host", text: $host, prompt: Text("10.0.0.5 or mac.local"))
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .accessibilityIdentifier("form-host")
                    TextField("Port", text: $port)
                        .keyboardType(.numberPad)
                        .accessibilityIdentifier("form-port")
                } header: {
                    Text("Router")
                } footer: {
                    Text("The Mac running Agent Router, as this phone can reach it. Turn on Listen on your network in the router's dashboard settings first.")
                }
                Section {
                    SecureField(editing == nil ? "Password" : "New password (leave blank to keep)", text: $password)
                        .textContentType(.password)
                        .accessibilityIdentifier("form-password")
                } header: {
                    Text("Password")
                } footer: {
                    Text("Set on the router's dashboard, on the Mac. Kept in this phone's Keychain.")
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
                    .disabled(testing || hostValue.isEmpty || portValue == nil)
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
                    host = r.host
                    port = String(r.port)
                }
            }
        }
    }

    private func save() {
        guard let portValue else { return }
        var router = editing ?? Router(name: "", host: "")
        router.name = name.trimmingCharacters(in: .whitespaces)
        router.host = hostValue
        router.port = portValue
        if editing == nil { store.add(router, password: password) } else { store.update(router, password: password) }
        dismiss()
    }

    /// Health first — it needs no password, so it separates "wrong address" from
    /// "wrong password" — then the auth status with the credential.
    private func test() async {
        testing = true
        defer { testing = false }
        guard let portValue, let url = URL(string: "http://\(hostValue):\(portValue)") else {
            testResult = (false, "That is not a usable address.")
            return
        }
        let pw = password.isEmpty ? editing.flatMap { store.password(for: $0) } : password
        let client = RouterClient(baseURL: url, password: pw)
        do {
            let health: Health = try await client.get("/_router/health")
            let auth: AuthStatus = try await client.get("/api/auth")
            testResult = (true, "Agent Router \(health.version) answered\(auth.fromLoopback ? "" : ", and the password is right").")
        } catch let e as RouterClient.Failure {
            testResult = (false, e.errorDescription ?? "Failed.")
        } catch {
            testResult = (false, error.localizedDescription)
        }
    }
}
