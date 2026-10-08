import SwiftUI

struct MainView: View {
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
            Spacer()
            Text("No clouds connected. No files are downloaded or uploaded.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(24)
    }
}
