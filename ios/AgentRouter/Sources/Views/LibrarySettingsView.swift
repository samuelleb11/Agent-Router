import SwiftUI

/// Settings → Forges: where the Library imports from, each forge checked, changed and removed.
struct ForgesSettingsView: View {
    let state: RouterState
    @State private var model: LibrarySettingsState?
    @State private var editing: ForgeRow?
    @State private var adding = false
    @State private var removing: ForgeRow?
    @State private var said: [String: (text: String, ok: Bool)] = [:]

    var body: some View {
        List {
            Section { Text(LibrarySettings.forgesExplain).font(.footnote) }
            if let f = model?.failure { Section { Label(f, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange) } }
            if let forges = model?.forges, let model {
                Section("Forges") {
                    if forges.isEmpty { Text("No forge yet.").foregroundStyle(.secondary) }
                    ForEach(forges) { f in
                        VStack(alignment: .leading, spacing: 3) {
                            HStack { Text(f.key).font(.body.monospaced()); Text(LibrarySettings.kindLabel(f.kind)).font(.caption).foregroundStyle(.secondary) }
                            Text(f.baseURL).font(.caption.monospaced()).foregroundStyle(.secondary)
                            Text("\(f.tokenPresent ? "a token is kept" : "no token") · \(f.sources) source\(f.sources == 1 ? "" : "s")").font(.caption).foregroundStyle(.secondary)
                            HStack(spacing: 16) {
                                Button("Check") {
                                    Task {
                                        switch await model.call({ try await $0.post(LibrarySettings.forgePath(f.key, "/check"), [String: String]()) as ForgeChecked }) {
                                        case .success(let c): said[f.key] = (LibrarySettings.checked(c), c.result == "ok")
                                        case .failure(let e): said[f.key] = (e.errorDescription ?? "Refused", false)
                                        }
                                    }
                                }
                                Button("Change") { editing = f }
                                Button("Remove", role: .destructive) { removing = f }
                            }
                            .buttonStyle(.borderless).font(.caption.weight(.medium))
                            if let s = said[f.key] { Text(s.text).font(.caption).foregroundStyle(s.ok ? Color.green : Color.red) }
                        }
                    }
                    Button("Add a forge", systemImage: "plus") { adding = true }
                }
                .sheet(isPresented: $adding) { ForgeForm(model: model, forge: nil) }
                .sheet(item: $editing) { ForgeForm(model: model, forge: $0) }
                .confirmationDialog("Remove the forge \(removing?.key ?? "")?", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }), titleVisibility: .visible) {
                    Button("Remove", role: .destructive) {
                        if let f = removing {
                            Task {
                                if case .failure(let e) = await model.call({ try await $0.delete(LibrarySettings.forgePath(f.key)) as OkReply }) {
                                    said[f.key] = (e.errorDescription ?? "Refused", false)
                                }
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle("Forges")
        .overlay { if model?.forges == nil, model?.failure == nil { ProgressView() } }
        .refreshable { await model?.load() }
        .task {
            let m = LibrarySettingsState(router: state)
            model = m
            await m.load()
        }
    }
}

private struct ForgeForm: View {
    let model: LibrarySettingsState
    let forge: ForgeRow?
    @Environment(\.dismiss) private var dismiss
    @State private var key = ""
    @State private var kind = "gitlab"
    @State private var url = ""
    @State private var token = ""
    @State private var clear = false
    @State private var message: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Key", text: $key).textInputAutocapitalization(.never).autocorrectionDisabled().disabled(forge != nil)
                    Picker("Kind", selection: $kind) { ForEach(LibrarySettings.forgeKinds, id: \.0) { Text($0.1).tag($0.0) } }
                    TextField("Address", text: $url, prompt: Text("https://gitlab.example.com")).keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
                    SecureField("Token", text: $token).disabled(clear)
                    if forge?.tokenPresent == true { Toggle("Remove the token", isOn: $clear) }
                } footer: {
                    Text(forge?.tokenPresent == true ? "Leave the token empty to keep the token kept now. GitLab needs read_api; GitHub, read access to contents."
                         : "Lowercase letters and digits, with single hyphens between them, for the key. GitLab needs read_api; GitHub, read access to contents. A public GitHub repository needs none.")
                }
                if let message { Section { Text(message).font(.footnote).foregroundStyle(.red) } }
            }
            .navigationTitle(forge == nil ? "Add a forge" : "Change the forge \(forge!.key)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        let t = token.trimmingCharacters(in: .whitespaces)
                        let body = ForgeBody(kind: kind, baseURL: url.trimmingCharacters(in: .whitespaces), token: clear ? .remove : t.isEmpty ? .keep : .set(t))
                        Task {
                            switch await model.call({ try await $0.put(LibrarySettings.forgePath(key), body) as OkReply }) {
                            case .success: dismiss()
                            case .failure(let e): message = e.errorDescription
                            }
                        }
                    }
                    .disabled(!LibrarySettings.isKey(key) || url.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            .onAppear { if let f = forge { key = f.key; kind = f.kind; url = f.baseURL } }
        }
    }
}

/// Settings → Library: its sources, publish tokens and whether a machine's publish goes live.
struct LibrarySettingsView: View {
    let state: RouterState
    @State private var model: LibrarySettingsState?
    @State private var addingSource = false
    @State private var editingSource: SourceRow?
    @State private var removingSource: SourceRow?
    @State private var addingToken = false
    @State private var madeToken: PublishTokenMade?
    @State private var revoking: PublishToken?
    @State private var secret: SecretReply?
    @State private var said: [String: (text: String, ok: Bool)] = [:]

