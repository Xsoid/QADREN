import SwiftUI

struct MainView: View {
    @StateObject private var connections = StorageConnections()
    @StateObject private var localWorkspace = LocalWorkspace()
    @StateObject private var rightLocalWorkspace = LocalWorkspace()
    @State private var jobs: [JobRow] = []
    @State private var jobError: String?
    @State private var section: DispatcherSection? = .overview

    var body: some View {
        NavigationSplitView {
            List(DispatcherSection.allCases, selection: $section) { item in
                Label(item.title, systemImage: item.symbol).tag(item)
            }
            .navigationSplitViewColumnWidth(min: 190, ideal: 215)
            .safeAreaInset(edge: .bottom) {
                HStack(spacing: 8) {
                    Circle().fill(CoreBridge.workerStatus == "ready" ? .green : .orange).frame(width: 8, height: 8)
                    Text("Local Worker")
                    Spacer()
                }
                .font(.caption).foregroundStyle(.secondary).padding(12)
            }
        } detail: {
            Group {
                switch section ?? .overview {
                case .overview: overview
                case .jobs: jobsWorkspace
                case .files: filesWorkspace
                case .connections: connectionsWorkspace
                }
            }
            .navigationTitle((section ?? .overview).title)
            .toolbar { Button("Refresh", systemImage: "arrow.clockwise") { refreshCurrentSection() } }
        }
        .onAppear { refreshJobs() }
    }

