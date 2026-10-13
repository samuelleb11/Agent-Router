import SwiftUI

/// Settings → Clients: each coding client this router can configure, and the form that previews,
/// checks, applies or restores its configuration, or shares it to install elsewhere. On a hosted
/// router, a connected machine can be chosen, whose Claude Code and Codex are set up there.
struct ClientsView: View {
    let state: RouterState
    @State private var model: ClientSetupState?
    @State private var chosen: String?

    var body: some View {
        List {
            Section {
                Text(ClientSetup.explain)
                if model?.setup?.mode == "server" { Text(ClientSetup.hostedNote).foregroundStyle(.secondary) }
            }
            .font(.footnote)
            if let f = model?.failure { Section { Label(f, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange) } }
            if let model, model.setup?.mode == "server" {
                Section {
                    Picker("Machine", selection: Binding(get: { model.machine }, set: { id in Task { await model.choose(id) } })) {
                        Text("None — export").tag(String?.none)
                        ForEach(model.machines) { Text("\($0.label) · \($0.platform)").tag(Optional($0.id)) }
                    }
                    if let f = model.thereFailure { Label(f, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange).font(.footnote) }
                } footer: {
                    Text(model.machines.isEmpty ? "No machine is connected to this router now." : ClientSetup.machineNote)
                }
            }
            if let model, let setup = shown(model) {
                if setup.clients.isEmpty {
                    Section { Text("No supported clients were reported by this router.").foregroundStyle(.secondary) }
                } else {
                    let client = setup.clients.first { $0.id == chosen } ?? setup.clients[0]
                    let label = model.machine.map(model.label)
                    Section {
                        Picker("Client", selection: Binding(get: { client.id }, set: { chosen = $0 })) {
                            ForEach(setup.clients) { Text($0.name).tag($0.id) }
                        }
                        .pickerStyle(.segmented)
                        LabeledContent("Status", value: ClientSetup.status(client))
                        LabeledContent("Installation", value: ClientSetup.installation(client, hosted: setup.mode == "server", machine: label))
                        if let p = client.configPath { LabeledContent("Configuration", value: p).font(.caption) }
                    }
                    ConfigureClientSections(model: model, setup: setup, client: client,
                                            machine: client.canApply ? model.machine.map { (id: $0, label: model.label($0)) } : nil)
                        .id("\(client.id):\(model.machine ?? "")")
                }
            } else if model?.machine != nil, model?.thereFailure == nil {
                Section { ProgressView("Reading the machine’s clients…") }
            }
        }
        .navigationTitle("Clients")
        .overlay { if model?.setup == nil, model?.failure == nil { ProgressView() } }
        .refreshable { await model?.load() }
        .animation(.default, value: model?.machine)
        .task {
            let m = ClientSetupState(router: state)
            model = m
            await m.load()
        }
    }

    /// What the page is about: the router's own reading, else the chosen machine's once it is in.
    private func shown(_ model: ClientSetupState) -> ClientSetupReply? {
        guard let id = model.machine else { return model.setup }
        return model.there?.machine == id ? model.there : nil
    }
}

/// The form for one client. Its token and the files it makes stay in this view's state only:
/// leaving the page or choosing another client clears them. With `machine`, every request names
/// the hosted router's chosen machine, whose agent reads and writes this client's settings.
private struct ConfigureClientSections: View {
    let model: ClientSetupState
    let setup: ClientSetupReply
    let client: SetupClient
    let machine: (id: String, label: String)?
    @State private var chosenModel = ""
    @State private var url = ""
    @State private var token = ""
    @State private var project = ""
    @State private var preview: ClientPreview?
    @State private var checks: [ClientCheck]?
    @State private var busy = false
    @State private var message: (text: String, ok: Bool)?
    @State private var confirm: String?

    private var local: Bool { setup.mode == "local" }
    /// Where Apply and Restore write: this router's machine, else the chosen one.
    private var writes: Bool { local || machine != nil }
    private var selected: SetupModel? { setup.models.first { $0.id == chosenModel } }
    private var ready: Bool { selected?.available == true && !url.trimmingCharacters(in: .whitespaces).isEmpty }
    private var input: ClientInput { ClientSetup.input(model: chosenModel, url: url, token: token, project: project) }