    var body: some View {
        List {
            if let f = model?.failure { Section { Label(f, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange) } }
            if let model {
                sourcesSection(model)
                tokensSection(model)
                Section {
                    Toggle("Publishes from machines go live", isOn: Binding(
                        get: { model.setting?.machinePublishes == "live" },
                        set: { live in
                            Task { _ = await model.call({ try await $0.put("/api/library/settings", LibrarySetting(machinePublishes: live ? "live" : "approval")) as OkReply }) }
                        }))
                    .disabled(model.setting == nil)
                } header: { Text("Publishes from machines") } footer: { Text(LibrarySettings.machinesExplain) }
            }
        }
        .navigationTitle("Library")
        .overlay { if model?.sources == nil, model?.failure == nil { ProgressView() } }
        .refreshable { await model?.load() }
        .sheet(item: $secret) { s in SecretSheet(secret: s, origin: state.router.origin) }
        .sheet(item: $madeToken) { TokenMadeSheet(token: $0) }
        .task {
            let m = LibrarySettingsState(router: state)
            model = m
            await m.load()
        }
    }

    @ViewBuilder private func sourcesSection(_ model: LibrarySettingsState) -> some View {
        Section {
            ForEach(model.sources ?? []) { s in
                VStack(alignment: .leading, spacing: 3) {
                    Text(s.key).font(.body.monospaced())
                    Text("\(s.forge) · \(s.repository) @ \(s.ref)\(s.path.isEmpty ? "" : " · \(s.path)")").font(.caption).foregroundStyle(.secondary)
                    Text("\(s.live ? "new versions go live" : "new versions wait for approval") · \(s.hook ? "a webhook secret is set" : "no webhook secret yet")").font(.caption).foregroundStyle(.secondary)
                    Text(LibrarySettings.lastWords(s)).font(.caption2).foregroundStyle(s.lastResult.map { if case .refused = $0 { return Color.red }; return Color.secondary } ?? Color.secondary)
                    HStack(spacing: 14) {
                        Button("Import now") {
                            Task {
                                switch await model.call({ try await $0.post(LibrarySettings.sourcePath(s.key, "/import"), [String: String]()) as ImportReply }) {
                                case .success(let r): said[s.key] = (LibrarySettings.importWords(r.imported), true)
                                case .failure(let e): said[s.key] = (e.errorDescription ?? "Refused", false)
                                }
                            }
                        }
                        Button(s.hook ? "New webhook secret" : "Make a webhook secret") {
                            Task {
                                switch await model.call({ try await $0.post(LibrarySettings.sourcePath(s.key, "/secret"), [String: String]()) as SecretReply }) {
                                case .success(let r): secret = r
                                case .failure(let e): said[s.key] = (e.errorDescription ?? "Refused", false)
                                }
                            }
                        }
                        Button("Change") { editingSource = s }
                        Button("Remove", role: .destructive) { removingSource = s }
                    }
                    .buttonStyle(.borderless).font(.caption.weight(.medium))
                    if let m = said[s.key] { Text(m.text).font(.caption).foregroundStyle(m.ok ? Color.secondary : Color.red) }
                }
            }
            Button("Add a source", systemImage: "plus") { addingSource = true }.disabled(model.forges?.isEmpty != false)
        } header: {
            Text("Sources")
        } footer: {
            Text(LibrarySettings.sourcesExplain)
        }
        .sheet(isPresented: $addingSource) { SourceForm(model: model, source: nil) }
        .sheet(item: $editingSource) { SourceForm(model: model, source: $0) }
        .confirmationDialog("Remove the source \(removingSource?.key ?? "")?", isPresented: Binding(get: { removingSource != nil }, set: { if !$0 { removingSource = nil } }), titleVisibility: .visible) {
            Button("Remove", role: .destructive) {
                if let s = removingSource { Task { _ = await model.call({ try await $0.delete(LibrarySettings.sourcePath(s.key)) as OkReply }) } }
            }
        }
    }

