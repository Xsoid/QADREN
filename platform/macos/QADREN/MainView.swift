import SwiftUI

struct MainView: View {
    @Environment(\.openWindow) private var openWindow
    @Environment(\.colorScheme) private var colorScheme
    @StateObject private var connections = StorageConnections()
    @StateObject private var dashboardLayout = DashboardLayoutStore()
    @SceneStorage("qadren.monitoring.panel-id") private var monitoringPanelID = ""
    @StateObject private var feed = MonitoringFeed()
    @State private var isEditingLayout = false
    /// Survives a recreated window content and remembers the user's choice.
    @SceneStorage("qadren.settings.sidebar-hidden") private var settingsSidebarHidden = false
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
        .qadrenAccessibilityPresentation()
        .tint(QDTheme.color(.focus, for: colorScheme))
        .navigationTitle(workspace.title)
        .background(
            TitlebarLogoAccessory(
                toggleSidebar: workspace == .scenarios ? toggleSettingsSidebar : nil,
                sidebarVisible: !settingsSidebarHidden
            )
            .frame(width: 0, height: 0)
        )
        .background(InitialFocusReset().frame(width: 0, height: 0))
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                // The Settings window has no toolbar actions: each action
                // lives in the section that it changes.
                if workspace == .monitoring {
                    Menu {
                        Button("Контрольная панель") { openWindow(id: "monitoring") }
                        Button("Настройки") { openWindow(id: "scenario-settings") }
                    } label: {
                        Label("Пространство", systemImage: "rectangle.3.group")
                    }
                    Button("Обновить", systemImage: "arrow.clockwise") { refreshCurrentWorkspace() }
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
                    }
                }
            }
        }
        .onAppear {
            if workspace == .monitoring {
                let panelID = monitoringPanelID.isEmpty ? UUID().uuidString : monitoringPanelID
                monitoringPanelID = panelID
                dashboardLayout.load(panelID: panelID)
                feed.start()
            }
        }
        .onDisappear { feed.stop() }
        .onReceive(NotificationCenter.default.publisher(for: .qadrenStorageConnectionsDidChange)) { _ in
            connections.refresh()
        }
    }

    private var monitoringWorkspace: some View {
        monitoringDashboard
    }

    private var monitoringDashboard: some View {
        ScrollView {
            VStack(spacing: 10) {
                if let alert = monitoringAlert {
                    HStack(alignment: .top, spacing: 8) {
                        QDStatusLabel(kind: .failed, text: "Требуется внимание")
                        Text(alert)
                            .font(.caption)
                            .foregroundStyle(QDTheme.color(.primaryText, for: colorScheme))
                            .textSelection(.enabled)
                        Spacer(minLength: 0)
                    }
                    .padding(12)
                    .background(QDTheme.color(.panel, for: colorScheme), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .overlay {
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .stroke(QDTheme.semanticColor(.failed, for: colorScheme), lineWidth: 1)
                    }
                    .padding(.horizontal, 12).padding(.top, 12)
                    .accessibilityElement(children: .combine)
                }

                // Top alignment: widgets of different heights start on one line.
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 270), spacing: 10, alignment: .top)], spacing: 10) {
                    ForEach(dashboardLayout.widgets) { widget in
                        monitoringWidget(widget)
                    }
                }
                .padding(12)

                TimelineView(.periodic(from: .now, by: MonitoringFeed.refreshInterval)) { context in
                    HStack(spacing: 8) {
                        QDStatusLabel(kind: workerStatusKind, text: workerStatus)
                        Divider().frame(height: 14)
                        Text("Служба: \(LocalWorkerController.statusText)").font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        freshnessLabel(now: context.date)
                    }
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
                        Text("Источник: запрос status к Worker и локальный JobStore; содержимое файлов не читается.")
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
                QDStatusLabel(kind: jobWidgetStatus, text: "Задания: \(jobPresentationStatus.title)")
                if widget.density == .compact {
                    QDMetric(value: "\(feed.jobTotal)", label: "в инвентаре", provenance: "локальный JobStore")
                } else if visibleJobs.isEmpty {
                    QDEmptyState(title: "Заданий нет", detail: "Задания появятся, когда Worker выполнит сценарий по вашему запросу или по расписанию.", symbol: "tray")
                } else {
                    JobList(jobs: visibleJobs)
                }
            }
        case .queue:
            QDWidgetFrame(title: "Очередь", symbol: "list.number", status: jobWidgetStatus, actions: {
                layoutActions(for: widget)
            }) {
                if let jobError {
                    QDEmptyState(title: "Очередь недоступна", detail: jobError, symbol: "exclamationmark.triangle")
                } else {
                    QDStatusLabel(kind: jobWidgetStatus, text: "Очередь: \(jobPresentationStatus.title)")
                    if widget.density == .compact {
                        QDMetric(value: "\(feed.jobTotal)", label: "в инвентаре", provenance: "локальный JobStore")
                    } else {
                        LazyVGrid(
                            columns: [GridItem(.adaptive(minimum: 150, maximum: 220), spacing: 12)],
                            alignment: .leading,
                            spacing: 8
                        ) {
                            QDMetric(value: "\(jobQueueSummary.active)", label: "выполняется", provenance: "локальный JobStore")
                            QDMetric(value: "\(jobQueueSummary.waiting)", label: "в очереди", provenance: "локальный JobStore")
                            QDMetric(value: "\(jobQueueSummary.unknown)", label: "неизвестно", provenance: "требует сверки")
                        }
                    }
                    if widget.density != .compact {
                        Text("Порядок, прогресс и оценка времени появятся только когда их подтвердит Core.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    if widget.density == .expanded {
                        Text("Источник: локальный JobStore; Worker выполняет запуски сценариев по расписанию.")
                            .font(.caption2).foregroundStyle(.secondary)
                    }
                }
            }
        case .connections:
            QDWidgetFrame(title: "Подключения", symbol: "externaldrive", status: connectionWidgetStatus, actions: {
                layoutActions(for: widget)
            }) {
                if let error = connections.error {
                    QDEmptyState(title: "Подключения недоступны", detail: error, symbol: "exclamationmark.triangle")
                } else if widget.density == .compact {
                    QDMetric(value: "\(connections.entries.count)", label: "подключений", provenance: "локальный Registry")
                } else if connections.entries.isEmpty {
                    VStack(alignment: .leading, spacing: 10) {
                        Text("Подключения ещё не настроены.").foregroundStyle(.secondary)
                        Button("Открыть настройки") { openWindow(id: "scenario-settings") }
                    }
                } else {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(widget.density == .expanded ? connections.entries : Array(connections.entries.prefix(4))) { entry in
                            let status = connectionStatusKind(for: entry)
                            let statusTitle = connectionStatusTitle(for: entry)
                            HStack(spacing: 8) {
                                Image(systemName: status.symbol)
                                    .foregroundStyle(status.color(for: colorScheme))
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(entry.name).lineLimit(1)
                                    Text("\(adapterTitle(entry.adapter)) · \(statusTitle)")
                                        .font(.caption).foregroundStyle(.secondary)
                                }
                                Spacer()
                            }
                            .accessibilityElement(children: .ignore)
                            .accessibilityLabel("\(entry.name), \(adapterTitle(entry.adapter)), \(statusTitle)")
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
                        JobList(jobs: visibleJobs)
                    }
                }
            }.padding(28)
        }
    }

    private var scenarioSettingsWorkspace: some View {
        SettingsPanelView(
            connections: connections,
            sidebarVisibility: Binding(
                get: { settingsSidebarHidden ? .detailOnly : .all },
                set: { settingsSidebarHidden = $0 == .detailOnly }
            )
        )
    }

    private func toggleSettingsSidebar() {
        withAnimation {
            settingsSidebarHidden.toggle()
        }
    }

    private func refreshCurrentWorkspace() {
        switch workspace {
        case .monitoring:
            Task { await feed.refresh() }
            connections.refresh()
        case .scenarios:
            connections.refresh()
        }
    }

    private var jobs: [JobRow] { feed.jobs }
    private var jobError: String? { feed.jobError }

    /// Worker state comes only from the IPC `status` request. The in-process
    /// Core library cannot see the separate Worker process.
    private var workerStatus: String {
        guard let checked = feed.lastWorkerCheck else { return feed.worker.title }
        return "\(feed.worker.title) · \(checked.formatted(date: .omitted, time: .standard))"
    }
    private var workerStatusKind: QDStatusKind {
        switch feed.worker {
        case .unchecked: .unknown
        case .responding: .ready
        case .notResponding: .needsAction
        case .unavailable: .offline
        }
    }

    @ViewBuilder
    private func freshnessLabel(now: Date) -> some View {
        switch MonitoringFreshness.evaluate(
            lastSuccess: feed.lastInventoryRead, now: now, interval: MonitoringFeed.refreshInterval
        ) {
        case .never:
            QDStatusLabel(kind: .unknown, text: "Данные ещё не прочитаны")
        case let .fresh(date):
            Text("Обновлено \(date.formatted(date: .omitted, time: .standard))")
                .font(.caption).foregroundStyle(.secondary)
        case let .stale(date):
            QDStatusLabel(kind: .needsAction, text: "Данные устарели: последнее чтение \(date.formatted(date: .omitted, time: .standard))")
        }
    }
    private var jobPresentationStatus: JobPresentationStatus {
        // D-024: the states present in the counts of all Jobs.
        JobPresentationStatus.aggregate(
            feed.jobCounts.filter { $0.value > 0 }.map(\.key), inventoryFailed: jobError != nil
        )
    }
    private var jobWidgetStatus: QDStatusKind {
        jobStatusKind(for: jobPresentationStatus)
    }
    private var jobQueueSummary: JobQueueSummary {
        JobQueueSummary(counts: feed.jobCounts)
    }
    private var connectionWidgetStatus: QDStatusKind {
        statusKind(for: connectionPresentationStatus)
    }

    private func connectionStatusKind(for entry: StorageEntry) -> QDStatusKind {
        statusKind(for: ConnectionPresentationStatus(adapter: entry.adapter, enabled: entry.enabled, registryStatus: entry.status))
    }

    private func connectionStatusTitle(for entry: StorageEntry) -> String {
        ConnectionPresentationStatus(adapter: entry.adapter, enabled: entry.enabled, registryStatus: entry.status).title
    }

    private var connectionPresentationStatus: ConnectionPresentationStatus {
        ConnectionPresentationStatus.aggregate(
            connections.entries.map {
                ConnectionPresentationStatus(adapter: $0.adapter, enabled: $0.enabled, registryStatus: $0.status)
            },
            registryFailed: connections.error != nil
        )
    }

    private func statusKind(for status: ConnectionPresentationStatus) -> QDStatusKind {
        switch status {
        case .ready: .ready
        case .needsAction: .needsAction
        case .failed: .failed
        case .offline: .offline
        case .disconnected: .offline
        case .paused: .paused
        case .waiting: .waiting
        case .unknown: .unknown
        }
    }

    private var monitoringAlert: String? {
        if let jobError { return "Журнал заданий: " + jobError }
        if let error = connections.error { return "Подключения: " + error }
        if dashboardLayout.recoveredFromInvalidLayout {
            return "Сохранённую раскладку не удалось прочитать. Показана безопасная стандартная панель; исходная запись не удалена."
        }
        return nil
    }
    private var coreStatus: String {
        switch CoreBridge.status {
        case "ready - worker idle", "Core ready - worker idle": "Библиотека Core загружена"
        default: CoreBridge.status
        }
    }
    private func adapterTitle(_ adapter: String) -> String { StorageConnector(adapter: adapter)?.title ?? adapter }

    /// `g1-*` is a test-only durable fixture created by the early mock runner.
    /// It is retained for safe diagnostics, but is never product queue data.
    private var visibleJobs: [JobRow] {
        jobs.filter { !$0.id.hasPrefix("g1-") }
    }
}