    var body: some View {
        Section {
            Picker("Model", selection: $chosenModel) {
                ForEach(setup.models) { Text(ClientSetup.modelLabel($0)).tag($0.id) }
            }
            .pickerStyle(.navigationLink)
            TextField("Router URL", text: $url).keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
            SecureField("Client access token", text: $token)
            if ClientSetup.asksProject(local: local, client: client.id) {
                TextField("Project directory (optional)", text: $project, prompt: Text("/path/to/project"))
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
            }
        } header: {
            Text("Set up \(client.name)")
        } footer: {
            VStack(alignment: .leading, spacing: 4) {
                if setup.models.allSatisfy({ !$0.available }) { Text(ClientSetup.noCredential) }
                Text("The exact model ID is sent to the router. Use an address reachable from the machine running the client.")
                Text(ClientSetup.tokenNote)
            }
        }
        .onChange(of: chosenModel) { _, _ in reset() }
        .onChange(of: url) { _, _ in reset() }
        .onChange(of: token) { _, _ in reset() }
        .onChange(of: project) { _, _ in reset() }
        .onAppear {
            if url.isEmpty { url = setup.url }
            if chosenModel.isEmpty { chosenModel = (setup.models.first(where: \.available) ?? setup.models.first)?.id ?? "" }
        }
        let limits = ClientSetup.limits(client: client.id, provider: selected?.provider, nativeProtocol: selected?.nativeProtocol, capabilities: selected?.capabilities)
        if !limits.isEmpty {
            Section("Compatibility") { ForEach(limits, id: \.self) { Text($0).font(.footnote) } }
        }
        Section {
            Button("Preview configuration") { Task { await run("preview") } }.disabled(busy || !ready)
            Button("Check connection") { Task { await run("check") } }.disabled(busy || !ready)
            if ClientSetup.offersRestore(writes: writes, client: client, project: project) {
                Button("Restore previous configuration", role: .destructive) { confirm = "restore" }.disabled(busy)
            }
            if let message { Text(message.text).font(.footnote).foregroundStyle(message.ok ? Color.secondary : Color.red) }
        } footer: {
            Text(ClientSetup.checkNote)
        }
        .confirmationDialog(confirm == "apply" ? "Apply \(client.name) configuration\(at)?" : "Restore \(client.name) configuration\(at)?",
                            isPresented: Binding(get: { confirm != nil }, set: { if !$0 { confirm = nil } }), titleVisibility: .visible) {
            Button(confirm == "apply" ? "Apply configuration" : "Restore configuration") {
                let act = confirm
                Task { if act == "apply" { await apply() } else { await restore() } }
            }
        }
        if let checks {
            Section("Connection check") {
                ForEach(checks) { c in
                    VStack(alignment: .leading, spacing: 2) {
                        Text("\(c.label): \(c.status == "pass" ? "Passed" : c.status == "fail" ? "Failed" : "Needs attention")")
                            .foregroundStyle(c.status == "pass" ? Color.green : c.status == "fail" ? Color.red : Color.orange)
                        Text(c.detail).font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        }
        if let preview, let content = preview.content {
            Section {
                Text(content).font(.caption.monospaced()).textSelection(.enabled)
                Button("Copy configuration", systemImage: "doc.on.doc") { UIPasteboard.general.string = content }
                if let file = Self.file(ClientSetup.filename(client: client.id, format: preview.format), content) {
                    ShareLink(item: file) { Label("Share \(file.lastPathComponent)", systemImage: "square.and.arrow.up") }
                }
                if let catalogue = preview.catalogueContent, let file = Self.file(ClientSetup.catalogueFilename, catalogue) {
                    ShareLink(item: file) { Label("Share the Codex model catalogue", systemImage: "square.and.arrow.up") }
                }
                if ClientSetup.offersApply(writes: writes, client: client, preview: preview) {
                    Button("Apply configuration") { confirm = "apply" }.disabled(busy)
                }
                Button("Clear preview and token") { self.preview = nil; token = "" }
            } header: {
                Text("Configuration")
            } footer: {
                Text((preview.path.map { "Settings to add to \($0)\(at). Applying merges them into your existing configuration. " } ?? "Merge these settings into \(client.name)’s configuration on the machine running the client. ")
                     + (token.isEmpty ? "If this client runs on another machine, replace the token placeholder with a Client access token." : "This configuration contains your router access token. Save it as a private client configuration."))
            }
        }
    }

    private func reset() { preview = nil; checks = nil; message = nil }

    /// " on studio", for a chosen machine.
    private var at: String { machine.map { " on \($0.label)" } ?? "" }

    private func run(_ act: String) async {
        busy = true
        defer { busy = false }
        message = nil
        switch await model.run(act, client: client.id, input: input, machine: machine?.id) {
        case .success(let p):
            checks = p.checks
            if act == "preview" { preview = p }
            if act == "check", !p.warnings.isEmpty { message = (p.warnings.joined(separator: " "), true) }
        case .failure(let e): message = (e.errorDescription ?? "Refused", false)
        }
    }

    private func apply() async {
        guard let preview else { return }
        busy = true
        defer { busy = false }
        switch await model.apply(client: client.id, input: ClientSetup.input(model: chosenModel, url: url, token: token, project: project, revision: preview.revision), machine: machine?.id) {
        case .success(let a): message = (ClientSetup.appliedWords(a, name: client.name, machine: machine?.label), true); await model.load()
        case .failure(let e): message = (e.errorDescription ?? "Refused", false)
        }
    }

    private func restore() async {
        busy = true
        defer { busy = false }
        switch await model.restore(client: client.id, project: project, machine: machine?.id) {
        case .success(let r): message = (ClientSetup.restoredWords(r), true); self.preview = nil; await model.load()
        case .failure(let e): message = (e.errorDescription ?? "Refused", false)
        }
    }

    /// A configuration written to a file under its name, for the share sheet to hand on.
    private static func file(_ name: String, _ content: String) -> URL? {
        let url = FileManager.default.temporaryDirectory.appending(path: name)
        return (try? content.write(to: url, atomically: true, encoding: .utf8)) != nil ? url : nil
    }
}