    @ViewBuilder private func tokensSection(_ model: LibrarySettingsState) -> some View {
        Section {
            ForEach(model.tokens ?? []) { t in
                VStack(alignment: .leading, spacing: 3) {
                    HStack { Text(t.name); Spacer(); Text(t.revokedAt == nil ? "active" : "revoked").font(.caption2).foregroundStyle(t.revokedAt == nil ? Color.green : Color.secondary) }
                    Text("may publish \(LibrarySettings.scopeWords(t)) · \(t.live ? "goes live" : "waits for approval")").font(.caption).foregroundStyle(.secondary)
                }
                .swipeActions { if t.revokedAt == nil { Button("Revoke", role: .destructive) { revoking = t } } }
            }
            Button("Make a publish token", systemImage: "plus") { addingToken = true }
        } header: {
            Text("Publish tokens")
        } footer: {
            Text(LibrarySettings.tokensExplain)
        }
        .sheet(isPresented: $addingToken) { PublishTokenForm(model: model) { madeToken = $0 } }
        .confirmationDialog("Revoke the token \(revoking?.name ?? "")?", isPresented: Binding(get: { revoking != nil }, set: { if !$0 { revoking = nil } }), titleVisibility: .visible) {
            Button("Revoke", role: .destructive) {
                if let t = revoking { Task { _ = await model.call({ try await $0.delete("/api/library/tokens/" + RouterClient.segment(t.id)) as OkReply }) } }
            }
        }
    }
}

extension SecretReply: Identifiable { var id: String { secret } }

private struct SourceForm: View {
    let model: LibrarySettingsState
    let source: SourceRow?
    @Environment(\.dismiss) private var dismiss
    @State private var key = ""
    @State private var forge = ""
    @State private var repository = ""
    @State private var ref = "main"
    @State private var path = ""
    @State private var live = false
    @State private var message: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Key", text: $key).textInputAutocapitalization(.never).autocorrectionDisabled().disabled(source != nil)
                    Picker("Forge", selection: $forge) { ForEach(model.forges ?? []) { Text($0.key).tag($0.key) } }
                    TextField("Repository", text: $repository, prompt: Text("group/project")).textInputAutocapitalization(.never).autocorrectionDisabled()
                    TextField("Ref", text: $ref).textInputAutocapitalization(.never).autocorrectionDisabled()
                    TextField("Path", text: $path, prompt: Text("empty for the whole of it")).textInputAutocapitalization(.never).autocorrectionDisabled()
                    Toggle("Its new versions go live", isOn: $live)
                } footer: {
                    Text("group/project on GitLab, owner/repo on GitHub; a branch, a tag or a commit.")
                }
                if let message { Section { Text(message).font(.footnote).foregroundStyle(.red) } }
            }
            .navigationTitle(source == nil ? "Add a source" : "Change the source")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        let body = SourceBody(forge: forge, repository: repository.trimmingCharacters(in: .whitespaces), ref: ref.trimmingCharacters(in: .whitespaces),
                                              path: path.trimmingCharacters(in: .whitespaces), live: live)
                        Task {
                            switch await model.call({ try await $0.put(LibrarySettings.sourcePath(key), body) as OkReply }) {
                            case .success: dismiss()
                            case .failure(let e): message = e.errorDescription
                            }
                        }
                    }
                    .disabled(!LibrarySettings.isKey(key) || forge.isEmpty || repository.trimmingCharacters(in: .whitespaces).isEmpty || ref.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            .onAppear {
                if let s = source { key = s.key; forge = s.forge; repository = s.repository; ref = s.ref; path = s.path; live = s.live }
                else if forge.isEmpty { forge = model.forges?.first?.key ?? "" }
            }
        }
    }
}