    private var overview: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Dispatcher").font(.largeTitle.bold())
                    Text("Local Controller and Worker · current operational state").foregroundStyle(.secondary)
                }
                LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 3), spacing: 14) {
                    StatusTile(title: "Worker", value: CoreBridge.workerStatus, detail: "This Mac", symbol: "cpu")
                    StatusTile(title: "Queue", value: "\(jobs.count)", detail: jobs.isEmpty ? "No recorded Jobs" : "Recorded Jobs", symbol: "list.number")
                    StatusTile(title: "Connections", value: "\(connections.entries.count)", detail: "Independent local identities", symbol: "externaldrive")
                }
                WorkspaceCard(title: "Queue", subtitle: "Durable Job inventory") {
                    if jobs.isEmpty {
                        EmptyWorkspaceState(title: "No Jobs", detail: "When execution is available, its queue and operational state appear here.", symbol: "checkmark.circle")
                    } else {
                        JobList(jobs: jobs, onRun: runMockJob, onPause: pauseMockJob, onResume: resumeMockJob, onCancel: cancelMockJob)
                    }
                }
                WorkspaceCard(title: "Health", subtitle: "Local runtime") {
                    LabeledContent("Core", value: CoreBridge.status)
                    LabeledContent("iCloud Documents", value: ICloudAvailability.status)
                    Text("File content is not downloaded, uploaded, or previewed by this build.").font(.caption).foregroundStyle(.secondary)
                }
            }.padding(28)
        }
    }

    private var jobsWorkspace: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                WorkspaceHeader(title: "Job queue", subtitle: "Durable attempts and their current local state") {
                    Button("Queue test Job") { queueMockJob() }
                }
                if let jobError { Notice(text: jobError, symbol: "exclamationmark.triangle") }
                WorkspaceCard(title: "Queue", subtitle: "Operations shown here do not create a Scenario") {
                    if jobs.isEmpty {
                        EmptyWorkspaceState(title: "Queue is empty", detail: "The test command creates only an app-owned local artifact after an explicit Run.", symbol: "tray")
                    } else {
                        JobList(jobs: jobs, onRun: runMockJob, onPause: pauseMockJob, onResume: resumeMockJob, onCancel: cancelMockJob)
                    }
                }
            }.padding(28)
        }
    }

    private var filesWorkspace: some View {
        VStack(spacing: 0) {
            WorkspaceHeader(title: "Files", subtitle: "Two local metadata panels") { EmptyView() }
                .padding(.horizontal, 28).padding(.vertical, 18)
            HStack(spacing: 0) {
                FileManagerPane(title: "Left panel", workspace: localWorkspace)
                Divider()
                FileManagerPane(title: "Right panel", workspace: rightLocalWorkspace)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            Notice(text: "Metadata only. Content preview is unavailable; copy, move, rename and delete remain disabled until their safe Job paths are implemented.", symbol: "shield")
                .padding(16)
        }
    }

    private var connectionsWorkspace: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                WorkspaceHeader(title: "Storage connections", subtitle: "Every connection has an independent identity and credentials") {
                    Button("Add Yandex Disk", systemImage: "plus") { connections.addYandex() }
                }
                if connections.hasLegacyYandexToken {
                    Notice(text: "An existing Yandex Disk authorization can be copied into an independent connection. The old Keychain item remains until the copy is verified.", symbol: "key") {
                        Button("Import existing connection") { connections.importLegacyYandex() }
                    }
                }
                if let error = connections.error { Notice(text: error, symbol: "exclamationmark.triangle") }
                if connections.entries.isEmpty && !connections.hasLegacyYandexToken {
                    EmptyWorkspaceState(title: "No connections", detail: "Add a storage connection to begin its local setup.", symbol: "externaldrive.badge.plus")
                        .frame(maxWidth: .infinity, minHeight: 240)
                }
                ForEach(connections.entries) { entry in
                    WorkspaceCard(title: entry.name, subtitle: entry.adapter) {
                        if entry.adapter == "yandex_disk" {
                            YandexConnectionCard(
                                entry: entry,
                                onRemove: { connections.remove(entry.id) },
                                onRename: { connections.rename(entry.id, name: $0) },
                                onSetEnabled: { connections.setEnabled(entry.id, enabled: $0) }
                            )
                            .id(entry.id + (entry.enabled ? ":enabled" : ":disabled"))
                        } else {
                            LabeledContent("Adapter", value: entry.adapter)
                        }
                    }
                }
                Text(ICloudAvailability.status).font(.caption).foregroundStyle(.secondary)
            }.padding(28)
        }
    }

    private func refreshCurrentSection() {
        switch section ?? .overview {
        case .overview, .jobs:
            refreshJobs()
            connections.refresh()
        case .files:
            localWorkspace.refresh()
            rightLocalWorkspace.refresh()
        case .connections:
            connections.refresh()
        }
    }

    private func refreshJobs() {
        do { jobs = try JobInventory.list(); jobError = nil }
        catch { jobError = error.localizedDescription }
    }
    private func queueMockJob() {
        do { try JobInventory.queueMockArtifact(); refreshJobs() }
        catch { jobError = error.localizedDescription }
    }
    private func runMockJob(_ jobID: String) {
        do { try JobInventory.runMockArtifact(jobID: jobID); refreshJobs() }
        catch { jobError = error.localizedDescription; refreshJobs() }
    }
    private func cancelMockJob(_ jobID: String) {
        do { try JobInventory.cancelMockArtifact(jobID: jobID); refreshJobs() }
        catch { jobError = error.localizedDescription; refreshJobs() }
    }
    private func pauseMockJob(_ jobID: String) {
        do { try JobInventory.pauseMockArtifact(jobID: jobID); refreshJobs() }
        catch { jobError = error.localizedDescription; refreshJobs() }
    }
    private func resumeMockJob(_ jobID: String) {
        do { try JobInventory.resumeMockArtifact(jobID: jobID); refreshJobs() }
        catch { jobError = error.localizedDescription; refreshJobs() }
    }
}

private enum DispatcherSection: String, CaseIterable, Hashable, Identifiable {
    case overview, jobs, files, connections
    var id: Self { self }
    var title: String {
        switch self { case .overview: "Overview"; case .jobs: "Jobs"; case .files: "Files"; case .connections: "Connections" }
    }
    var symbol: String {
        switch self { case .overview: "rectangle.3.group"; case .jobs: "checklist"; case .files: "folder"; case .connections: "externaldrive" }
    }
}

private struct StatusTile: View {
    let title: String
    let value: String
    let detail: String
    let symbol: String
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Image(systemName: symbol).foregroundStyle(.tint)
            Text(value).font(.title2.bold())
            Text(title).font(.headline)
            Text(detail).font(.caption).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, minHeight: 108, alignment: .leading)
        .padding(16)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

