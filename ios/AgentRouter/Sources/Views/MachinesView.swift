import SwiftUI

/// A hosted router's machines, as the web console's Machines screen shows them: how to
/// connect one first, whatever the list holds, and under it every machine the router has
/// seen since it last started — online first, each with its platform, how long it has been
/// online or offline, and whether it runs the router's own build (MA-C2 to MA-C6).
struct MachinesView: View {
    let state: RouterState
    @State private var model: MachinesState?
    /// The command just copied, so its button can say so for a moment.
    @State private var copied: String?

    var body: some View {
        let items = model?.items(now: .now) ?? []
        List {
            connectSection
            Section {
                if let rows = model?.rows {
                    // A failed read keeps the machines already read, and says why (MA-C6).
                    if let why = model?.failure {
                        Label {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(why)
                                Text("The machines below are the last ones the router sent.")
                                    .foregroundStyle(.secondary)
                            }
                        } icon: {
                            Image(systemName: "wifi.exclamationmark")
                        }
                        .font(.footnote)
                        .foregroundStyle(.orange)
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("machines-stale")
                    }
                    if rows.isEmpty {
                        // Which empty it is: nothing has connected since the router started (MA-C5).
                        Text(Machines.agentless)
                            .foregroundStyle(.secondary)
                    }
                    ForEach(items) { MachineRowView(item: $0) }
                } else if let why = model?.failure {
                    // A read that never succeeded is not an empty list (MA-C6).
                    Label {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Could not read the machines")
                            Text(why).foregroundStyle(.secondary)
                        }
                    } icon: {
                        Image(systemName: "exclamationmark.triangle")
                    }
                    .font(.footnote)
                    .foregroundStyle(.orange)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("machines-error")
                } else {
                    Text("Reading the machines…")
                        .foregroundStyle(.secondary)
                }
            } header: {
                HStack {
                    Text(Machines.seen)
                    Spacer()
                    Text(Machines.tally(items))
                }
            }
        }
        .navigationTitle("Machines")
        .refreshable { await model?.load() }
        .task {
            // The model outlives a pass through the background, so what was read stays.
            let m = model ?? MachinesState(router: state)
            model = m
            await m.poll()
        }
    }

    /// The installer's command for each platform, at the router's public address, each with
    /// its own copy button and a way to send it to the computer it is for (MA-C2).
    @ViewBuilder private var connectSection: some View {
        let origin = Machines.origin(publicURL: state.auth?.publicURL, router: state.router)
        Section {
            Text(Machines.connectHow)
                .font(.subheadline)
            ForEach(Machines.installCommands(origin: origin)) { c in
                VStack(alignment: .leading, spacing: 6) {
                    Text(c.label)
                        .font(.subheadline.weight(.semibold))
                    Text(c.command)
                        .font(.system(.footnote, design: .monospaced))
                        .textSelection(.enabled)
                        .accessibilityLabel("The command to run on \(c.label)")
                        .accessibilityValue(c.command)
                    HStack(spacing: 20) {
                        Button(copied == c.id ? "Copied" : "Copy",
                               systemImage: copied == c.id ? "checkmark" : "doc.on.doc") {
                            UIPasteboard.general.string = c.command
                            copied = c.id
                            Task {
                                try? await Task.sleep(for: .seconds(2))
                                if copied == c.id { copied = nil }
                            }
                        }
                        .accessibilityLabel(copied == c.id ? "Copied" : "Copy the \(c.label) command")
                        ShareLink(item: c.command) {
                            Label("Share", systemImage: "square.and.arrow.up")
                        }
                        .accessibilityLabel("Share the \(c.label) command")
                    }
                    // Two buttons in one row: each must take only its own taps.
                    .buttonStyle(.borderless)
                    .font(.footnote)
                }
                .padding(.vertical, 2)
                .accessibilityIdentifier("install-command")
            }
        } header: {
            Text(Machines.connect)
        } footer: {
            Text(Machines.withVSCode)
        }
    }
}

/// One machine at a glance: its dot and its label, its platform, online or offline and for
/// how long, and whether it runs the router's own build — quietly when it does (MA-C3, MA-C4).
private struct MachineRowView: View {
    let item: Machines.Item

    var body: some View {
        HStack(spacing: 10) {
            // Its words are beside it, so the dot is not said twice.
            StateDot(look: item.look)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(item.label)
                        .lineLimit(1)
                    if !item.platform.isEmpty {
                        Text(item.platform)
                            .foregroundStyle(.secondary)
                    }
                }
                Text(item.presence)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if let b = item.build {
                    Text(b.text)
                        .font(.caption)
                        .foregroundStyle(b.same ? Color.secondary : Color.orange)
                }
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("machine-row")
    }
}
