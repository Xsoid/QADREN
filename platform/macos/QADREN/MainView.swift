import SwiftUI

struct MainView: View {
    @Environment(\.openWindow) private var openWindow
    @Environment(\.colorScheme) private var colorScheme
    @StateObject private var connections = StorageConnections()
    @StateObject private var localWorkspace = LocalWorkspace()
    @StateObject private var rightLocalWorkspace = LocalWorkspace()
    @State private var jobs: [JobRow] = []
    @State private var jobError: String?
    @AppStorage("qadren.theme") private var themeRaw = QDThemeMode.dark.rawValue
    let workspace: DispatcherSection

    init(workspace: DispatcherSection = .monitoring) {
        self.workspace = workspace
    }

    var body: some View {
        Group {
            switch workspace {
            case .monitoring: monitoringWorkspace
            case .scenarios: scenarioSettingsWorkspace
            }
        }
        .tint(QDTheme.cyan)
        .navigationTitle(workspace.title)
        .toolbar {
            ToolbarItem(placement: .navigation) {
                QDLogo(size: 20)
            }
            ToolbarItemGroup(placement: .primaryAction) {
                Menu {
                    Button("Контрольная панель") { openWindow(id: "monitoring") }
                    Button("Панель настроек") { openWindow(id: "scenario-settings") }
                } label: {
                    Label("Пространство", systemImage: "rectangle.3.group")
                }
                if workspace == .monitoring {
                    Button("Новая панель", systemImage: "macwindow.on.rectangle") { openWindow(id: "monitoring") }
                }
                if workspace == .scenarios {
                    Button("Добавить подключение", systemImage: "plus") { connections.addYandex() }
                }
                Button("Обновить", systemImage: "arrow.clockwise") { refreshCurrentWorkspace() }
            }
        }
        .onAppear { refreshJobs() }
    }

    private var monitoringWorkspace: some View {
        monitoringDashboard
    }

    private var monitoringDashboard: some View {
        ScrollView {
            VStack(spacing: 10) {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 270), spacing: 10)], spacing: 10) {
                    QDWidgetFrame(title: "Система", symbol: "cpu", status: workerStatusKind) {
                        VStack(alignment: .leading, spacing: 10) {
                            QDStatusLabel(kind: workerStatusKind, text: workerStatus)
                            LabeledContent("Ядро", value: coreStatus)
                            LabeledContent("iCloud Documents", value: ICloudAvailability.status)
                            Text("Источник: локальный Core; содержимое файлов не читается.")
                                .font(.caption2).foregroundStyle(.secondary)
                        }
                    }

                    QDWidgetFrame(title: "Задания", symbol: "arrow.triangle.2.circlepath", status: jobWidgetStatus) {
                        if visibleJobs.isEmpty {
                            QDEmptyState(
                                title: "Заданий нет",
                                detail: "История появится после безопасного выполнения через общий Job Engine.",
                                symbol: "tray"
                            )
                        } else {
                            JobList(jobs: visibleJobs, onRun: runMockJob, onPause: pauseMockJob, onResume: resumeMockJob, onCancel: cancelMockJob)
                        }
                    }

                    QDWidgetFrame(title: "Очередь", symbol: "list.number") {
                        HStack(alignment: .top, spacing: 24) {
                            QDMetric(value: "\(visibleJobs.count)", label: "в инвентаре", provenance: "локальный JobStore")
                            QDMetric(value: nil, label: "ETA", provenance: "не поддерживается")
                        }
                        Text("Порядок, прогресс и оценка времени появятся только когда их подтвердит Core.")
                            .font(.caption).foregroundStyle(.secondary)
                    }

                    QDWidgetFrame(title: "Подключения", symbol: "externaldrive") {
                        if connections.entries.isEmpty {
                            VStack(alignment: .leading, spacing: 10) {
                                Text("Подключения ещё не настроены.").foregroundStyle(.secondary)
                                Button("Открыть настройки") { openWindow(id: "scenario-settings") }
                            }
                        } else {
                            VStack(alignment: .leading, spacing: 8) {
                                ForEach(connections.entries.prefix(4)) { entry in
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
            Notice(text: "Доступны только метаданные. Предпросмотр содержимого, копирование, перемещение, переименование и удаление выключены до реализации их безопасных Job-путей.", symbol: "shield")
                .padding(16)
        }
    }

    private var scenarioSettingsWorkspace: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                WorkspaceCard(title: "Сценарии синхронизации", subtitle: "Правила будущего выполнения") {
                    EmptyWorkspaceState(title: "Создание сценариев ещё не реализовано", detail: "Нельзя создать фиктивный сценарий: сначала должны появиться проверяемый план, политики конфликтов и безопасное выполнение в ядре.", symbol: "arrow.triangle.2.circlepath")
                }
                WorkspaceCard(title: "Интерфейс", subtitle: "Локальные настройки отображения") {
                    Picker("Тема", selection: $themeRaw) {
                        ForEach(QDThemeMode.allCases) { mode in
                            Text(mode.title).tag(mode.rawValue)
                        }
                    }
                    .pickerStyle(.segmented)
                    Text("Настройка хранится локально и не меняет сценарии, подключения или состояние Worker.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                if connections.hasLegacyYandexToken {
                    Notice(text: "Найдена прежняя авторизация Яндекс Диска. Её можно перенести в независимое подключение; старый элемент Keychain останется до проверки копии.", symbol: "key") {
                        Button("Перенести подключение") { connections.importLegacyYandex() }
                    }
                }
                if let error = connections.error { Notice(text: error, symbol: "exclamationmark.triangle") }
                if connections.entries.isEmpty && !connections.hasLegacyYandexToken {
                    EmptyWorkspaceState(title: "Подключений нет", detail: "Добавьте подключение, чтобы начать его локальную настройку.", symbol: "externaldrive.badge.plus")
                        .frame(maxWidth: .infinity, minHeight: 240)
                }
                ForEach(connections.entries) { entry in
                    WorkspaceCard(title: entry.name, subtitle: adapterTitle(entry.adapter)) {
                        if entry.adapter == "yandex_disk" {
                            YandexConnectionCard(
                                entry: entry,
                                onRemove: { connections.remove(entry.id) },
                                onRename: { connections.rename(entry.id, name: $0) },
                                onSetEnabled: { connections.setEnabled(entry.id, enabled: $0) }
                            )
                            .id(entry.id + (entry.enabled ? ":enabled" : ":disabled"))
                        } else {
                            LabeledContent("Адаптер", value: entry.adapter)
                        }
                    }
                }
                Text(ICloudAvailability.status).font(.caption).foregroundStyle(.secondary)
            }
            .frame(maxWidth: 1100, alignment: .leading)
            .padding(16)
        }
        .background(QDTheme.color(.canvas, for: colorScheme))
    }

    private func refreshCurrentWorkspace() {
        switch workspace {
        case .monitoring:
            refreshJobs()
            connections.refresh()
        case .scenarios:
            connections.refresh()
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
    case monitoring, scenarios
    var id: Self { self }
    var title: String {
        switch self { case .monitoring: "Контрольная панель"; case .scenarios: "Панель настроек" }
    }
    var symbol: String {
        switch self { case .monitoring: "rectangle.3.group"; case .scenarios: "slider.horizontal.3" }
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
                        Button("Новая папка", systemImage: "folder.badge.plus") { workspace.mkdir("Новая папка") }
                            .disabled(workspace.entries.contains(where: { $0.name == "Новая папка" }))
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
