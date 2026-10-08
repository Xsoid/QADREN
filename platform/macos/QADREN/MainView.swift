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
            TextField("Yandex OAuth Client ID", text: $yandex.clientID)
                .textFieldStyle(.roundedBorder)
                .disabled(yandex.busy)
            HStack {
                Button("Connect") { yandex.connect() }
                    .disabled(yandex.busy || yandex.connected)
                Button("Disconnect") { yandex.disconnect() }
                    .disabled(yandex.busy || !yandex.connected)
            }
            Text(yandex.status).font(.caption).foregroundStyle(.secondary)
            Spacer()
            Text("No file contents are downloaded or uploaded by QADREN.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(24)
    }
}
