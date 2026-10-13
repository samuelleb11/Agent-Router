import SwiftUI

/// Model routes and provider services, under Models: each listed, added, edited and removed with
/// the console's rules; built-in services read-only.
struct RegistrySections: View {
    let state: RouterState
    @State private var model: RegistryState?
    @State private var editingService: ProviderRegistry.Service?
    @State private var addingService = false
    @State private var editingRoute: ProviderRegistry.Route?
    @State private var addingRoute = false
    @State private var removing: (kind: String, id: String)?
    @State private var notice: String?

    var body: some View {
        Group {
            if let reg = model?.registry, let model {
                Section {
                    ForEach(reg.models) { r in
                        VStack(alignment: .leading, spacing: 3) {
                            Text(r.alias).font(.body.monospaced())
                            if let d = r.displayName { Text(d).font(.caption) }
                            Text("\(r.service) · \(r.upstreamModel) · \(r.accountId ?? "—")").font(.caption).foregroundStyle(.secondary)
                            Text(Registry.capabilityWords(r.capabilities)).font(.caption2).foregroundStyle(.secondary)
                            HStack(spacing: 16) {
                                Button("Edit") { editingRoute = r }
                                Button("Remove", role: .destructive) { removing = ("route", r.alias) }
                            }
                            .buttonStyle(.borderless).font(.caption.weight(.medium))
                        }
                    }
                    Button("Add model route", systemImage: "plus") { addingRoute = true }
                        .accessibilityIdentifier("add-route")
                } header: {
                    Text("Model routes")
                } footer: {
                    Text(Registry.routesExplain)
                }
                Section {
                    ForEach(reg.services) { s in
                        VStack(alignment: .leading, spacing: 3) {
                            HStack(spacing: 6) {
                                ProviderMark(provider: s.provider, form: .alone).font(.caption)
                                Text(s.label)
                                Text(s.id).font(.caption.monospaced()).foregroundStyle(.secondary)
                            }
                            Text(s.baseURL).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                            Text("\(Registry.protocolName(s.protocol)) · \(Registry.authName(s.authProfile))").font(.caption).foregroundStyle(.secondary)
                            Text(Registry.capabilityWords(s.capabilities)).font(.caption2).foregroundStyle(.secondary)
                            if !Registry.editable(s) {
                                Text(Registry.builtinNote).font(.caption2).foregroundStyle(.secondary)
                            } else {
                                HStack(spacing: 16) {
                                    Button("Edit") { editingService = s }
                                    if Registry.removable(s, in: reg) {
                                        Button("Remove", role: .destructive) { removing = ("service", s.id) }
                                    }
                                }
                                .buttonStyle(.borderless).font(.caption.weight(.medium))
                                if !Registry.removable(s, in: reg) { Text(Registry.inUseNote).font(.caption2).foregroundStyle(.secondary) }
                            }
                        }
                    }
                    Button("Add provider service", systemImage: "plus") { addingService = true }
                        .accessibilityIdentifier("add-service")
                    if let notice { Text(notice).font(.footnote).foregroundStyle(.red) }
                } header: {
                    Text("Provider services")
                } footer: {
                    Text(Registry.servicesExplain + " " + Registry.capabilitiesExplain)
                }
                .sheet(isPresented: $addingService) { ServiceForm(model: model, initial: nil) }
                .sheet(item: $editingService) { ServiceForm(model: model, initial: $0) }
                .sheet(isPresented: $addingRoute) { RouteForm(model: model, registry: reg, initial: nil) }
                .sheet(item: $editingRoute) { RouteForm(model: model, registry: reg, initial: $0) }
                .confirmationDialog("Remove \(removing?.id ?? "")?", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }), titleVisibility: .visible) {
                    Button("Remove", role: .destructive) {
                        if let r = removing {
                            Task { notice = r.kind == "route" ? await model.remove(route: r.id) : await model.remove(service: r.id) }
                        }
                    }
                }
            } else if let f = model?.failure {
                Section("Model routes") { Text(f).foregroundStyle(.orange) }
            }
        }
        .task {
            let m = model ?? RegistryState(router: state)
            model = m
            await m.load()
        }
    }
}