private struct WorkspaceCard<Content: View>: View {
    let title: String
    let subtitle: String
    @ViewBuilder let content: Content
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.headline)
                Text(subtitle).font(.caption).foregroundStyle(.secondary)
            }
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading).padding(18)
        .background(.background, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(.quaternary))
    }
}

private struct WorkspaceHeader<Actions: View>: View {
    let title: String
    let subtitle: String
    @ViewBuilder let actions: Actions
    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 16) {
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.largeTitle.bold())
                Text(subtitle).foregroundStyle(.secondary)
            }
            Spacer()
            actions
        }
    }
}

private struct Notice<Actions: View>: View {
    let text: String
    let symbol: String
    @ViewBuilder let actions: Actions
    init(text: String, symbol: String, @ViewBuilder actions: () -> Actions = { EmptyView() }) {
        self.text = text; self.symbol = symbol; self.actions = actions()
    }
    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: symbol).foregroundStyle(.orange)
            Text(text).font(.callout)
            Spacer(minLength: 0)
            actions
        }
        .padding(14)
        .background(.orange.opacity(0.10), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

private struct EmptyWorkspaceState: View {
    let title: String
    let detail: String
    let symbol: String
    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: symbol).font(.system(size: 28)).foregroundStyle(.secondary)
            Text(title).font(.headline)
            Text(detail).font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, minHeight: 150)
        .padding()
    }
}

private struct JobList: View {
    let jobs: [JobRow]
    let onRun: (String) -> Void
    let onPause: (String) -> Void
    let onResume: (String) -> Void
    let onCancel: (String) -> Void
    var body: some View {
        VStack(spacing: 0) {
            ForEach(jobs) { job in
                HStack(spacing: 12) {
                    Image(systemName: job.state == "paused" ? "pause.circle" : "circle.dashed").foregroundStyle(job.state == "paused" ? Color.orange : Color.accentColor)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(job.id).textSelection(.enabled)
                        Text(job.scenario_id == nil ? "Manual Job" : "Scenario Job").font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Text(job.state.capitalized).font(.callout.weight(.medium))
                    if job.state == "queued" {
                        Button("Run") { onRun(job.id) }
                        Button("Pause") { onPause(job.id) }
                        Button("Cancel", role: .destructive) { onCancel(job.id) }
                    } else if job.state == "paused" {
                        Button("Resume") { onResume(job.id) }
                        Button("Cancel", role: .destructive) { onCancel(job.id) }
                    }
                }.padding(.vertical, 10)
                if job.id != jobs.last?.id { Divider() }
            }
        }
    }
}

