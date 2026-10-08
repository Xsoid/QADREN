import SwiftUI

struct MainView: View {
    @StateObject private var yandex = YandexConnection()
    var body: some View {
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
            Text("Yandex Disk (read-only authorization)").font(.headline)
            Text(yandex.clientID.isEmpty ? "Client ID not configured for this build" : "OAuth application configured")
                .font(.caption)
                .foregroundStyle(.secondary)
            HStack {
                Button("Connect") { yandex.connect() }
                    .disabled(yandex.busy || yandex.connected || yandex.clientID.isEmpty)
                Button("Disconnect") { yandex.disconnect() }
                    .disabled(yandex.busy || !yandex.connected)
            }
            if let account = yandex.accountLabel {
                LabeledContent("Yandex account", value: account)
            }
            Text(yandex.status).font(.caption).foregroundStyle(.secondary)
            if yandex.connected {
                Divider()
                HStack {
                    Text("Disk metadata").font(.headline)
                    Button("Refresh") { Task { await yandex.refreshDisk() } }
                        .disabled(yandex.metadataBusy)
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
                Text("Root directory metadata (first 50 of \(yandex.rootTotal))").font(.subheadline)
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
                .frame(maxHeight: 190)
            }
            Spacer()
            Text("No file contents are downloaded or uploaded by QADREN.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(24)
    }
}