/// Toggles and limits for a capability set. Compaction is a native Responses route's alone.
private struct CapabilitiesEditor: View {
    @Binding var tools: Bool
    @Binding var images: Bool
    @Binding var reasoning: Bool
    @Binding var compaction: Bool
    @Binding var context: String
    @Binding var output: String
    let responses: Bool

    var body: some View {
        Toggle("Allow tools", isOn: $tools)
        Toggle("Allow images", isOn: $images)
        Toggle("Allow reasoning", isOn: $reasoning)
        Toggle("Allow compaction", isOn: $compaction).disabled(!responses)
        TextField("Context token limit (optional)", text: $context).keyboardType(.numberPad)
        TextField("Output token limit (optional)", text: $output).keyboardType(.numberPad)
        if let p = Registry.tokenLimitProblem(context) ?? Registry.tokenLimitProblem(output) { Text(p).font(.caption).foregroundStyle(.red) }
    }
}

private func limit(_ v: String) -> Int? { Int(v.trimmingCharacters(in: .whitespaces)) }

/// Add or edit a provider service.
private struct ServiceForm: View {
    let model: RegistryState
    let initial: ProviderRegistry.Service?
    @Environment(\.dismiss) private var dismiss
    @State private var id = ""
    @State private var label = ""
    @State private var provider = "openai"
    @State private var url = ""
    @State private var proto = "responses"
    @State private var auth = "bearer"
    @State private var tools = true
    @State private var images = true
    @State private var reasoning = true
    @State private var compaction = false
    @State private var context = ""
    @State private var output = ""
    @State private var message: String?
    @State private var busy = false

    private var input: ServiceInput {
        ServiceInput(id: id, label: label.trimmingCharacters(in: .whitespaces), provider: provider, baseURL: url.trimmingCharacters(in: .whitespaces),
                     protocol: proto, authProfile: auth,
                     capabilities: .init(tools: tools, images: images, reasoning: reasoning, compaction: compaction && proto == "responses",
                                         contextTokens: limit(context), outputTokens: limit(output)))
    }
    private var problem: String? {
        Registry.serviceProblem(input, existing: model.registry, editing: initial != nil) ?? Registry.tokenLimitProblem(context) ?? Registry.tokenLimitProblem(output)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Service ID", text: $id).textInputAutocapitalization(.never).autocorrectionDisabled().disabled(initial != nil)
                    TextField("Service name", text: $label)
                    Picker("Account provider", selection: $provider) { ForEach(Provider.addable, id: \.self) { Text(Provider.name($0)).tag($0) } }
                    TextField("Service base URL", text: $url, prompt: Text("https://provider.example"))
                        .keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
                    Picker("API protocol", selection: $proto) {
                        Text("OpenAI Responses").tag("responses")
                        Text("Anthropic Messages").tag("messages")
                    }
                    .onChange(of: proto) { _, p in if p == "messages" { compaction = false } }
                    Picker("Authentication header", selection: $auth) {
                        Text("Authorization: Bearer").tag("bearer")
                        Text("x-api-key").tag("api_key")
                    }
                } footer: {
                    Text("1–64 lowercase letters, digits, underscores or hyphens for the id; the HTTPS origin or API prefix, without a final /v1. Custom services need API-key accounts. Native subscription logins stay with their built-in service. No credentials are entered in this form.")
                }
                Section("Configured capabilities") {
                    CapabilitiesEditor(tools: $tools, images: $images, reasoning: $reasoning, compaction: $compaction, context: $context, output: $output, responses: proto == "responses")
                }
                if let p = message ?? (id.isEmpty && url.isEmpty ? nil : problem) { Section { Text(p).font(.footnote).foregroundStyle(.red) } }
            }
            .navigationTitle(initial == nil ? "Add provider service" : "Edit provider service")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() }.disabled(busy) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        Task {
                            busy = true
                            defer { busy = false }
                            if let refused = await model.save(input) { message = refused } else { dismiss() }
                        }
                    }
                    .disabled(problem != nil || busy)
                }
            }
            .onAppear {
                guard let s = initial else { return }
                id = s.id; label = s.label; provider = s.provider; url = s.baseURL; proto = s.protocol; auth = s.authProfile
                if let c = s.capabilities {
                    tools = c.tools; images = c.images; reasoning = c.reasoning; compaction = c.compaction
                    context = c.contextTokens.map(String.init) ?? ""; output = c.outputTokens.map(String.init) ?? ""
                }
            }
        }
    }
}