private struct FileManagerPane: View {
    let title: String
    @ObservedObject var workspace: LocalWorkspace
    @State private var selected = Set<String>()
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(title).font(.headline)
                Spacer()
                Button("Choose folder") { workspace.chooseRoot() }
                Button("Reload", systemImage: "arrow.clockwise") { workspace.refresh() }.disabled(workspace.root == nil)
            }.padding(.horizontal, 18).padding(.top, 16)
            if let root = workspace.root {
                VStack(alignment: .leading, spacing: 8) {
                    Text(root).font(.caption).textSelection(.enabled)
                    HStack {
                        Button("Up", systemImage: "chevron.up") { workspace.up() }.disabled(workspace.relativePath.isEmpty)
                        Text(workspace.relativePath.isEmpty ? "/" : "/" + workspace.relativePath).font(.caption.monospaced()).textSelection(.enabled)
                        Spacer()
                        if !selected.isEmpty { Text("\(selected.count) selected").font(.caption).foregroundStyle(.secondary) }
                        Button("New folder", systemImage: "folder.badge.plus") { workspace.mkdir("New Folder") }
                            .disabled(workspace.entries.contains(where: { $0.name == "New Folder" }))
                    }
                }.padding(.horizontal, 18)
                List(workspace.entries) { entry in
                    HStack {
                        Toggle("", isOn: Binding(get: { selected.contains(entry.name) }, set: { enabled in if enabled { selected.insert(entry.name) } else { selected.remove(entry.name) } })).labelsHidden()
                        Button { workspace.open(entry) } label: {
                            HStack {
                                Image(systemName: entry.is_directory ? "folder.fill" : "doc").foregroundStyle(entry.is_directory ? Color.accentColor : Color.secondary)
                                Text(entry.name)
                                Spacer()
                                if !entry.is_directory { Text(ByteCountFormatter.string(fromByteCount: Int64(clamping: entry.size), countStyle: .file)).foregroundStyle(.secondary) }
                            }
                        }.buttonStyle(.plain).disabled(!entry.is_directory)
                    }
                }.listStyle(.inset)
            } else {
                EmptyWorkspaceState(title: "Choose a folder", detail: "This panel reads only names, types and sizes.", symbol: "folder")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            if let error = workspace.error { Notice(text: error, symbol: "exclamationmark.triangle").padding(12) }
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
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

    init(entry: StorageEntry, onRemove: @escaping () -> Void, onRename: @escaping (String) -> Void, onSetEnabled: @escaping (Bool) -> Void) {
        self.entry = entry; self.onRemove = onRemove; self.onRename = onRename; self.onSetEnabled = onSetEnabled
        _draftName = State(initialValue: entry.name)
        _yandex = StateObject(wrappedValue: YandexConnection(connectionID: entry.id, enabled: entry.enabled))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Yandex Disk · connection " + String(entry.id.prefix(8))).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Remove", role: .destructive) { confirmRemoval = true }.disabled(yandex.busy || yandex.metadataBusy)
            }
            HStack {
                TextField("Connection name", text: $draftName).textFieldStyle(.roundedBorder)
                Button("Rename") { onRename(draftName) }.disabled(draftName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || draftName == entry.name)
                Toggle("Enabled", isOn: Binding(get: { entry.enabled }, set: { onSetEnabled($0) }))
            }
            if !entry.enabled { Text("Disabled locally — no new operations will be started for this connection.").font(.caption).foregroundStyle(.secondary) }
            Text(yandex.clientID.isEmpty ? "Client ID not configured for this build" : "OAuth application configured").font(.caption).foregroundStyle(.secondary)
            HStack {
                Button("Connect") { yandex.connect() }.disabled(!entry.enabled || yandex.busy || yandex.connected || yandex.clientID.isEmpty)
                Button("Disconnect") { yandex.disconnect() }.disabled(yandex.busy || !yandex.connected)
            }
            if let account = yandex.accountLabel { LabeledContent("Yandex account", value: account) }
            Text(yandex.status).font(.caption).foregroundStyle(.secondary)
            if yandex.connected {
                HStack { Text("Disk metadata").font(.subheadline.bold()); Button("Refresh") { Task { await yandex.refreshDisk() } }.disabled(!entry.enabled || yandex.metadataBusy) }
                if yandex.metadataBusy { ProgressView() }
                if let quota = yandex.quota {
                    LabeledContent("Total storage", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: quota.total_space), countStyle: .file))
                    LabeledContent("Used", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: quota.used_space), countStyle: .file))
                    LabeledContent("Free", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: quota.total_space > quota.used_space ? quota.total_space - quota.used_space : 0), countStyle: .file))
                }
                if let error = yandex.metadataError { Text("Disk API: " + error).font(.caption).foregroundStyle(.orange) }
                Text("Root directory metadata (first 50 of \(yandex.rootTotal))").font(.subheadline)
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 6) {
                        ForEach(yandex.rootItems) { item in
                            HStack {
                                Image(systemName: item.kind == "dir" ? "folder" : "doc")
                                Text(item.name).lineLimit(1)
                                Spacer()
                                if let size = item.size { Text(ByteCountFormatter.string(fromByteCount: Int64(clamping: size), countStyle: .file)).foregroundStyle(.secondary) }
                            }
                        }
                    }
                }.frame(maxHeight: 160)
            }
        }
        .confirmationDialog("Remove this QADREN connection locally?", isPresented: $confirmRemoval) {
            Button("Remove connection", role: .destructive, action: onRemove)
        } message: {
            Text("This removes only this local connection and its Keychain token. Files in Yandex Disk are untouched.")
        }
    }
}
