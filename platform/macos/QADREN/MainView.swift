import SwiftUI

struct MainView: View {
    @StateObject private var connections = StorageConnections()

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
                Text("QADREN does not download or upload file contents in this prototype.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .padding(24)
        }
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
