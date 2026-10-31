import SwiftUI

struct MainView: View {
    @Environment(\.colorScheme) private var colorScheme
    @StateObject private var connections = StorageConnections()
    @StateObject private var dashboardLayout = DashboardLayoutStore()
    /// Release 1 has one monitoring window, so its panel ID is an app
    /// preference. Window state is not restored after a forced quit, and a
    /// window-scoped ID then opened a new default panel every launch.
    @AppStorage("qadren.monitoring.panel-id") private var monitoringPanelID = ""
    /// The former window-scoped ID; read once to keep the current layout.
    @SceneStorage("qadren.monitoring.panel-id") private var windowPanelID = ""
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
            case .monitoring: monitoringDashboard
            case .scenarios: scenarioSettingsWorkspace
            }
        }
        .qadrenAccessibilityPresentation()
        .tint(QDTheme.color(.focus, for: colorScheme))
        .navigationTitle(workspace.title)
        .background(titlebarAccessory)
        .background(InitialFocusReset().frame(width: 0, height: 0))
        .background(layoutUndoShortcuts)
        .onAppear {
            if workspace == .monitoring {
                let panelID = [monitoringPanelID, windowPanelID].first { !$0.isEmpty } ?? UUID().uuidString
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

    /// The Settings window keeps the system title bar with the logo and the
    /// sidebar button; the monitoring window draws `MonitoringHeader`.
    @ViewBuilder
    private var titlebarAccessory: some View {
        if workspace == .scenarios {
            TitlebarLogoAccessory(toggleSidebar: toggleSettingsSidebar, sidebarVisible: !settingsSidebarHidden)
                .frame(width: 0, height: 0)
        }
    }

    /// Cmd-Z and Shift-Cmd-Z undo layout edits, including a removed widget,
    /// without separate toolbar buttons.
    @ViewBuilder
    private var layoutUndoShortcuts: some View {
        if workspace == .monitoring && isEditingLayout {
            ZStack {
                Button("Отменить компоновку") { dashboardLayout.undo() }
                    .keyboardShortcut("z", modifiers: .command)
                    .disabled(!dashboardLayout.canUndo)
                Button("Вернуть компоновку") { dashboardLayout.redo() }
                    .keyboardShortcut("z", modifiers: [.command, .shift])
                    .disabled(!dashboardLayout.canRedo)
            }
            .opacity(0)
            .accessibilityHidden(true)
        }
    }

    private var monitoringDashboard: some View {
        VStack(spacing: 0) {
            MonitoringHeader(isEditing: $isEditingLayout) { dashboardLayout.add($0) }
            monitoringCanvas
                .overlay(MonitoringScreenFrame())
        }
        .background(QDCyber.color(.canvas, for: colorScheme))
        .ignoresSafeArea(.container, edges: .top)
    }

    private var monitoringCanvas: some View {
        ScrollView {
            VStack(spacing: 10) {
                if let alert = monitoringAlert {
                    let failed = QDStatusKind.failed.cyberColor(for: colorScheme)
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        Text("Требуется внимание")
                            .font(QDCyber.heading(13))
                            .tracking(1.8)
                            .textCase(.uppercase)
                            .foregroundStyle(failed)
                        Text(alert)
                            .font(QDCyber.heading(14, weight: .medium))
                            .foregroundStyle(QDCyber.color(.primaryText, for: colorScheme))
                            .textSelection(.enabled)
                        Spacer(minLength: 0)
                    }
                    .padding(.vertical, 10).padding(.leading, 18).padding(.trailing, 12)
                    .background(failed.opacity(0.12))
                    .overlay(QDCyberCutShape().stroke(failed, lineWidth: 1.5))
                    .clipShape(QDCyberCutShape())
                    .padding(.horizontal, 16).padding(.top, 16)
                    .accessibilityElement(children: .combine)
                }

                if dashboardLayout.widgets.isEmpty {
                    QDEmptyState(
                        title: "На панели нет виджетов",
                        detail: isEditingLayout
                            ? "Нажмите «+ Виджет» в верхней панели."
                            : "Нажмите «Компоновка», затем «+ Виджет».",
                        symbol: "square.dashed"
                    )
                    .padding(12)
                } else {
                    // Top alignment: widgets of different heights start on one line.
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 270), spacing: 10, alignment: .top)], spacing: 10) {
                        ForEach(dashboardLayout.widgets) { widget in
                            MonitoringWidgetView(
                                widget: widget, feed: feed, connections: connections,
                                isEditing: isEditingLayout,
                                onSelectScenario: { dashboardLayout.setScenario(widget.id, scenarioID: $0) }
                            ) {
                                layoutActions(for: widget)
                            }
                        }
                    }
                    .padding(16)
                }
            }
        }
    }

    @ViewBuilder
    private func layoutActions(for widget: MonitoringWidgetLayout) -> some View {
        if isEditingLayout {
            Button("Переместить раньше", systemImage: "arrow.left") {
                dashboardLayout.move(widget.id, by: -1)
            }
            .disabled(dashboardLayout.widgets.first?.id == widget.id)
            .labelStyle(.iconOnly)
            .help("Переместить раньше")
            Button("Переместить позже", systemImage: "arrow.right") {
                dashboardLayout.move(widget.id, by: 1)
            }
            .disabled(dashboardLayout.widgets.last?.id == widget.id)
            .labelStyle(.iconOnly)
            .help("Переместить позже")
            if widget.kind.supportedDensities.count > 1 {
                Menu("Плотность виджета", systemImage: "rectangle.3.group") {
                    ForEach(widget.kind.supportedDensities) { density in
                        Button {
                            dashboardLayout.setDensity(widget.id, density: density)
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
            if widget.kind == .scenarios {
                Menu("Сколько сценариев показывать", systemImage: "list.number") {
                    ForEach(ScenariosWidgetLimit.choices, id: \.self) { limit in
                        Button {
                            dashboardLayout.setLimit(widget.id, limit: limit)
                        } label: {
                            let title = ScenariosWidgetLimit.title(limit)
                            if widget.limit == limit {
                                Label(title, systemImage: "checkmark")
                            } else {
                                Text(title)
                            }
                        }
                    }
                }
                .labelStyle(.iconOnly)
                .help("Сколько сценариев показывать")
            }
            Button("Убрать с панели", systemImage: "xmark") {
                dashboardLayout.remove(widget.id)
            }
            .labelStyle(.iconOnly)
            .help("Убрать виджет с панели. Задания и подключения не изменятся.")
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

    private var monitoringAlert: String? {
        // A service failure stays visible when the System widget is not on the panel.
        let service = feed.serviceHealth
        if service.kind.usesAttentionBorder, !dashboardLayout.widgets.contains(where: { $0.kind == .system }) {
            return "Фоновая служба: " + service.title
        }
        if let jobError = feed.jobError { return "Журнал заданий: " + jobError }
        if let error = connections.error { return "Подключения: " + error }
        if dashboardLayout.recoveredFromInvalidLayout {
            return "Сохранённую раскладку не удалось прочитать. Показана безопасная стандартная панель; исходная запись не удалена."
        }
        return nil
    }
}

enum DispatcherSection: String, CaseIterable, Hashable, Identifiable {
    case monitoring, scenarios
    var id: Self { self }
    var title: String {
        switch self { case .monitoring: "Q.A.D.R.E.N. Контрольная панель"; case .scenarios: "Настройки" }
    }
    var symbol: String {
        switch self { case .monitoring: "rectangle.3.group"; case .scenarios: "slider.horizontal.3" }
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