private struct PublishTokenForm: View {
    let model: LibrarySettingsState
    let made: (PublishTokenMade) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var kinds: Set<String> = []
    @State private var items = ""
    @State private var live = false
    @State private var message: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $name)
                    ForEach(Library.kinds, id: \.self) { k in
                        Toggle(k, isOn: Binding(get: { kinds.contains(k) }, set: { if $0 { kinds.insert(k) } else { kinds.remove(k) } }))
                    }
                    TextField("Only these items", text: $items, prompt: Text("skill/bench, command/deploy")).textInputAutocapitalization(.never).autocorrectionDisabled()
                    Toggle("What it publishes goes live", isOn: $live)
                } footer: {
                    Text("Versions it publishes are signed with it. Items as kind/name, separated by commas; empty for any.")
                }
                if let message { Section { Text(message).font(.footnote).foregroundStyle(.red) } }
            }
            .navigationTitle("Make a publish token")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Make") {
                        let body = PublishTokenBody(name: name.trimmingCharacters(in: .whitespaces), kinds: Library.kinds.filter(kinds.contains),
                                                    items: LibrarySettings.itemList(items), live: live)
                        Task {
                            switch await model.call({ try await $0.post("/api/library/tokens", body) as PublishTokenMade }) {
                            case .success(let t): dismiss(); made(t)
                            case .failure(let e): message = e.errorDescription
                            }
                        }
                    }
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
        }
    }
}

/// A source's webhook, shown once: the URL and the secret to give the forge.
private struct SecretSheet: View {
    let secret: SecretReply
    let origin: String
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        let url = secret.url ?? origin + secret.path
        NavigationStack {
            Form {
                Section {
                    Text(LibrarySettings.secretExplain).font(.footnote)
                    LabeledContent("URL") { Text(url).font(.caption.monospaced()).textSelection(.enabled) }
                    LabeledContent("Secret") { Text(secret.secret).font(.caption.monospaced()).textSelection(.enabled) }
                    Button("Copy the URL", systemImage: "doc.on.doc") { UIPasteboard.general.string = url }
                    Button("Copy the secret", systemImage: "doc.on.doc") { UIPasteboard.general.string = secret.secret }
                }
            }
            .navigationTitle("The webhook")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
            .interactiveDismissDisabled()
        }
    }
}

/// A publish token just made, in full, this once.
private struct TokenMadeSheet: View {
    let token: PublishTokenMade
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text("This is the only time the token is shown. Give it to the pipeline or session that publishes with it.").font(.footnote)
                    Text(token.token).font(.body.monospaced()).textSelection(.enabled)
                    Button("Copy the token", systemImage: "doc.on.doc") { UIPasteboard.general.string = token.token }
                }
            }
            .navigationTitle(token.name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("I have saved the token") { dismiss() } } }
            .interactiveDismissDisabled()
        }
    }
}
