import SwiftUI

/// The dashboard's settings modal, minus accounts (which have their own screen): how
/// Claude Code on the Mac reaches the router, the wrapper, the password, the router.
struct RouterSettingsView: View {
    let state: RouterState
    @State private var model: SettingsState?
    @State private var project = ""
    @State private var confirmRemovePassword = false

    var body: some View {
        List {
            if let f = model?.failure { Section { Label(f, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange) } }
            if let m = model?.message { Section { Text(m).font(.footnote) } }
            if let model {
                setupSection(model)
                wrapperSection(model)
                NotificationsSection(model: model, state: state)
                PasswordSection(model: model, confirmRemove: $confirmRemovePassword)
                routerSection(model)
            } else {
                Section { HStack { ProgressView(); Text("Reading…").foregroundStyle(.secondary) } }
            }
        }
        .navigationTitle("Settings")
        .refreshable { await model?.load() }
        .task {
            let m = SettingsState(router: state)
            model = m
            await m.load()
        }
    }

    @ViewBuilder private func setupSection(_ model: SettingsState) -> some View {
        if let s = model.setup {
            Section {
                LabeledContent {
                    Button(s.service.registered ? "Remove" : "Install") {
                        Task { await model.service(enable: !s.service.registered) }
                    }
                    .buttonStyle(.bordered)
                    .disabled(!s.service.supported || model.busy != nil)
                } label: {
                    VStack(alignment: .leading) {
                        Text("Start at login")
                        Text(s.service.supported ? (s.service.registered ? "installed as a login item" : "not installed") : "not supported on \(s.service.platform)")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
                LabeledContent {
                    Button(s.globalRouted ? "Stop routing" : "Route everything") {
                        Task { await model.config(enable: !s.globalRouted, project: nil) }
                    }
                    .buttonStyle(.bordered)
                    .disabled(model.busy != nil)
                } label: {
                    VStack(alignment: .leading) {
                        Text("Route every project")
                        Text(s.globalSettingsPath).font(.caption2).foregroundStyle(.secondary).lineLimit(1).truncationMode(.head)
                    }
                }
                ForEach(s.managed.filter { $0.scope != "global" }) { m in
                    LabeledContent {
                        Button("Stop") { Task { await model.config(enable: false, project: dirOf(m.path)) } }
                            .buttonStyle(.bordered)
                            .disabled(model.busy != nil)
                    } label: {
                        VStack(alignment: .leading) {
                            Text(dirOf(m.path).split(separator: "/").last.map(String.init) ?? m.path)
                            Text(m.path).font(.caption2).foregroundStyle(.secondary).lineLimit(1).truncationMode(.head)
                        }
                    }
                }
                HStack {
                    TextField("Route a single project", text: $project, prompt: Text("/path/to/project"))
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                    Button("Route") {
                        let p = project.trimmingCharacters(in: .whitespaces)
                        guard !p.isEmpty else { return }
                        Task { await model.config(enable: true, project: p); project = "" }
                    }
                    .buttonStyle(.bordered)
                    .disabled(project.trimmingCharacters(in: .whitespaces).isEmpty || model.busy != nil)
                }
            } header: {
                Text("Claude Code on the Mac")
            } footer: {
                Text("Only `env.ANTHROPIC_BASE_URL` is ever written to a settings file. Everything else is left as it is, and turning routing off puts back whatever was there before — unless you changed it yourself since, in which case it is left alone.")
            }
        }
    }

    private func dirOf(_ settingsPath: String) -> String {
        // <project>/.claude/settings.local.json → <project>
        var parts = settingsPath.split(separator: "/").map(String.init)
        if parts.last == "settings.local.json" { parts.removeLast() }
        if parts.last == ".claude" { parts.removeLast() }
        return "/" + parts.joined(separator: "/")
    }

    @ViewBuilder private func wrapperSection(_ model: SettingsState) -> some View {
        if let sh = model.shim {
            Section {
                Toggle(isOn: Binding(get: { sh.auto ?? false }, set: { on in Task { await model.shimAuto(on) } })) {
                    VStack(alignment: .leading) {
                        Text("Manage automatically")
                        Text(sh.autoLog?.first.map { "\($0.what) · \(Fmt.ago(Fmt.date(ms: $0.at)!))" }
                             ?? "Wraps every extension version VSCode installs, within seconds.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
                .disabled(!sh.available && !(sh.auto ?? false) || model.busy != nil)
                ForEach(sh.targets, id: \.path) { t in
                    LabeledContent {
                        Button(t.state == "installed" ? "Remove" : t.state == "inconsistent" ? "Reinstall" : "Install") {
                            Task { await model.shim(path: t.path, action: t.state == "installed" ? "remove" : "install") }
                        }
                        .buttonStyle(.bordered)
                        .disabled(!sh.available && t.state != "installed" || model.busy != nil)
                    } label: {
                        VStack(alignment: .leading) {
                            Text(t.ext.replacingOccurrences(of: "anthropic.claude-code-", with: "").replacingOccurrences(of: "-darwin-arm64", with: ""))
                            Text(t.state.replacingOccurrences(of: "_", with: " "))
                                .font(.caption)
                                .foregroundStyle(t.state == "installed" ? .green : t.state == "inconsistent" ? .orange : .secondary)
                        }
                    }
                }
                if !sh.available {
                    Label("The wrapper binary is not beside the router on the Mac, so it cannot be installed from here.", systemImage: "exclamationmark.triangle")
                        .font(.footnote).foregroundStyle(.orange)
                }
            } header: {
                Text("Wrapper — typed messages and stops")
            } footer: {
                Text("Each Claude Code extension version on the Mac spawns its own binary; the wrapper takes that binary's place. Sessions must be restarted to pick it up, and an extension update ships unwrapped — install it again here.")
            }
        }
    }

    @ViewBuilder private func routerSection(_ model: SettingsState) -> some View {
        Section {
            LabeledContent("Address", value: state.router.address)
            if let h = state.health { LabeledContent("Version", value: "\(h.version) · pid \(h.pid)") }
            if let s = model.setup {
                LabeledContent("Dashboard", value: s.url)
                if let url = state.router.baseURL {
                    Link(destination: url) { Label("Open the dashboard in Safari", systemImage: "safari") }
                }
            }
        } header: {
            Text("Router")
        } footer: {
            Text("Everything above is stored on the Mac. The router's own settings (port, database, upstream, body cap) are environment variables read at startup.")
        }
    }
}

/// Set, change or remove the router password from the phone — allowed because this
/// phone already presented it.
struct PasswordSection: View {
    let model: SettingsState
    @Binding var confirmRemove: Bool
    @State private var a = ""
    @State private var b = ""
    @State private var hint: String?

    var body: some View {
        Section {
            LabeledContent("Status", value: model.auth?.configured == true
                           ? "set" + (Fmt.date(ms: model.auth?.updatedAt).map { " · changed \(Fmt.ago($0))" } ?? "")
                           : model.auth?.hasCredentials == true ? "not set — access tokens are configured" : "not set — other devices are refused")
            if let auth = model.auth, auth.accessTokens > 0 {
                LabeledContent("Active access tokens", value: "\(auth.accessTokens)")
            }
            SecureField("New password", text: $a, prompt: Text("at least 8 characters"))
            SecureField("Confirm", text: $b, prompt: Text("again"))
            HStack {
                Button(model.auth?.configured == true ? "Change password" : "Set password") {
                    guard a.count >= 8 else { hint = "use at least 8 characters"; return }
                    guard a == b else { hint = "the two entries differ"; return }
                    Task { await model.password(a, clear: false); a = ""; b = ""; hint = nil }
                }
                .buttonStyle(.borderedProminent)
                .disabled(model.busy != nil)
                if model.auth?.configured == true {
                    Button("Remove", role: .destructive) { confirmRemove = true }
                        .buttonStyle(.bordered)
                }
                if let hint { Text(hint).font(.caption).foregroundStyle(.secondary) }
            }
        } header: {
            Text("Password")
        } footer: {
            Text("Changing the password affects devices using that password. Access tokens remain valid. If this phone uses the password, enter its replacement under Edit. Manage tokens in the web dashboard’s Settings → Access tokens.")
        }
        .confirmationDialog("Remove the password?", isPresented: $confirmRemove, titleVisibility: .visible) {
            Button("Remove", role: .destructive) { Task { await model.password(nil, clear: true) } }
        } message: {
            Text("Devices using this password lose access. Access tokens remain valid. If this phone uses the password, replace its credential under Edit.")
        }
    }
}

/// Notifications: this phone's permission and token, and the router's APNs key.
struct NotificationsSection: View {
    let model: SettingsState
    let state: RouterState
    @State private var teamId = ""
    @State private var keyId = ""
    @State private var bundleId = ""
    @State private var p8 = ""
    @State private var confirmRemove = false

    private var registrar: PushRegistrar { PushRegistrar.shared }

    var body: some View {
        Section {
            LabeledContent {
                Button(registrar.authorization == .denied ? "Open Settings" : "Enable") {
                    if registrar.authorization == .denied, let url = URL(string: UIApplication.openSettingsURLString) {
                        UIApplication.shared.open(url)
                    } else {
                        Task { await registrar.enable() }
                    }
                }
                .buttonStyle(.bordered)
                .disabled(registrar.authorization == .authorized && registrar.token != nil && registrar.registrationError == nil)
            } label: {
                VStack(alignment: .leading) {
                    Text("This phone")
                    Text(phoneStatus).font(.caption).foregroundStyle(registrar.registrationError == nil ? Color.secondary : Color.orange)
                }
            }
            if registrar.token != nil {
                LabeledContent {
                    Button("Register again") { Task { _ = await registrar.register(with: state); await model.load() } }
                        .buttonStyle(.bordered)
                } label: {
                    VStack(alignment: .leading) {
                        Text("With this router")
                        Text(registrar.routers[state.router.id] ?? "not registered yet").font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        } header: {
            Text("Notifications")
        } footer: {
            Text("A permission or a question in any session under the wrapper is announced here, once; tapping it opens that session.")
        }
        .task {
            await registrar.refreshAuthorization()
            if let p = model.push { teamId = p.teamId ?? ""; keyId = p.keyId ?? ""; bundleId = p.bundleId ?? "" }
        }

        Section {
            if let p = model.push {
                LabeledContent("Key on the router", value: p.configured ? "\(p.keyId ?? "set") · \(p.devices.count) phone\(p.devices.count == 1 ? "" : "s")" : "not set")
                if let f = p.keyFile { Text("Key file: \(f)").font(.caption).foregroundStyle(.secondary) }
                if let e = p.lastError { Text("last failure: \(e)").font(.caption).foregroundStyle(.red) }
                if let s = p.lastSent, let at = Fmt.date(ms: s.at) { Text("last sent \(Fmt.ago(at)): \(s.what)").font(.caption).foregroundStyle(.secondary) }
                ForEach(p.devices) { d in
                    LabeledContent("\(d.name ?? "phone") · …\(d.tokenTail)") {
                        Text(d.lastResult ?? d.environment).font(.caption)
                    }
                }
            }
            TextField("Team id", text: $teamId, prompt: Text("ABCDE12345")).textInputAutocapitalization(.characters).autocorrectionDisabled()
            TextField("Key id", text: $keyId, prompt: Text("ABC123DEF4")).textInputAutocapitalization(.characters).autocorrectionDisabled()
            TextField("Bundle id", text: $bundleId, prompt: Text("ca.whnet.agent-router")).textInputAutocapitalization(.never).autocorrectionDisabled()
            VStack(alignment: .leading, spacing: 4) {
                Text("APNs key (.p8)").font(.caption).foregroundStyle(.secondary)
                TextEditor(text: $p8)
                    .font(.system(.caption, design: .monospaced))
                    .frame(minHeight: 88)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
            }
            HStack {
                Button(model.push?.configured == true ? "Replace key" : "Save key") {
                    Task { await model.pushConfig(teamId: teamId, keyId: keyId, p8: p8, bundleId: bundleId.isEmpty ? nil : bundleId); p8 = "" }
                }
                .buttonStyle(.borderedProminent)
                .disabled(teamId.isEmpty || keyId.isEmpty || p8.isEmpty || model.busy != nil)
                if model.push?.configured == true {
                    Button("Send test") { Task { await model.pushTest() } }.buttonStyle(.bordered).disabled(model.busy != nil)
                    Button("Remove", role: .destructive) { confirmRemove = true }.buttonStyle(.bordered)
                }
            }
        } header: {
            Text("Apple push key")
        } footer: {
            Text("One key, made once at developer.apple.com → Certificates, IDs & Profiles → Keys → + → Apple Push Notifications service. Drop the downloaded AuthKey_….p8 into ~/.agent-router on the Mac and the router takes it in by itself — or paste it here. Either way it stays in the router's secrets file and never comes back out.")
        }
        .confirmationDialog("Remove the push key?", isPresented: $confirmRemove, titleVisibility: .visible) {
            Button("Remove", role: .destructive) { Task { await model.pushClear() } }
        } message: {
            Text("Phones stay registered, but nothing can be sent until a key is set again.")
        }
    }

    private var phoneStatus: String {
        if let e = registrar.registrationError {
            return e.contains("aps-environment") ? "this build has no push entitlement — sign into Xcode and rebuild" : e
        }
        switch registrar.authorization {
        case .denied: return "off in iOS Settings"
        case .authorized, .provisional, .ephemeral: return registrar.token == nil ? "allowed, waiting for Apple's token" : "on"
        default: return "not asked yet"
        }
    }
}
