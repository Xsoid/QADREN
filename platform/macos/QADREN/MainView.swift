import SwiftUI

struct MainView: View {
    @StateObject private var connections = StorageConnections()
    @StateObject private var localWorkspace = LocalWorkspace()
    @StateObject private var rightLocalWorkspace = LocalWorkspace()
    @State private var jobs: [JobRow] = []
    @State private var jobError: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                HStack(spacing: 12) {
                    Image(systemName: "externaldrive.badge.icloud")
                        .font(.system(size: 36))
                        .foregroundStyle(.tint)
                    VStack(alignment: .leading) {
                        Text("QADREN")
                            .font(.title.bold())
                        Text("Native macOS bootstrap")
                            .foregroundStyle(.secondary)
                    }
                }
                Divider()
                LabeledContent("Core version", value: CoreBridge.version)
                LabeledContent("Core status", value: CoreBridge.status)
                LabeledContent("Local worker", value: CoreBridge.workerStatus)
                Divider()
                Text("Local files").font(.headline)
                HStack(alignment: .top, spacing: 18) { LocalWorkspacePanel(title: "Left", workspace: localWorkspace); LocalWorkspacePanel(title: "Right", workspace: rightLocalWorkspace) }
                Text("Metadata only: QADREN does not open file contents. Copy, move and delete remain unavailable.").font(.caption).foregroundStyle(.secondary)
                Divider()
                HStack {
                    Text("Job queue").font(.headline)
                    Spacer()
                    Button("Queue mock file") { queueMockJob() }
                    Button("Refresh jobs") { refreshJobs() }
                }
                if let jobError {
                    Text(jobError).font(.caption).foregroundStyle(.orange)
                } else if jobs.isEmpty {
                    Text("No jobs yet. Queued mock Jobs change local SQLite only until you explicitly run them.")
                        .font(.caption).foregroundStyle(.secondary)
                } else {
                    ForEach(jobs) { job in
                        HStack {
                            Text(job.id).lineLimit(1)
                            Spacer()
                            Text(job.state).foregroundStyle(.secondary)
                            if job.state == "queued" {
                                Button("Run") { runMockJob(job.id) }
                                Button("Pause") { pauseMockJob(job.id) }
                                Button("Cancel", role: .destructive) { cancelMockJob(job.id) }
                            } else if job.state == "paused" {
                                Button("Resume") { resumeMockJob(job.id) }
                                Button("Cancel", role: .destructive) { cancelMockJob(job.id) }
                            }
                        }
                    }
                }
                Divider()
                HStack {
                    Text("Storage connections")
                        .font(.headline)
                    Spacer()
                    Button("Add Yandex Disk") { connections.addYandex() }
                    Button("Reload") { connections.refresh() }
                }
                Text("Each connection has separate credentials and its own identity.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if connections.hasLegacyYandexToken {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Existing Yandex Disk authorization found")
                        Button("Import existing connection") {
                            connections.importLegacyYandex()
                        }
                        Text("Your old token stays in Keychain until the copy has been verified.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                if let error = connections.error {
                    Text(error).font(.caption).foregroundStyle(.orange)
                }
                if connections.entries.isEmpty && !connections.hasLegacyYandexToken {
                    Text("No storage connections. Use Add Yandex Disk to begin.")
                        .foregroundStyle(.secondary)
                }
                ForEach(connections.entries) { entry in
                    if entry.adapter == "yandex_disk" {
                        YandexConnectionCard(
                            entry: entry,
                            onRemove: { connections.remove(entry.id) },
                            onRename: { name in connections.rename(entry.id, name: name) },
                            onSetEnabled: { enabled in connections.setEnabled(entry.id, enabled: enabled) }
                        )
                        .id(entry.id + (entry.enabled ? ":enabled" : ":disabled"))
                        Divider()
                    } else {
                        LabeledContent(entry.name, value: entry.adapter)
                        Divider()
                    }
                }
                Text("iCloud Documents is not yet implemented. Cloud file content transfer is disabled.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Text("QADREN does not download or upload file contents in this prototype.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .padding(24)
        }
        .onAppear { refreshJobs() }
    }

    private func refreshJobs() {
        do {
            jobs = try JobInventory.list()
            jobError = nil
        } catch {
            jobError = error.localizedDescription
        }
    }

    private func queueMockJob() {
        do {
            try JobInventory.queueMockArtifact()
            refreshJobs()
        } catch {
            jobError = error.localizedDescription
        }
    }

    private func runMockJob(_ jobID: String) {
        do {
            try JobInventory.runMockArtifact(jobID: jobID)
            refreshJobs()
        } catch {
            jobError = error.localizedDescription
            refreshJobs()
        }
    }

    private func cancelMockJob(_ jobID: String) {
        do {
            try JobInventory.cancelMockArtifact(jobID: jobID)
            refreshJobs()
        } catch {
            jobError = error.localizedDescription
            refreshJobs()
        }
    }

    private func pauseMockJob(_ jobID: String) {
        do {
            try JobInventory.pauseMockArtifact(jobID: jobID)
            refreshJobs()
        } catch {
            jobError = error.localizedDescription
            refreshJobs()
        }
    }

    private func resumeMockJob(_ jobID: String) {
        do {
            try JobInventory.resumeMockArtifact(jobID: jobID)
            refreshJobs()
        } catch {
            jobError = error.localizedDescription
            refreshJobs()
        }
    }
}

