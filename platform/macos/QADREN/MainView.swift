import SwiftUI

struct MainView: View {
    @Environment(\.openWindow) private var openWindow
    @Environment(\.colorScheme) private var colorScheme
    @StateObject private var connections = StorageConnections()
    @StateObject private var localWorkspace = LocalWorkspace()
    @StateObject private var rightLocalWorkspace = LocalWorkspace()
    @StateObject private var dashboardLayout = DashboardLayoutStore()
    @SceneStorage("qadren.monitoring.panel-id") private var monitoringPanelID = ""
    @State private var isEditingLayout = false
    @State private var jobs: [JobRow] = []
    @State private var jobError: String?
    let workspace: DispatcherSection

    init(workspace: DispatcherSection = .monitoring) {
        self.workspace = workspace
    }

    var body: some View {
        Group {
            switch workspace {
            case .monitoring: monitoringWorkspace
            case .scenarios: scenarioSettingsWorkspace
            case .files: filesWorkspace
            }
        }
        .tint(QDTheme.cyan)
        .navigationTitle(workspace.title)
        .background(TitlebarLogoAccessory().frame(width: 0, height: 0))
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                Menu {
                    Button("Контрольная панель") { openWindow(id: "monitoring") }
                    Button("Файловый менеджер") { openWindow(id: "file-manager") }
                    Button("Настройки") { openWindow(id: "scenario-settings") }
                } label: {
                    Label("Пространство", systemImage: "rectangle.3.group")
                }
                if workspace == .monitoring || workspace == .files {
                    Button("Новая панель", systemImage: "macwindow.on.rectangle") { openWindow(id: "monitoring") }
                    if workspace == .monitoring {
                        Button(isEditingLayout ? "Готово" : "Компоновка", systemImage: "rectangle.3.group.bubble") {
                            isEditingLayout.toggle()
                        }
                        Button("Отменить компоновку", systemImage: "arrow.uturn.backward") {
                            dashboardLayout.undo()
                        }
                        .disabled(!isEditingLayout || !dashboardLayout.canUndo)
                        .keyboardShortcut("z", modifiers: .command)
                        .help("Отменить последнее изменение компоновки")
                        Button("Вернуть компоновку", systemImage: "arrow.uturn.forward") {
                            dashboardLayout.redo()
                        }
                        .disabled(!isEditingLayout || !dashboardLayout.canRedo)
                        .keyboardShortcut("z", modifiers: [.command, .shift])
                        .help("Вернуть отменённое изменение компоновки")
                        Menu("Добавить виджет", systemImage: "plus.rectangle.on.rectangle") {
                            ForEach(dashboardLayout.hiddenWidgets) { widget in
                                Button(widget.kind.title) {
                                    dashboardLayout.setVisible(widget.kind, isVisible: true)
                                }
                            }
                            if dashboardLayout.hiddenWidgets.isEmpty {
                                Text("Все стандартные виджеты уже на панели")
                            }
                            Divider()
                            Button("Сбросить компоновку") { dashboardLayout.reset() }
                        }
                        .disabled(!isEditingLayout)
                    }
                    Button("Обновить", systemImage: "arrow.clockwise") { refreshCurrentWorkspace() }
                }
            }
        }
        .onAppear {
            if workspace == .monitoring {
                let panelID = monitoringPanelID.isEmpty ? UUID().uuidString : monitoringPanelID
                monitoringPanelID = panelID
                dashboardLayout.load(panelID: panelID)
            }
            refreshJobs()
        }
    }

    private var monitoringWorkspace: some View {
        monitoringDashboard
    }

    private var monitoringDashboard: some View {
        ScrollView {
            VStack(spacing: 10) {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 270), spacing: 10)], spacing: 10) {
                    ForEach(dashboardLayout.visibleWidgets) { widget in
                        monitoringWidget(widget)
                    }
                }
                .padding(12)

                HStack(spacing: 8) {
                    QDStatusLabel(kind: workerStatusKind, text: "Worker: \(workerStatus)")
                    Divider().frame(height: 14)
                    Text("Обновлено по запросу").font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Text("Внимание: не поддерживается Core").font(.caption).foregroundStyle(.secondary)
                }
                .padding(.horizontal, 12).padding(.bottom, 8)
            }
        }
        .background(QDTheme.color(.canvas, for: colorScheme))
    }

    @ViewBuilder
    private func monitoringWidget(_ widget: MonitoringWidgetLayout) -> some View {
        switch widget.kind {
        case .system:
            QDWidgetFrame(title: "Система", symbol: "cpu", status: workerStatusKind, actions: {
                layoutActions(for: widget)
            }) {
                VStack(alignment: .leading, spacing: 10) {
                    QDStatusLabel(kind: workerStatusKind, text: workerStatus)
                    LabeledContent("Ядро", value: coreStatus)
                    if widget.density != .compact {
                        Text("Источник: локальный Core; содержимое файлов не читается.")
                            .font(.caption2).foregroundStyle(.secondary)
                    }
                    if widget.density == .expanded {
                        LabeledContent("Режим", value: "только метаданные")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
        case .jobs:
            QDWidgetFrame(title: "Задания", symbol: "arrow.triangle.2.circlepath", status: jobWidgetStatus, actions: {
                layoutActions(for: widget)
            }) {
                if widget.density == .compact {
                    QDMetric(value: "\(visibleJobs.count)", label: "в инвентаре", provenance: "локальный JobStore")
                } else if visibleJobs.isEmpty {
                    QDEmptyState(title: "Заданий нет", detail: "История появится после безопасного выполнения через общий Job Engine.", symbol: "tray")
                } else {
                    JobList(jobs: visibleJobs, onRun: runMockJob, onPause: pauseMockJob, onResume: resumeMockJob, onCancel: cancelMockJob)
                }
            }
        case .queue:
            QDWidgetFrame(title: "Очередь", symbol: "list.number", actions: {
                layoutActions(for: widget)
            }) {
                HStack(alignment: .top, spacing: 24) {
                    QDMetric(value: "\(visibleJobs.count)", label: "в инвентаре", provenance: "локальный JobStore")
                    if widget.density != .compact {
                        QDMetric(value: nil, label: "ETA", provenance: "не поддерживается")
                    }
                }
                if widget.density != .compact {
                    Text("Порядок, прогресс и оценка времени появятся только когда их подтвердит Core.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                if widget.density == .expanded {
                    Text("Источник: локальный JobStore; автоматическое исполнение не заявлено.")
                        .font(.caption2).foregroundStyle(.secondary)
                }
            }
        case .connections:
            QDWidgetFrame(title: "Подключения", symbol: "externaldrive", actions: {
                layoutActions(for: widget)
            }) {
                if widget.density == .compact {
                    QDMetric(value: "\(connections.entries.count)", label: "подключений", provenance: "локальный Registry")
                } else if connections.entries.isEmpty {
                    VStack(alignment: .leading, spacing: 10) {
                        Text("Подключения ещё не настроены.").foregroundStyle(.secondary)
                        Button("Открыть настройки") { openWindow(id: "scenario-settings") }
                    }
                } else {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(widget.density == .expanded ? connections.entries : Array(connections.entries.prefix(4))) { entry in
                            HStack(spacing: 8) {
                                Image(systemName: entry.enabled ? "externaldrive.fill" : "externaldrive")
                                    .foregroundStyle(entry.enabled ? QDTheme.cyan : .secondary)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(entry.name).lineLimit(1)
                                    Text("\(adapterTitle(entry.adapter)) · \(entry.status)")
                                        .font(.caption).foregroundStyle(.secondary)
                                }
                                Spacer()
                            }
                        }
                        Button("Настроить подключения") { openWindow(id: "scenario-settings") }
                            .font(.caption)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func layoutActions(for widget: MonitoringWidgetLayout) -> some View {
        if isEditingLayout {
            Button("Переместить раньше", systemImage: "arrow.left") {
                dashboardLayout.move(widget.kind, by: -1)
            }
            .disabled(dashboardLayout.widgets.first?.kind == widget.kind)
            .labelStyle(.iconOnly)
            .help("Переместить раньше")
            Button("Переместить позже", systemImage: "arrow.right") {
                dashboardLayout.move(widget.kind, by: 1)
            }
            .disabled(dashboardLayout.widgets.last?.kind == widget.kind)
            .labelStyle(.iconOnly)
            .help("Переместить позже")
            Button("Скрыть виджет", systemImage: "eye.slash") {
                dashboardLayout.setVisible(widget.kind, isVisible: false)
            }
            .labelStyle(.iconOnly)
            .help("Скрыть виджет")
            Menu("Плотность виджета", systemImage: "rectangle.3.group") {
                ForEach(MonitoringWidgetDensity.allCases) { density in
                    Button {
                        dashboardLayout.setDensity(widget.kind, density: density)
                    } label: {
                        if widget.density == density {
                            Label(density.title, systemImage: "checkmark")
                        } else {
                            Text(density.title)
                        }
                    }
                }
            }
            .labelStyle(.iconOnly)
            .help("Плотность виджета")
        }
    }

    private var jobsWorkspace: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                WorkspaceHeader(title: "Очередь заданий", subtitle: "Долговременные попытки и их текущее локальное состояние") { EmptyView() }
                if let jobError { Notice(text: jobError, symbol: "exclamationmark.triangle") }
                WorkspaceCard(title: "Очередь", subtitle: "Эти действия не создают постоянный сценарий") {
                    if visibleJobs.isEmpty {
                        EmptyWorkspaceState(title: "Очередь пуста", detail: "Задания появятся после реализации реальных сценариев или безопасных ручных файловых операций.", symbol: "tray")
                    } else {
                        JobList(jobs: visibleJobs, onRun: runMockJob, onPause: pauseMockJob, onResume: resumeMockJob, onCancel: cancelMockJob)
                    }
                }
            }.padding(28)
        }
    }

    private var filesWorkspace: some View {
        VStack(spacing: 0) {
            WorkspaceHeader(title: "Файловый менеджер", subtitle: "Две локальные панели метаданных") { EmptyView() }
                .padding(.horizontal, 28).padding(.vertical, 18)
            HStack(spacing: 0) {
                FileManagerPane(title: "Левая панель", workspace: localWorkspace)
                Divider()
                FileManagerPane(title: "Правая панель", workspace: rightLocalWorkspace)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            Notice(text: "Доступны только метаданные. Предпросмотр содержимого, создание папок, копирование, перемещение, переименование и удаление выключены до реализации их безопасных Job-путей.", symbol: "shield")
                .padding(16)
        }
    }

    private var scenarioSettingsWorkspace: some View {
        SettingsPanelView(connections: connections)
    }

    private func refreshCurrentWorkspace() {
        switch workspace {
        case .monitoring:
            refreshJobs()
            connections.refresh()
        case .scenarios:
            connections.refresh()
        case .files:
            localWorkspace.refresh()
            rightLocalWorkspace.refresh()
        }
    }

    private var workerStatus: String {
        switch CoreBridge.workerStatus {
        case "Idle", "Idle Worker": "Worker простаивает"
        default: CoreBridge.workerStatus
        }
    }
    private var workerStatusKind: QDStatusKind {
        switch CoreBridge.workerStatus {
        case "Idle", "Idle Worker": .ready
        case "Busy": .active
        case "Unavailable": .unsupported
        default: .unknown
        }
    }
    private var jobWidgetStatus: QDStatusKind {
        if visibleJobs.contains(where: { $0.state == "unknown" }) { return .unknown }
        if visibleJobs.contains(where: { $0.state == "running" }) { return .active }
        if visibleJobs.contains(where: { $0.state == "paused" }) { return .paused }
        if visibleJobs.contains(where: { $0.state == "queued" }) { return .waiting }
        return visibleJobs.isEmpty ? .waiting : .ready
    }
    private var coreStatus: String {
        switch CoreBridge.status {
        case "ready - worker idle", "Core ready - worker idle": "Готово — Worker простаивает"
        default: CoreBridge.status
        }
    }
    private func adapterTitle(_ adapter: String) -> String { adapter == "yandex_disk" ? "Яндекс Диск" : adapter }

    private func refreshJobs() {
        do { jobs = try JobInventory.list(); jobError = nil }
        catch { jobError = error.localizedDescription }
    }
    /// `g1-*` is a test-only durable fixture created by the early mock runner.
    /// It is retained for safe diagnostics, but is never product queue data.
    private var visibleJobs: [JobRow] {
        jobs.filter { !$0.id.hasPrefix("g1-") }
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

enum DispatcherSection: String, CaseIterable, Hashable, Identifiable {
    case monitoring, scenarios, files
    var id: Self { self }
    var title: String {
        switch self { case .monitoring: "Контрольная панель"; case .scenarios: "Настройки"; case .files: "Файловый менеджер" }
    }
    var symbol: String {
        switch self { case .monitoring: "rectangle.3.group"; case .scenarios: "slider.horizontal.3"; case .files: "rectangle.split.2x1" }
    }
}

private enum MonitoringView {
    case dashboard, jobs, files
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
    @Environment(\.colorScheme) private var colorScheme
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.system(size: 13, weight: .semibold))
                Text(subtitle).font(.caption).foregroundStyle(.secondary)
            }
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading).padding(12)
        .background(QDTheme.color(.panel, for: colorScheme), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).stroke(QDTheme.color(.stroke, for: colorScheme)))
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
                        Text(job.scenario_id == nil ? "Разовое задание" : "Задание сценария").font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Text(jobStateTitle(job.state)).font(.callout.weight(.medium))
                    if job.id.hasPrefix("g1-") && job.state == "queued" {
                        Button("Запустить") { onRun(job.id) }
                        Button("Приостановить") { onPause(job.id) }
                        Button("Отменить", role: .destructive) { onCancel(job.id) }
                    } else if job.id.hasPrefix("g1-") && job.state == "paused" {
                        Button("Возобновить") { onResume(job.id) }
                        Button("Отменить", role: .destructive) { onCancel(job.id) }
                    }
                }.padding(.vertical, 10)
                if job.id != jobs.last?.id { Divider() }
            }
        }
    }
}