/// Add or edit a model route.
private struct RouteForm: View {
    let model: RegistryState
    let registry: ProviderRegistry
    let initial: ProviderRegistry.Route?
    @Environment(\.dismiss) private var dismiss
    @State private var alias = ""
    @State private var display = ""
    @State private var service = ""
    @State private var upstream = ""
    @State private var account = ""
    @State private var overrides = false
    @State private var tools = true
    @State private var images = true
    @State private var reasoning = true
    @State private var compaction = false
    @State private var context = ""
    @State private var output = ""
    @State private var message: String?
    @State private var busy = false

    private var chosen: ProviderRegistry.Service? { registry.services.first { $0.id == service } }
    private var eligible: [Account] { Registry.eligible(model.accounts, service: chosen, hosted: model.hosted) }
    private var input: RouteInput {
        let d = display.trimmingCharacters(in: .whitespaces)
        return RouteInput(alias: alias, service: service, upstreamModel: upstream, accountId: account, displayName: d.isEmpty ? nil : d,
                          capabilities: overrides ? .init(tools: tools, images: images, reasoning: reasoning, compaction: compaction && chosen?.protocol == "responses",
                                                          contextTokens: limit(context), outputTokens: limit(output)) : nil)
    }
    private var problem: String? {
        Registry.routeProblem(input, registry: registry, accounts: model.accounts, hosted: model.hosted, editing: initial != nil)
            ?? (overrides ? Registry.tokenLimitProblem(context) ?? Registry.tokenLimitProblem(output) : nil)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Client model name", text: $alias).textInputAutocapitalization(.never).autocorrectionDisabled().disabled(initial != nil)
                    TextField("Display name (optional)", text: $display)
                    Picker("Provider service", selection: $service) {
                        ForEach(registry.services) { Text("\($0.label) · \($0.id)").tag($0.id) }
                    }
                    // An account that cannot sign for the new service is let go; one that can stays,
                    // as an edited route's does when the form is filled in.
                    .onChange(of: service) { _, _ in
                        if !eligible.contains(where: { $0.id == account }) { account = "" }
                        if chosen?.protocol == "messages" { compaction = false }
                    }
                    TextField("Upstream model ID", text: $upstream).textInputAutocapitalization(.never).autocorrectionDisabled()
                    Picker("Provider account", selection: $account) {
                        Text("Choose an account").tag("")
                        ForEach(eligible) { Text("\($0.label) · \($0.id)").tag($0.id) }
                    }
                } footer: {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("The exact alias clients select. Reusing a built-in model name overrides its usual route.")
                        if eligible.isEmpty { Text(Registry.noEligible) }
                        if chosen?.builtin == false { Text(Registry.customAccounts) }
                    }
                }
                Section {
                    Toggle("Override configured capabilities", isOn: $overrides)
                    if overrides {
                        CapabilitiesEditor(tools: $tools, images: $images, reasoning: $reasoning, compaction: $compaction, context: $context, output: $output, responses: chosen?.protocol == "responses")
                    }
                } footer: {
                    Text(overrides ? "Model settings can further restrict a service. They cannot enable a feature disabled by its service, or increase a service’s token limits." : "Inherit the service’s configured capabilities and limits.")
                }
                if let p = message ?? (alias.isEmpty && upstream.isEmpty ? nil : problem) { Section { Text(p).font(.footnote).foregroundStyle(.red) } }
            }
            .navigationTitle(initial == nil ? "Add model route" : "Edit model route")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() }.disabled(busy) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        Task {
                            busy = true
                            defer { busy = false }
                            if let refused = await model.save(input) { message = refused } else { dismiss() }
                        }
                    }
                    .disabled(problem != nil || busy)
                }
            }
            .onAppear {
                if service.isEmpty { service = registry.services.first?.id ?? "" }
                guard let r = initial else { return }
                alias = r.alias; display = r.displayName ?? ""; service = r.service; upstream = r.upstreamModel; account = r.accountId ?? ""
                if let c = r.capabilities {
                    overrides = true
                    tools = c.tools; images = c.images; reasoning = c.reasoning; compaction = c.compaction
                    context = c.contextTokens.map(String.init) ?? ""; output = c.outputTokens.map(String.init) ?? ""
                }
            }
        }
    }
}
