import SwiftUI

/// The Library of a hosted router: every skill, command and subagent by kind, how much each is
/// used, and a sync of every machine.
struct LibraryView: View {
    let state: RouterState
    @State private var model: LibraryState?

    var body: some View {
        List {
            if state.auth?.isHosted == false {
                Section { Text(Library.notHere).foregroundStyle(.secondary) }
            } else {
                if let f = model?.failure { Section { Label(f, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange) } }
                Section {
                    Text(Library.listingWords(model?.usage?.listing)).font(.footnote).foregroundStyle(.secondary)
                    Button {
                        Task { await model?.syncAll() }
                    } label: {
                        if model?.busy == true { ProgressView() } else { Label("Sync every machine now", systemImage: "arrow.triangle.2.circlepath") }
                    }
                    .disabled(model?.busy != false)
                    .accessibilityIdentifier("library-sync-all")
                    if let d = model?.done { Text(d).font(.caption).foregroundStyle(.secondary) }
                }
                if let items = model?.items {
                    if items.isEmpty { Section { Text(Library.empty).foregroundStyle(.secondary) } }
                    ForEach(Library.kinds, id: \.self) { kind in
                        let ofKind = items.filter { $0.kind == kind }
                        if !ofKind.isEmpty {
                            Section(Library.kindWord(kind) + "s") {
                                ForEach(ofKind) { item in
                                    NavigationLink(value: LibraryItemRoute(routerId: state.router.id, kind: item.kind, name: item.name)) {
                                        LibraryRow(item: item)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle("Library")
        .overlay { if model?.items == nil, model?.failure == nil, state.auth?.isHosted != false { ProgressView() } }
        .refreshable { await model?.load() }
        .task {
            let m = model ?? LibraryState(router: state)
            model = m
            while !Task.isCancelled {
                await m.load()
                do { try await Task.sleep(for: .seconds(5)) } catch { return }
            }
        }
    }
}

private struct LibraryRow: View {
    let item: LibraryItem

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack {
                Text(item.name).font(.body.monospaced())
                if item.archivedAt != nil { Text("archived").font(.caption2).foregroundStyle(.secondary) }
            }
            if !item.description.isEmpty {
                Text(item.description).font(.caption).foregroundStyle(.secondary).lineLimit(2)
            }
            Text([Library.liveWords(item),
                  item.pending > 0 ? "\(item.pending) waiting for approval" : nil,
                  Library.sourceWords(item.source),
                  item.uses30d.map(Library.usedWords)].compactMap { $0 }.joined(separator: " · "))
                .font(.caption2)
                .foregroundStyle(item.pending > 0 ? Color.orange : Color.secondary)
        }
        .padding(.vertical, 2)
    }
}

/// One item: its description and source, its use, its versions with what can be decided on
/// each, its files, and its main file as Claude Code reads it.
struct LibraryItemView: View {
    let state: RouterState
    let kind: String
    let name: String
    @State private var model: LibraryState?
    @State private var detail: LibraryItemDetail?
    @State private var main: String?
    @State private var failure: String?
    @State private var notice: String?
    @State private var archiving = false
    @State private var working = false

    var body: some View {
        List {
            if let failure { Section { Label(failure, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange) } }
            if let notice { Section { Text(notice).font(.footnote).foregroundStyle(.red) } }
            if let d = detail {
                Section {
                    if !d.description.isEmpty { Text(d.description) }
                    LabeledContent("Kind", value: Library.kindWord(d.kind))
                    LabeledContent("Source", value: Library.sourceWords(d.source))
                    if d.archivedAt != nil { Text("Archived: no machine places it any more.").foregroundStyle(.secondary) }
                }
                usageSection
                Section("Versions") {
                    ForEach(d.versions) { v in versionRow(v, kind: d.kind, name: d.name) }
                }
                if !d.files.isEmpty {
                    Section("Files") {
                        ForEach(d.files) { f in
                            LabeledContent(f.path + (f.executable ? " (executable)" : ""), value: ByteCountFormatter.string(fromByteCount: Int64(f.size), countStyle: .file))
                                .font(.caption.monospaced())
                        }
                    }
                }
                if let main {
                    Section(Library.mainFile(kind: d.kind, name: d.name)) {
                        RichText(text: Library.body(main))
                    }
                }
                if d.archivedAt == nil {
                    Section {
                        Button("Archive \(Library.kindWord(d.kind).lowercased())", systemImage: "archivebox", role: .destructive) { archiving = true }
                            .disabled(working)
                    }
                }
            }
        }
        .navigationTitle(name)
        .navigationBarTitleDisplayMode(.inline)
        .overlay { if detail == nil, failure == nil { ProgressView() } }
        .refreshable { await load() }
        .confirmationDialog("Archive \(name)?", isPresented: $archiving, titleVisibility: .visible) {
            Button("Archive", role: .destructive) {
                Task {
                    working = true
                    defer { working = false }
                    notice = await model?.archive(kind: kind, name: name)
                    await load()
                }
            }
        } message: {
            Text("Every machine stops placing it at its next sync. Its versions stay in the Library.")
        }
        .task {
            if model == nil { model = LibraryState(router: state) }
            await load()
        }
    }

    @ViewBuilder private var usageSection: some View {
        if let u = model?.usage?.items.first(where: { $0.kind == kind && $0.name == name }) {
            Section("Use") {
                LabeledContent("Last 30 days", value: Library.usedWords(u.uses30d))
                if let at = Fmt.date(ms: u.lastUsedAt) { LabeledContent("Last used", value: Fmt.ago(at)) }
                ForEach(Array(u.machines.enumerated()), id: \.offset) { _, m in
                    LabeledContent(Library.machineWords(m), value: Fmt.count(Double(m.uses)))
                }
                ForEach(Array(u.projects.enumerated()), id: \.offset) { _, p in
                    LabeledContent(ProjectPath.name(p.project), value: Fmt.count(Double(p.uses)))
                }
            }
        }
    }

    @ViewBuilder private func versionRow(_ v: LibraryVersion, kind: String, name: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("v\(v.number)").font(.body.monospaced())
                Text(Library.stateWord(v.state)).font(.caption)
                    .foregroundStyle(v.state == "live" ? Color.green : v.state == "pending" ? Color.orange : Color.secondary)
                Spacer()
                if let at = Fmt.date(ms: v.createdAt) { Text(Fmt.ago(at)).font(.caption2).foregroundStyle(.secondary) }
            }
            Text(Library.provenanceWords(v.provenance)).font(.caption).foregroundStyle(.secondary)
            if let session = Library.sessionOf(v.provenance) {
                NavigationLink(value: SessionByIdRoute(routerId: state.router.id, sessionId: session)) {
                    Label("Open the session that published it", systemImage: "bubble.left.and.bubble.right").font(.caption)
                }
            }
            HStack(spacing: 16) {
                ForEach(Library.acts(v.state), id: \.self) { act in
                    Button(Library.actWord(act)) {
                        Task {
                            working = true
                            defer { working = false }
                            notice = await model?.act(act, kind: kind, name: name, version: v.number)
                            await load()
                        }
                    }
                    .buttonStyle(.borderless)
                    .font(.caption.weight(.medium))
                    .disabled(working)
                }
            }
        }
        .padding(.vertical, 2)
    }

    private func load() async {
        guard let model else { return }
        async let list: Void = model.load()
        switch await model.detail(kind: kind, name: name) {
        case .success(let d):
            detail = d
            failure = nil
            if let newest = d.versions.map(\.number).max() {
                main = await model.file(kind: kind, name: name, version: newest, path: Library.mainFile(kind: kind, name: name))
            }
        case .failure(let e):
            failure = e.errorDescription
        }
        await list
    }
}