private func jobStateTitle(_ state: String) -> String {
    switch state {
    case "queued": "В очереди"
    case "paused": "Приостановлено"
    case "completed": "Завершено"
    case "cancelled": "Отменено"
    case "running": "Выполняется"
    default: state
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
                Button("Выбрать папку") { workspace.chooseRoot() }
                Button("Обновить", systemImage: "arrow.clockwise") { workspace.refresh() }.disabled(workspace.root == nil)
            }.padding(.horizontal, 18).padding(.top, 16)
            if let root = workspace.root {
                VStack(alignment: .leading, spacing: 8) {
                    Text(root).font(.caption).textSelection(.enabled)
                    HStack {
                        Button("Вверх", systemImage: "chevron.up") { workspace.up() }.disabled(workspace.relativePath.isEmpty)
                        Text(workspace.relativePath.isEmpty ? "/" : "/" + workspace.relativePath).font(.caption.monospaced()).textSelection(.enabled)
                        Spacer()
                        if !selected.isEmpty { Text("Выбрано: \(selected.count)").font(.caption).foregroundStyle(.secondary) }
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
                EmptyWorkspaceState(title: "Выберите папку", detail: "Эта панель читает только имена, типы и размеры.", symbol: "folder")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            if let error = workspace.error { Notice(text: error, symbol: "exclamationmark.triangle").padding(12) }
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
    }
}

struct YandexConnectionCard: View {
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
                    Text("Яндекс Диск · подключение " + String(entry.id.prefix(8))).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Удалить", role: .destructive) { confirmRemoval = true }.disabled(yandex.busy || yandex.metadataBusy)
            }
            HStack {
                TextField("Название подключения", text: $draftName).textFieldStyle(.roundedBorder)
                Button("Переименовать") { onRename(draftName) }.disabled(draftName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || draftName == entry.name)
                Toggle("Включено", isOn: Binding(get: { entry.enabled }, set: { onSetEnabled($0) }))
            }
            if !entry.enabled { Text("Подключение выключено локально — новые операции для него не запускаются.").font(.caption).foregroundStyle(.secondary) }
            Text(yandex.clientID.isEmpty ? "Client ID не настроен в этой сборке" : "OAuth-приложение настроено").font(.caption).foregroundStyle(.secondary)
            HStack {
                Button("Подключить") { yandex.connect() }.disabled(!entry.enabled || yandex.busy || yandex.connected || yandex.clientID.isEmpty)
                Button("Отключить") { yandex.disconnect() }.disabled(yandex.busy || !yandex.connected)
            }
            if let account = yandex.accountLabel { LabeledContent("Аккаунт Яндекс", value: account) }
            Text(connectionStatus).font(.caption).foregroundStyle(.secondary)
            if yandex.connected {
                HStack { Text("Метаданные Диска").font(.subheadline.bold()); Button("Обновить") { Task { await yandex.refreshDisk() } }.disabled(!entry.enabled || yandex.metadataBusy) }
                if yandex.metadataBusy { ProgressView() }
                if let quota = yandex.quota {
                    LabeledContent("Всего", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: quota.total_space), countStyle: .file))
                    LabeledContent("Занято", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: quota.used_space), countStyle: .file))
                    LabeledContent("Свободно", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: quota.total_space > quota.used_space ? quota.total_space - quota.used_space : 0), countStyle: .file))
                }
                if let error = yandex.metadataError { Text("API Диска: " + error).font(.caption).foregroundStyle(.orange) }
                Text("Метаданные корневого каталога (первые 50 из \(yandex.rootTotal))").font(.subheadline)
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
        .confirmationDialog("Удалить это локальное подключение QADREN?", isPresented: $confirmRemoval) {
            Button("Удалить подключение", role: .destructive, action: onRemove)
        } message: {
            Text("Будут удалены только локальное подключение и его токен Keychain. Файлы на Яндекс Диске не изменятся.")
        }
    }

    private var connectionStatus: String {
        switch yandex.status {
        case "Registered connection": "Подключение зарегистрировано"
        case "Connected": "Подключено"
        case "Disconnected": "Отключено"
        default: yandex.status
        }
    }
}