private struct LocalWorkspacePanel: View {
    let title: String
    @ObservedObject var workspace: LocalWorkspace
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack { Text(title).font(.subheadline.bold()); Spacer(); Button("Choose") { workspace.chooseRoot() }; Button("Reload") { workspace.refresh() }.disabled(workspace.root == nil) }
            if let root = workspace.root {
                Text(root).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                HStack { Button("Up") { workspace.up() }.disabled(workspace.relativePath.isEmpty); Text(workspace.relativePath.isEmpty ? "/" : "/" + workspace.relativePath).font(.caption); Spacer(); Button("New folder") { workspace.mkdir("New Folder") }.disabled(workspace.entries.contains(where: { $0.name == "New Folder" })) }
                ForEach(workspace.entries) { entry in Button { workspace.open(entry) } label: { HStack { Image(systemName: entry.is_directory ? "folder" : "doc"); Text(entry.name); Spacer(); if !entry.is_directory { Text(ByteCountFormatter.string(fromByteCount: Int64(clamping: entry.size), countStyle: .file)) } }.foregroundStyle(entry.is_directory ? .primary : .secondary) }.buttonStyle(.plain).disabled(!entry.is_directory) }
            } else { Text("Choose a folder.").font(.caption).foregroundStyle(.secondary) }
            if let error = workspace.error { Text(error).font(.caption).foregroundStyle(.orange) }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct YandexConnectionCard: View {
    let entry: StorageEntry
    let onRemove: () -> Void
    let onRename: (String) -> Void
    let onSetEnabled: (Bool) -> Void
    @StateObject private var yandex: YandexConnection
    @State private var confirmRemoval = false
    @State private var draftName: String

    init(
        entry: StorageEntry,
        onRemove: @escaping () -> Void,
        onRename: @escaping (String) -> Void,
        onSetEnabled: @escaping (Bool) -> Void
    ) {
        self.entry = entry
        self.onRemove = onRemove
        self.onRename = onRename
        self.onSetEnabled = onSetEnabled
        _draftName = State(initialValue: entry.name)
        _yandex = StateObject(wrappedValue: YandexConnection(connectionID: entry.id, enabled: entry.enabled))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text(entry.name).font(.headline)
                    Text("Yandex Disk · connection " + String(entry.id.prefix(8)))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button("Remove") { confirmRemoval = true }
                    .disabled(yandex.busy || yandex.metadataBusy)
            }
            HStack {
                TextField("Connection name", text: $draftName)
                    .textFieldStyle(.roundedBorder)
                Button("Rename") { onRename(draftName) }
                    .disabled(draftName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || draftName == entry.name)
                Toggle("Enabled", isOn: Binding(
                    get: { entry.enabled },
                    set: { onSetEnabled($0) }
                ))
            }
            if !entry.enabled {
                Text("Disabled locally — no new operations will be started for this connection.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Text(yandex.clientID.isEmpty ? "Client ID not configured for this build" : "OAuth application configured")
                .font(.caption)
                .foregroundStyle(.secondary)
            HStack {
                Button("Connect") { yandex.connect() }
                    .disabled(!entry.enabled || yandex.busy || yandex.connected || yandex.clientID.isEmpty)
                Button("Disconnect") { yandex.disconnect() }
                    .disabled(yandex.busy || !yandex.connected)
            }
            if let account = yandex.accountLabel {
                LabeledContent("Yandex account", value: account)
            }
            Text(yandex.status).font(.caption).foregroundStyle(.secondary)

            if yandex.connected {
                HStack {
                    Text("Disk metadata").font(.subheadline.bold())
                    Button("Refresh") { Task { await yandex.refreshDisk() } }
                        .disabled(!entry.enabled || yandex.metadataBusy)
                }
                if yandex.metadataBusy { ProgressView() }
                if let quota = yandex.quota {
                    LabeledContent("Total storage", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: quota.total_space), countStyle: .file))
                    LabeledContent("Used", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: quota.used_space), countStyle: .file))
                    LabeledContent("Free", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: quota.total_space > quota.used_space ? quota.total_space - quota.used_space : 0), countStyle: .file))
                }
                if let error = yandex.metadataError {
                    Text("Disk API: " + error).font(.caption).foregroundStyle(.orange)
                }
                Text("Root directory metadata (first 50 of \(yandex.rootTotal))")
                    .font(.subheadline)
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 6) {
                        ForEach(yandex.rootItems) { item in
                            HStack {
                                Image(systemName: item.kind == "dir" ? "folder" : "doc")
                                Text(item.name).lineLimit(1)
                                Spacer()
                                if let size = item.size {
                                    Text(ByteCountFormatter.string(fromByteCount: Int64(clamping: size), countStyle: .file))
                                        .foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                }
                .frame(maxHeight: 160)
            }
        }
        .confirmationDialog(
            "Remove this QADREN connection locally?",
            isPresented: $confirmRemoval
        ) {
            Button("Remove connection", role: .destructive, action: onRemove)
        } message: {
            Text("This removes only this local connection and its Keychain token. Files in Yandex Disk are untouched.")
        }
    }
}
