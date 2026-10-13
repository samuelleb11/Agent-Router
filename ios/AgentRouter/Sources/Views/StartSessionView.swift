import SwiftUI

/// Start a session the router hosts, as the console's form does: the client, the machine on a
/// hosted router, the directory there and the model. The session it starts opens at once.
struct StartSessionSheet: View {
    let state: RouterState
    @Environment(RouterStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var client = SessionStart.firstClient
    @State private var machineRows: [MachineRow] = []
    @State private var catalogue: [CatalogueModel] = []
    @State private var machine = ""
    @State private var cwd = ""
    @State private var model = ""
    @State private var working = false
    @State private var message: String?

    private var hosted: Bool { state.auth?.isHosted == true }
    private var machines: [SessionStart.Machine] { SessionStart.machines(machineRows, client: client) }
    private var models: [CatalogueModel] { SessionStart.models(catalogue, client: client) }
    private var valid: Bool {
        (!hosted || machines.contains { $0.id == machine }) && !cwd.trimmingCharacters(in: .whitespaces).isEmpty
            && models.contains { $0.id == model }
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    if SessionStart.clients.count > 1 {
                        Picker("Client", selection: $client) {
                            ForEach(SessionStart.clients, id: \.self) { Text(SessionClient.productName($0)).tag($0) }
                        }
                    }
                    if hosted {
                        Picker("Machine", selection: $machine) {
                            if machines.isEmpty { Text("none").tag("") }
                            ForEach(machines) { Text($0.label).tag($0.id) }
                        }
                        .accessibilityIdentifier("start-machine")
                    }
                    TextField("Directory", text: $cwd, prompt: Text("/Users/you/code/project"))
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                        .accessibilityIdentifier("start-directory")
                    Picker("Model", selection: $model) {
                        if models.isEmpty { Text("none").tag("") }
                        ForEach(models) { Text($0.name).tag($0.id) }
                    }
                    .accessibilityIdentifier("start-model")
                } footer: {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(SessionStart.why)
                        if hosted && machines.isEmpty { Text(SessionStart.noMachine(client)) }
                        if !catalogue.isEmpty && models.isEmpty { Text(SessionStart.noModel(client)) }
                        Text("The directory is its full path on that machine.")
                    }
                }
                if let message { Section { Text(message).font(.footnote).foregroundStyle(.red) } }
            }
            .navigationTitle(SessionStart.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Start") { Task { await start() } }
                        .disabled(!valid || working)
                        .accessibilityIdentifier("start-go")
                }
            }
            .onChange(of: client) { _, _ in machine = machines.first?.id ?? ""; model = models.first?.id ?? "" }
            .task { await keepCurrent() }
        }
    }

    /// The catalogue once, and the machines every five seconds while the form is open, so one
    /// that comes online can be chosen without closing it.
    private func keepCurrent() async {
        guard let api = state.liveClient else { return }
        do { catalogue = try await api.get("/api/models") } catch { message = LiveFeed.describe(error) }
        if model.isEmpty { model = models.first?.id ?? "" }
        guard hosted else { return }
        while !Task.isCancelled {
            do {
                machineRows = try await api.get("/api/machines")
                if !machines.contains(where: { $0.id == machine }) { machine = machines.first?.id ?? "" }
            } catch {
                message = LiveFeed.describe(error)
            }
            do { try await Task.sleep(for: .seconds(5)) } catch { return }
        }
    }

    private func start() async {
        guard let api = state.liveClient else { return }
        working = true
        defer { working = false }
        do {
            let reply: StartReply = try await api.post("/api/live/start",
                SessionStart.body(hosted: hosted, machine: machine, cwd: cwd, model: model, client: client))
            guard let id = reply.session, !id.isEmpty else { message = "The router started nothing it named."; return }
            await state.refreshSessions()
            store.pendingOpen = PendingOpen(routerId: state.router.id, sessionId: id)
            dismiss()
        } catch {
            message = LiveFeed.describe(error)
        }
    }
}
