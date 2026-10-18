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
    @State private var localCopyPreview: LocalCopyPreview?
    @State private var localCopyPreviewRequest: LocalCopyPreviewRequest?
    @State private var localCopyPreviewError: String?
    @State private var isLocalCopyConfirmationPresented = false
    @State private var isLocalReplaceConfirmationPresented = false
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
        .qadrenAccessibilityPresentation()
        .tint(QDTheme.color(.focus, for: colorScheme))
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
                        Menu("Размер сетки", systemImage: "rectangle.grid.1x2") {
                            ForEach(MonitoringGridScale.allCases) { scale in
                                Button {
                                    dashboardLayout.setGridScale(scale)
                                } label: {
                                    if dashboardLayout.gridScale == scale {
                                        Label(scale.title, systemImage: "checkmark")
                                    } else {
                                        Text(scale.title)
                                    }
                                }
                            }
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
        .onReceive(NotificationCenter.default.publisher(for: .qadrenStorageConnectionsDidChange)) { _ in
            connections.refresh()
        }
        .onChange(of: localWorkspace.root) { _ in invalidateLocalCopyPreviewIfNeeded() }
        .onChange(of: localWorkspace.relativePath) { _ in invalidateLocalCopyPreviewIfNeeded() }
        .onChange(of: localWorkspace.selectedNames) { _ in invalidateLocalCopyPreviewIfNeeded() }
        .onChange(of: rightLocalWorkspace.root) { _ in invalidateLocalCopyPreviewIfNeeded() }
        .onChange(of: rightLocalWorkspace.relativePath) { _ in invalidateLocalCopyPreviewIfNeeded() }
        .alert("Разрешить загрузку и копирование?", isPresented: $isLocalCopyConfirmationPresented) {
            Button("Отмена", role: .cancel) {}
            Button("Загрузить и скопировать") { copyConfirmedLocalFile() }
        } message: {
            Text("macOS может скачать выбранный исходный файл из File Provider на этот Mac. QADREN скопирует только этот файл и не заменит существующий файл назначения.")
        }
        .alert("Разрешить загрузку и замену?", isPresented: $isLocalReplaceConfirmationPresented) {
            Button("Отмена", role: .cancel) {}
            Button("Загрузить и заменить", role: .destructive) { replaceConfirmedLocalFile() }
        } message: {
            Text("macOS может скачать выбранный исходный файл из File Provider на этот Mac. QADREN заменит только существующий выбранный файл назначения. Новой версии файла не создаётся.")
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

                if dashboardLayout.visibleWidgets.isEmpty {
                    QDEmptyState(
                        title: "На панели нет виджетов",
                        detail: "Откройте режим компоновки, чтобы вернуть скрытые виджеты или сбросить раскладку.",
                        symbol: "rectangle.3.group"
                    )
                    .padding(12)
                } else {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: CGFloat(dashboardLayout.gridScale.minimumWidth)), spacing: 10)], spacing: 10) {
                        ForEach(dashboardLayout.visibleWidgets) { widget in
                            monitoringWidget(widget)
                        }
                    }
                    .padding(12)
                }

                HStack(spacing: 8) {
                    QDStatusLabel(kind: workerStatusKind, text: "Worker: \(workerStatus)")
                    Divider().frame(height: 14)
                    Text("Обновлено по запросу").font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    QDStatusLabel(kind: .unsupported, text: "Файловые операции пока недоступны")
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
                QDStatusLabel(kind: jobWidgetStatus, text: "Задания: \(jobPresentationStatus.title)")
                if widget.density == .compact {
                    QDMetric(value: "\(visibleJobs.count)", label: "в инвентаре", provenance: "локальный JobStore")
                } else if visibleJobs.isEmpty {
                    QDEmptyState(title: "Заданий нет", detail: "История появится после безопасного выполнения через общий Job Engine.", symbol: "tray")
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
                        QDMetric(value: "\(visibleJobs.count)", label: "в инвентаре", provenance: "локальный JobStore")
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
                        Text("Источник: локальный JobStore; автоматическое исполнение не заявлено.")
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
                        JobList(jobs: visibleJobs)
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
                FileManagerPane(
                    title: "Левая панель", workspace: localWorkspace,
                    localConnections: localConnections,
                    addLocalComputer: connections.addLocalComputer,
                    setLocalConnectionEnabled: connections.setEnabled,
                    connectionError: connections.error
                )
                Divider()
                FileManagerPane(
                    title: "Правая панель", workspace: rightLocalWorkspace,
                    localConnections: localConnections,
                    addLocalComputer: connections.addLocalComputer,
                    setLocalConnectionEnabled: connections.setEnabled,
                    connectionError: connections.error
                )
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            HStack(spacing: 10) {
                Button("Проверить план копирования") {
                    previewSelectedLocalCopy()
                }
                .disabled(localWorkspace.selectedRegularFilePaths.count != 1 || localWorkspace.root == nil || rightLocalWorkspace.root == nil)
                Button("Копировать файл") {
                    isLocalCopyConfirmationPresented = true
                }
                .disabled(localCopyPreview == nil || localCopyPreview?.destination_exists == true || localCopyPreviewRequest?.isCurrent(source: localWorkspace, destination: rightLocalWorkspace) != true)
                Button("Заменить файл", role: .destructive) {
                    isLocalReplaceConfirmationPresented = true
                }
                .disabled(localCopyPreview == nil || localCopyPreview?.destination_exists != true || localCopyPreviewRequest?.isCurrent(source: localWorkspace, destination: rightLocalWorkspace) != true)
                Text("Только метаданные; файл и Job не создаются.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
            }
            .padding(.horizontal, 16)
            if let preview = localCopyPreview,
               let request = localCopyPreviewRequest,
               request.isCurrent(source: localWorkspace, destination: rightLocalWorkspace) {
                Notice(
                    text: copyPreviewText(preview),
                    symbol: preview.destination_exists ? "exclamationmark.triangle" : "doc.text.magnifyingglass"
                )
                .padding(.horizontal, 16)
            }
            if let localCopyPreviewError {
                Notice(text: localCopyPreviewError, symbol: "exclamationmark.triangle")
                    .padding(.horizontal, 16)
            }
            Notice(text: "Создание папок, копирование и контролируемая замена файлов идут только через Rust JobStore. Каждая операция требует отдельного подтверждения загрузки исходного файла; перемещение, переименование и удаление выключены.", symbol: "shield")
                .padding(16)
        }
    }

    private func previewSelectedLocalCopy() {
        let sources = localWorkspace.selectedRegularFilePaths
        guard sources.count == 1, let source = sources.first else { return }
        do {
            let request = try localWorkspace.copyPreviewRequest(sourceRelativePath: source, to: rightLocalWorkspace)
            let preview = try localWorkspace.previewCopy(sourceRelativePath: source, to: rightLocalWorkspace)
            localCopyPreviewRequest = request
            localCopyPreview = preview
            localCopyPreviewError = nil
        } catch {
            localCopyPreview = nil
            localCopyPreviewRequest = nil
            localCopyPreviewError = error.localizedDescription
        }
    }

    private func invalidateLocalCopyPreviewIfNeeded() {
        guard let request = localCopyPreviewRequest,
              !request.isCurrent(source: localWorkspace, destination: rightLocalWorkspace)
        else { return }
        localCopyPreview = nil
        localCopyPreviewRequest = nil
    }

    private func copyConfirmedLocalFile() {
        guard let request = localCopyPreviewRequest else { return }
        do {
            _ = try localWorkspace.copyFile(request: request, to: rightLocalWorkspace)
            localCopyPreview = nil
            localCopyPreviewRequest = nil
            localCopyPreviewError = nil
            refreshJobs()
        } catch {
            localCopyPreviewError = error.localizedDescription
        }
    }

    private func replaceConfirmedLocalFile() {
        guard let request = localCopyPreviewRequest else { return }
        do {
            _ = try localWorkspace.replaceFile(request: request, to: rightLocalWorkspace)
            localCopyPreview = nil
            localCopyPreviewRequest = nil
            localCopyPreviewError = nil
            refreshJobs()
        } catch {
            localCopyPreviewError = error.localizedDescription
        }
    }

    private func copyPreviewText(_ preview: LocalCopyPreview) -> String {
        let size = ByteCountFormatter.string(fromByteCount: Int64(clamping: preview.size), countStyle: .file)
        let collision = preview.destination_exists ? " Назначение занято. Подтвердите загрузку исходного файла для контролируемой замены." : " Назначение свободно. Подтвердите загрузку исходного файла для копирования."
        return "План: \(preview.source_relative) → \(preview.destination_relative), \(size).\(collision)"
    }

    private var scenarioSettingsWorkspace: some View {
        SettingsPanelView(connections: connections)
    }

    private var localConnections: [StorageEntry] {
        connections.entries.filter { $0.adapter == "local_filesystem" }
    }

    private func refreshCurrentWorkspace() {
        switch workspace {
        case .monitoring:
            refreshJobs()
            connections.refresh()
        case .scenarios:
            connections.refresh()
        case .files:
            connections.refresh()
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
    private var jobPresentationStatus: JobPresentationStatus {
        JobPresentationStatus.aggregate(
            visibleJobs.map(\.state), inventoryFailed: jobError != nil
        )
    }
    private var jobWidgetStatus: QDStatusKind {
        jobStatusKind(for: jobPresentationStatus)
    }
    private var jobQueueSummary: JobQueueSummary {
        JobQueueSummary(states: visibleJobs.map(\.state))
    }
    private var connectionWidgetStatus: QDStatusKind {
        statusKind(for: connectionPresentationStatus)
    }

    private func connectionStatusKind(for entry: StorageEntry) -> QDStatusKind {
        statusKind(for: ConnectionPresentationStatus(enabled: entry.enabled, registryStatus: entry.status))
    }

    private func connectionStatusTitle(for entry: StorageEntry) -> String {
        ConnectionPresentationStatus(enabled: entry.enabled, registryStatus: entry.status).title
    }

    private var connectionPresentationStatus: ConnectionPresentationStatus {
        ConnectionPresentationStatus.aggregate(
            connections.entries.map { ConnectionPresentationStatus(enabled: $0.enabled, registryStatus: $0.status) },
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

private struct FileManagerPane: View {
    let title: String
    @ObservedObject var workspace: LocalWorkspace
    let localConnections: [StorageEntry]
    let addLocalComputer: () -> String?
    let setLocalConnectionEnabled: (String, Bool) -> Bool
    let connectionError: String?
    @State private var newDirectoryName = ""
    @State private var selectedConnectionID = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(title).font(.headline)
                Picker("Подключение", selection: $selectedConnectionID) {
                    Text("Выберите подключение").tag("")
                    ForEach(localConnections) { connection in
                        Text(connection.name).tag(connection.id)
                    }
                }
                .frame(maxWidth: 220)
                .onChange(of: selectedConnectionID) { connectionID in
                    workspace.bind(connectionID: connectionID)
                }
                Spacer()
                Button(folderButtonTitle) { chooseFolder() }
                Button("Обновить", systemImage: "arrow.clockwise") { workspace.refresh() }.disabled(workspace.root == nil)
            }
            .padding(.horizontal, 18).padding(.top, 16)
            .onAppear {
                selectFirstAvailableConnectionIfNeeded()
            }
            .onChange(of: localConnections.map(\.id)) { _ in
                selectFirstAvailableConnectionIfNeeded()
            }
            if let root = workspace.root {
                VStack(alignment: .leading, spacing: 8) {
                    Text(root).font(.caption).textSelection(.enabled)
                    HStack {
                        Button("Вверх", systemImage: "chevron.up") { workspace.up() }.disabled(workspace.relativePath.isEmpty)
                        Text(workspace.relativePath.isEmpty ? "/" : "/" + workspace.relativePath).font(.caption.monospaced()).textSelection(.enabled)
                        Spacer()
                        if !workspace.selectedNames.isEmpty { Text("Выбрано: \(workspace.selectedNames.count)").font(.caption).foregroundStyle(.secondary) }
                    }
                    HStack(spacing: 8) {
                        TextField("Имя новой папки", text: $newDirectoryName)
                        Button("Создать папку") {
                            workspace.createDirectory(
                                named: newDirectoryName,
                                connectionID: selectedConnectionID
                            )
                            newDirectoryName = ""
                        }
                        .disabled(
                            !selectedConnectionIsAvailable
                                || newDirectoryName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                        )
                    }
                    Button("Показать неопределённые операции") {
                        workspace.findDirectoryReconciliations(connectionID: selectedConnectionID)
                    }
                    .disabled(!selectedConnectionIsAvailable)
                    Button("Показать неопределённые копирования") {
                        workspace.findFileCopyReconciliations(connectionID: selectedConnectionID)
                    }
                    .disabled(!selectedConnectionIsAvailable)
                    ForEach(workspace.reconciliationJobIDs, id: \.self) { jobID in
                        HStack(spacing: 8) {
                            Text("Создание папки требует проверки")
                                .font(.caption)
                                .foregroundStyle(.orange)
                            Button("Проверить операцию") {
                                workspace.reconcileDirectoryCreate(
                                    jobID: jobID,
                                    connectionID: selectedConnectionID
                                )
                            }
                            .disabled(!selectedConnectionIsAvailable)
                            Text(jobID)
                                .font(.caption.monospaced())
                                .textSelection(.enabled)
                        }
                    }
                    ForEach(workspace.copyReconciliationJobIDs, id: \.self) { jobID in
                        HStack(spacing: 8) {
                            Text("Копирование требует проверки")
                                .font(.caption)
                                .foregroundStyle(.orange)
                            Button("Проверить операцию") {
                                workspace.reconcileFileCopy(jobID: jobID, connectionID: selectedConnectionID)
                            }
                            .disabled(!selectedConnectionIsAvailable)
                            Text(jobID).font(.caption.monospaced()).textSelection(.enabled)
                        }
                    }
                }.padding(.horizontal, 18)
                List(workspace.entries) { entry in
                    HStack {
                        Toggle("", isOn: Binding(
                            get: { workspace.selectedNames.contains(entry.name) },
                            set: { workspace.setSelected(entry.name, isSelected: $0) }
                        )).labelsHidden()
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
            if let connectionError { Notice(text: connectionError, symbol: "exclamationmark.triangle").padding(12) }
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
    }

    private var folderButtonTitle: String {
        guard let connection = selectedConnection else {
            return "Добавить подключение и выбрать папку"
        }
        return connection.enabled ? "Выбрать папку" : "Включить и выбрать папку"
    }

    private var selectedConnection: StorageEntry? {
        localConnections.first { $0.id == selectedConnectionID }
    }

    private var selectedConnectionIsAvailable: Bool {
        selectedConnection?.enabled == true
    }

    private func selectFirstAvailableConnectionIfNeeded() {
        guard selectedConnection == nil else { return }
        guard let connection = localConnections.first(where: \.enabled) ?? localConnections.first else { return }
        selectedConnectionID = connection.id
        workspace.bind(connectionID: connection.id)
    }

    private func chooseFolder() {
        if selectedConnectionIsAvailable {
            workspace.chooseRoot(for: selectedConnectionID)
            return
        }
        if let connection = selectedConnection {
            guard setLocalConnectionEnabled(connection.id, true) else { return }
            selectedConnectionID = connection.id
            workspace.bind(connectionID: connection.id)
            workspace.chooseRoot(for: connection.id)
            return
        }
        guard let connectionID = addLocalComputer() else { return }
        selectedConnectionID = connectionID
        workspace.bind(connectionID: connectionID)
        workspace.chooseRoot(for: connectionID)
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