enum DispatcherSection: String, CaseIterable, Hashable, Identifiable {
    case monitoring, scenarios
    var id: Self { self }
    var title: String {
        switch self { case .monitoring: "Контрольная панель"; case .scenarios: "Настройки" }
    }
    var symbol: String {
        switch self { case .monitoring: "rectangle.3.group"; case .scenarios: "slider.horizontal.3" }
    }
}

private enum MonitoringView {
    case dashboard, jobs
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
    @Environment(\.colorScheme) private var colorScheme
    var body: some View {
        VStack(spacing: 0) {
            ForEach(jobs) { job in
                let presentation = JobPresentationStatus(state: job.state)
                let status = jobStatusKind(for: presentation)
                HStack(spacing: 12) {
                    Image(systemName: status.symbol).foregroundStyle(status.color(for: colorScheme))
                    VStack(alignment: .leading, spacing: 3) {
                        Text(job.id).textSelection(.enabled)
                        Text(job.scenario_id == nil ? "Разовое задание" : "Задание сценария").font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Text(presentation.title).font(.callout.weight(.medium))
                }.padding(.vertical, 10)
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel("\(job.id), \(presentation.title)")
                if job.id != jobs.last?.id { Divider() }
            }
        }
    }
}

private func jobStatusKind(for status: JobPresentationStatus) -> QDStatusKind {
    switch status {
    case .empty: .ready
    case .ready: .ready
    case .active: .active
    case .waiting: .waiting
    case .paused: .paused
    case .needsAction: .needsAction
    case .failed: .failed
    case .unknown: .unknown
    case .completed: .ready
    case .cancelled: .paused
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
