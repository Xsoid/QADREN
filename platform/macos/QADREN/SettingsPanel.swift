import SwiftUI
import AppKit

@MainActor
final class ScenarioSettings: ObservableObject {
    @Published private(set) var entries: [ScenarioEntry] = []
    @Published private(set) var error: String?

    func refresh() {
        do {
            entries = try StorageRegistryBridge.listScenarios()
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    func create(
        name: String,
        leftID: String,
        leftRoot: String,
        rightID: String,
        rightRoot: String,
        direction: String,
        scope: ScenarioScopeDraft = ScenarioScopeDraft()
    ) -> String? {
        do {
            let id = try StorageRegistryBridge.createScenario(
                name: name,
                leftID: leftID,
                leftRoot: leftRoot,
                rightID: rightID,
                rightRoot: rightRoot,
                direction: direction,
                selectedPaths: scope.selectedPaths.sorted(),
                deletionMode: scope.deletionMode
            )
            if !scope.retryUntilSuccess {
                try StorageRegistryBridge.setScenarioRetryUntilSuccess(id: id, enabled: false)
            }
            refresh()
            return nil
        } catch {
            self.error = error.localizedDescription
            return self.error
        }
    }

    func remove(_ id: String) -> String? {
        do {
            try StorageRegistryBridge.removeScenario(id: id)
            refresh()
            return nil
        } catch {
            return error.localizedDescription
        }
    }

    func setEnabled(_ id: String, enabled: Bool) {
        do {
            try StorageRegistryBridge.setScenarioEnabled(id: id, enabled: enabled)
            refresh()
        } catch {
            self.error = error.localizedDescription
        }
    }

    func update(
        id: String,
        name: String,
        leftID: String,
        leftRoot: String,
        rightID: String,
        rightRoot: String,
        direction: String,
        scope: ScenarioScopeDraft = ScenarioScopeDraft()
    ) -> String? {
        do {
            let current = entries.first { $0.id == id }
            // A new revision only when the revision configuration changes.
            // The retry mode alone keeps the revision and a running batch.
            let revisionChanged = current.map {
                $0.name != name.trimmingCharacters(in: .whitespacesAndNewlines)
                    || $0.left_connection_id != leftID || $0.left_root != leftRoot
                    || $0.right_connection_id != rightID || $0.right_root != rightRoot
                    || $0.direction != direction
                    || Set($0.selected_paths) != scope.selectedPaths
                    || $0.deletion_mode != scope.deletionMode.rawValue
            } ?? true
            if revisionChanged {
                try StorageRegistryBridge.updateScenario(
                    id: id,
                    name: name,
                    leftID: leftID,
                    leftRoot: leftRoot,
                    rightID: rightID,
                    rightRoot: rightRoot,
                    direction: direction,
                    selectedPaths: scope.selectedPaths.sorted(),
                    deletionMode: scope.deletionMode
                )
            }
            if current?.retry_until_success != scope.retryUntilSuccess {
                try StorageRegistryBridge.setScenarioRetryUntilSuccess(id: id, enabled: scope.retryUntilSuccess)
            }
            refresh()
            return nil
        } catch {
            self.error = error.localizedDescription
            return self.error
        }
    }
}

@MainActor
final class ScenarioScheduleSettings: ObservableObject {
    @Published private(set) var entries: [ScenarioScheduleEntry] = []
    @Published private(set) var upcoming: [ScenarioScheduleOccurrence] = []
    @Published private(set) var automation = ScenarioAutomationEntry(
        locally_paused: false, pending_automatic_run: false
    )
    @Published private(set) var runState = ScenarioRunState(active: nil, skipped: [])
    @Published private(set) var isStopping = false
    @Published private(set) var error: String?

    func refresh(scenarioID: String) {
        do {
            entries = try StorageRegistryBridge.listScenarioSchedules(scenarioID: scenarioID)
            automation = try StorageRegistryBridge.scenarioAutomation(scenarioID: scenarioID)
            runState = try StorageRegistryBridge.scenarioRunState(scenarioID: scenarioID)
            let now = Date()
            upcoming = try StorageRegistryBridge.dueScenarioSchedules(
                after: now,
                through: now.addingTimeInterval(14 * 24 * 60 * 60)
            )
            .filter { $0.scenario_id == scenarioID }
            error = nil
        } catch {
            upcoming = []
            self.error = error.localizedDescription
        }
    }

    /// "Остановить": the Worker stops the active run (D-025). A planning run
    /// ends now; an executing run ends after the current file.
    func stopRun(scenarioID: String) {
        isStopping = true
        Task {
            do {
                try await Task.detached { try WorkerIPC.stopRun(scenarioID: scenarioID) }.value
                error = nil
            } catch {
                self.error = "Фоновая служба не остановила запуск: " + error.localizedDescription
            }
            isStopping = false
            refresh(scenarioID: scenarioID)
        }
    }

    func report(_ message: String) {
        error = message
    }

    func setLocallyPaused(scenarioID: String, paused: Bool) {
        do {
            try StorageRegistryBridge.setScenarioLocallyPaused(scenarioID: scenarioID, paused: paused)
            refresh(scenarioID: scenarioID)
        } catch {
            self.error = error.localizedDescription
        }
    }

    func create(
        scenarioID: String, timezone: String, intervalHours: UInt8, daysMask: UInt8, hour: UInt8, minute: UInt8
    ) {
        do {
            try StorageRegistryBridge.createScenarioSchedule(
                scenarioID: scenarioID, timezone: timezone, intervalHours: intervalHours,
                daysMask: daysMask, hour: hour, minute: minute
            )
            refresh(scenarioID: scenarioID)
        } catch {
            self.error = error.localizedDescription
        }
    }

    func setEnabled(scenarioID: String, scheduleID: String, enabled: Bool) {
        do {
            try StorageRegistryBridge.setScenarioScheduleEnabled(
                scenarioID: scenarioID, scheduleID: scheduleID, enabled: enabled
            )
            refresh(scenarioID: scenarioID)
        } catch {
            self.error = error.localizedDescription
        }
    }

    func remove(scenarioID: String, scheduleID: String) {
        do {
            try StorageRegistryBridge.removeScenarioSchedule(scenarioID: scenarioID, scheduleID: scheduleID)
            refresh(scenarioID: scenarioID)
        } catch {
            self.error = error.localizedDescription
        }
    }
}

private enum SettingsPanelSection: String, CaseIterable, Hashable, Identifiable {
    case connections
    case scenarios
    case application

    var id: Self { self }
    var title: String {
        switch self {
        case .connections: "Подключения"
        case .scenarios: "Сценарии"
        case .application: "Настройки приложения"
        }
    }
    var symbol: String {
        switch self {
        case .connections: "externaldrive"
        case .scenarios: "arrow.triangle.branch"
        case .application: "gearshape"
        }
    }
}

struct SettingsPanelView: View {
    @ObservedObject var connections: StorageConnections
    /// The window titlebar owns the sidebar button; see TitlebarLogoAccessory.
    @Binding var sidebarVisibility: NavigationSplitViewVisibility
    @StateObject private var scenarios = ScenarioSettings()
    @State private var selection: SettingsPanelSection? = .connections
    @State private var isEditingScenario = false
    @State private var editedScenario: ScenarioEntry?
    @State private var schedulesScenario: ScenarioEntry?
    @State private var previewScenario: ScenarioEntry?

    var body: some View {
        NavigationSplitView(columnVisibility: $sidebarVisibility) {
            List(selection: $selection) {
                Section("Настройка") {
                    ForEach(SettingsPanelSection.allCases) { section in
                        Label(section.title, systemImage: section.symbol).tag(section)
                    }
                }
            }
            .listStyle(.sidebar)
            .navigationTitle("Настройки")
            .withoutSystemSidebarToggle()
            .withoutTopScrollEdge()
        } detail: {
            Group {
                switch selection ?? .connections {
                case .connections:
                    ConnectionsSettingsView(connections: connections)
                case .scenarios:
                    if let schedulesScenario {
                        ScenarioSchedulesView(
                            scenario: schedulesScenario,
                            done: { self.schedulesScenario = nil }
                        )
                    } else if let previewScenario {
                        ScenarioDryRunView(
                            scenario: previewScenario,
                            done: { self.previewScenario = nil }
                        )
                    } else if isEditingScenario {
                        ScenarioEditorView(
                            connections: scenarioEndpoints,
                            scenario: editedScenario,
                            saveScenario: saveScenario,
                            removeScenario: removeScenario,
                            cancel: { isEditingScenario = false }
                        )
                    } else {
                        ScenariosSettingsView(
                            scenarios: scenarios,
                            connections: connections.entries,
                            createScenario: beginScenarioCreation,
                            editScenario: beginScenarioEditing,
                            editSchedules: { schedulesScenario = $0 },
                            previewScenario: { previewScenario = $0 }
                        )
                    }
                case .application:
                    ApplicationSettingsView()
                }
            }
            .frame(minWidth: 560, minHeight: 500)
            .contentBelowTitlebar()
        }

        .onAppear {
            connections.refresh()
            scenarios.refresh()
        }
    }

    private var scenarioEndpoints: [StorageEntry] {
        // Yandex Disk is a one-way source only; the editor limits it (D-023).
        var endpoints = connections.entries.filter {
            ["local_filesystem", "smb_share", "yandex_disk"].contains($0.adapter)
        }
        if !endpoints.contains(where: { $0.adapter == "local_filesystem" }) {
            endpoints.append(StorageEntry(
                id: "local-computer", adapter: "local_filesystem", name: "Этот Mac",
                enabled: true, status: "connected", credential_ref: nil, provider_account_id: nil
            ))
        }
        return endpoints
    }

    private func beginScenarioCreation() {
        schedulesScenario = nil
        editedScenario = nil
        isEditingScenario = true
    }

    private func beginScenarioEditing(_ scenario: ScenarioEntry) {
        schedulesScenario = nil
        editedScenario = scenario
        isEditingScenario = true
    }

    private func removeScenario(_ id: String) -> String? {
        let result = scenarios.remove(id)
        if result == nil {
            editedScenario = nil
            isEditingScenario = false
        }
        return result
    }

    private func saveScenario(
        _ name: String, _ leftID: String, _ leftRoot: String,
        _ rightID: String, _ rightRoot: String, _ direction: String,
        _ scope: ScenarioScopeDraft
    ) -> String? {
        do {
            let localID: String?
            if leftID == "local-computer" || rightID == "local-computer" {
                localID = try (connections.entries.first { $0.adapter == "local_filesystem" && $0.enabled }?.id
                    ?? StorageRegistryBridge.ensureLocalComputer())
            } else {
                localID = nil
            }
            let resolvedLeftID = leftID == "local-computer" ? localID! : leftID
            let resolvedRightID = rightID == "local-computer" ? localID! : rightID
            let result: String?
            if let editedScenario {
                result = scenarios.update(
                    id: editedScenario.id, name: name,
                    leftID: resolvedLeftID, leftRoot: leftRoot,
                    rightID: resolvedRightID, rightRoot: rightRoot, direction: direction,
                    scope: scope
                )
            } else {
                result = scenarios.create(
                    name: name, leftID: resolvedLeftID, leftRoot: leftRoot,
                    rightID: resolvedRightID, rightRoot: rightRoot, direction: direction,
                    scope: scope
                )
            }
            if result == nil {
                connections.refresh()
                editedScenario = nil
                isEditingScenario = false
            }
            return result
        } catch {
            return error.localizedDescription
        }
    }
}

/// Storage connectors that the product knows (D-020). A connector is the
/// kind of storage; a connection is one configured record of it.
enum StorageConnector: String, CaseIterable, Identifiable {
    case thisMac = "local_filesystem"
    case networkDrive = "smb_share"
    case yandexDisk = "yandex_disk"
    case ispmanager = "ispmanager"

    var id: String { rawValue }

    init?(adapter: String) {
        self.init(rawValue: adapter)
    }

    var title: String {
        switch self {
        case .thisMac: "Этот Mac"
        case .networkDrive: "Сетевой диск (SMB)"
        case .yandexDisk: "Яндекс.Диск"
        case .ispmanager: "ISPmanager"
        }
    }

    var symbol: String {
        switch self {
        case .thisMac: "laptopcomputer"
        case .networkDrive: "server.rack"
        case .yandexDisk: "cloud"
        case .ispmanager: "globe"
        }
    }

    var tint: Color {
        switch self {
        case .thisMac: QDTheme.cyan
        case .networkDrive: QDTheme.blue
        case .yandexDisk: QDTheme.warning
        case .ispmanager: QDTheme.success
        }
    }

    var summary: String {
        switch self {
        case .thisMac: "Папки и диски этого Mac. Одно подключение на Mac, создаётся автоматически."
        case .networkDrive: "Общая папка по протоколу SMB: NAS, компьютер Windows или другой Mac. Логин и пароль хранятся в Keychain."
        case .yandexDisk: "Облачное хранилище. Подключений может быть сколько угодно."
        case .ispmanager: "Панель хостинга ISPmanager 6. Только источник резервной копии: QADREN читает данные и ничего не меняет на сервере. Логин и пароль хранятся в Keychain."
        }
    }
}

/// One screen of the Connections section. The section is one navigation
/// stack inside the Settings window: the list, the connector list, and the
/// connector form. Breadcrumbs go back; no extra window or sheet opens.
private enum ConnectionsRoute: Hashable {
    case list
    case connectors
    case newNetworkDrive
    case editNetworkDrive(String)
    case newISPmanager
    case editISPmanager(String)
}

private struct ConnectionsSettingsView: View {
    @ObservedObject var connections: StorageConnections
    @State private var route: ConnectionsRoute = .list

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                if route != .list {
                    SettingsBreadcrumbs(items: crumbs)
                }
                switch route {
                case .list:
                    connectionList
                case .connectors:
                    ConnectorListView(connections: connections) { connector in
                        if connector == .networkDrive { route = .newNetworkDrive }
                        if connector == .ispmanager { route = .newISPmanager }
                        if connector == .yandexDisk {
                            // The new card has "Войти в Яндекс" (D-023).
                            connections.addYandex()
                            route = .list
                        }
                    }
                case .newNetworkDrive:
                    SMBConnectionForm(
                        connections: connections, editing: nil,
                        onDone: { route = .list }, onCancel: { route = .connectors }
                    )
                case let .editNetworkDrive(id):
                    if let entry = connections.entries.first(where: { $0.id == id }) {
                        SMBConnectionForm(
                            connections: connections, editing: entry,
                            onDone: { route = .list }, onCancel: { route = .list }
                        )
                    }
                case .newISPmanager:
                    ISPmanagerConnectionForm(
                        connections: connections, editing: nil,
                        onDone: { route = .list }, onCancel: { route = .connectors }
                    )
                case let .editISPmanager(id):
                    if let entry = connections.entries.first(where: { $0.id == id }) {
                        ISPmanagerConnectionForm(
                            connections: connections, editing: entry,
                            onDone: { route = .list }, onCancel: { route = .list }
                        )
                    }
                }
            }
            .frame(maxWidth: 900, alignment: .leading)
            .padding(24)
        }
        // Each screen starts at the top.
        .id(route)
        .onAppear {
            connections.refresh()
            // The cards load after the window cleared its initial focus.
            // Opening the section must not select a name field.
            DispatchQueue.main.async { NSApp.keyWindow?.makeFirstResponder(nil) }
        }
        .onChange(of: connections.entries.map(\.id)) { ids in
            if case let .editNetworkDrive(id) = route, !ids.contains(id) { route = .list }
            if case let .editISPmanager(id) = route, !ids.contains(id) { route = .list }
        }
    }

    private var crumbs: [SettingsBreadcrumbs.Item] {
        let root = SettingsBreadcrumbs.Item(title: "Подключения") { route = .list }
        switch route {
        case .list:
            return [root]
        case .connectors:
            return [root, .init(title: "Добавить подключение")]
        case .newNetworkDrive:
            return [
                root,
                .init(title: "Добавить подключение") { route = .connectors },
                .init(title: StorageConnector.networkDrive.title)
            ]
        case .newISPmanager:
            return [
                root,
                .init(title: "Добавить подключение") { route = .connectors },
                .init(title: StorageConnector.ispmanager.title)
            ]
        case let .editNetworkDrive(id), let .editISPmanager(id):
            let name = connections.entries.first { $0.id == id }?.name ?? "Подключение"
            return [root, .init(title: name) { route = .list }, .init(title: "Адрес и вход")]
        }
    }

    @ViewBuilder
    private var connectionList: some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 4) {
                Text("Подключения").font(.largeTitle.bold())
                Text("Хранилища, с которыми работают сценарии. Каждое подключение относится к одному коннектору.")
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button("Добавить подключение", systemImage: "plus") { route = .connectors }
        }
        if connections.hasLegacyYandexToken {
            SettingsNotice(
                text: "Найдена прежняя авторизация Яндекс Диска. Перенос создаёт отдельное подключение и оставляет старый Keychain-элемент до проверки копии.",
                symbol: "key"
            ) {
                Button("Перенести подключение") { connections.importLegacyYandex() }
            }
        }
        if let error = connections.error {
            SettingsNotice(text: error, symbol: "exclamationmark.triangle")
        }
        if connections.entries.filter({ $0.adapter == StorageConnector.thisMac.rawValue }).count > 1 {
            SettingsNotice(
                text: "Найдено несколько подключений «Этот Mac» из старых данных. Нужно одно: удалите лишние. Подключение, которое используют сценарии, удалить нельзя.",
                symbol: "square.on.square"
            )
        }
        ForEach(connections.entries) { entry in
            ConnectionCard(
                entry: entry,
                connections: connections,
                canRemove: canRemove(entry),
                onRemove: { connections.remove(entry.id) },
                onRename: { connections.rename(entry.id, name: $0) },
                onSetEnabled: { connections.setEnabled(entry.id, enabled: $0) },
                onEditAddress: {
                    route = entry.adapter == StorageConnector.ispmanager.rawValue
                        ? .editISPmanager(entry.id) : .editNetworkDrive(entry.id)
                }
            )
            .id(entry.id + (entry.enabled ? ":enabled" : ":disabled") + ":" + entry.name)
        }
        Text("Удаление подключения меняет только локальную запись QADREN и её ключ в Keychain. Файлы в хранилище не удаляются.")
            .font(.caption).foregroundStyle(.secondary)
    }

    /// "This Mac" always exists. Only an extra Local record from older data
    /// can be removed.
    private func canRemove(_ entry: StorageEntry) -> Bool {
        guard entry.adapter == StorageConnector.thisMac.rawValue else { return true }
        return connections.entries.filter { $0.adapter == entry.adapter }.count > 1
    }
}

/// The path of a nested Settings screen. Every item except the last one
/// goes back to its screen; the back button goes one level up.
private struct SettingsBreadcrumbs: View {
    struct Item {
        let title: String
        var action: (() -> Void)?
    }

    let items: [Item]

    var body: some View {
        HStack(spacing: 6) {
            if items.count > 1, let back = items[items.count - 2].action {
                Button("Назад", systemImage: "chevron.left", action: back)
                    .labelStyle(.iconOnly)
                    .help("На уровень выше")
                    .keyboardShortcut("[", modifiers: .command)
            }
            ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                if index > 0 {
                    Image(systemName: "chevron.right").font(.caption).foregroundStyle(.tertiary)
                }
                if index < items.count - 1, let action = item.action {
                    Button(item.title, action: action)
                        .buttonStyle(.link)
                        .lineLimit(1)
                } else {
                    Text(item.title).fontWeight(.semibold).lineLimit(1)
                }
            }
        }
        .font(.callout)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Путь в настройках")
    }
}

/// One connection with a visible boundary: connector icon and color, name,
/// status, and the actions that the connector supports now.
private struct ConnectionCard: View {
    let entry: StorageEntry
    @ObservedObject var connections: StorageConnections
    let canRemove: Bool
    let onRemove: () -> Void
    let onRename: (String) -> Void
    let onSetEnabled: (Bool) -> Void
    let onEditAddress: () -> Void
    @State private var name: String
    @State private var confirmRemoval = false

    init(
        entry: StorageEntry, connections: StorageConnections, canRemove: Bool,
        onRemove: @escaping () -> Void,
        onRename: @escaping (String) -> Void, onSetEnabled: @escaping (Bool) -> Void,
        onEditAddress: @escaping () -> Void
    ) {
        self.entry = entry
        self.connections = connections
        self.canRemove = canRemove
        self.onRemove = onRemove
        self.onRename = onRename
        self.onSetEnabled = onSetEnabled
        self.onEditAddress = onEditAddress
        _name = State(initialValue: entry.name)
    }

    private var connector: StorageConnector? { StorageConnector(adapter: entry.adapter) }
    private var tint: Color { connector?.tint ?? .secondary }
    private var removalMessage: String {
        switch connector {
        case .yandexDisk:
            "Будут удалены только локальная запись QADREN и её ключ в Keychain. Файлы на Яндекс.Диске не изменятся."
        case .networkDrive:
            "Будут удалены только локальная запись QADREN и логин с паролем в Keychain. Файлы на сетевом диске не изменятся."
        case .ispmanager:
            "Будут удалены только локальная запись QADREN и логин с паролем в Keychain. Данные на сервере ISPmanager не изменятся."
        default:
            "Будет удалена только лишняя локальная запись QADREN. Файлы на этом Mac не изменятся."
        }
    }

    private var nameChanged: Bool {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return !trimmed.isEmpty && trimmed != entry.name
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 10, style: .continuous)
        VStack(alignment: .leading, spacing: 14) {
            header
            Divider()
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                GridRow {
                    Text("Название").foregroundStyle(.secondary)
                    HStack {
                        TextField("Название подключения", text: $name)
                            .textFieldStyle(.roundedBorder)
                            .onSubmit { if nameChanged { onRename(name) } }
                        Button("Сохранить") { onRename(name) }
                            .disabled(!nameChanged)
                    }
                }
                GridRow {
                    Text("Работа").foregroundStyle(.secondary)
                    Toggle(entry.enabled ? "Включено" : "Выключено", isOn: Binding(
                        get: { entry.enabled }, set: { onSetEnabled($0) }
                    ))
                    .toggleStyle(.switch)
                }
            }
            if connector == .networkDrive {
                SMBConnectionDetails(entry: entry, connections: connections, onEdit: onEditAddress)
            }
            if connector == .ispmanager {
                ISPmanagerConnectionDetails(entry: entry, connections: connections, onEdit: onEditAddress)
            }
            if connector == .ispmanager {
                Text("Панель — только источник резервной копии. Сценарии резервного копирования с неё появятся на следующем этапе; сейчас доступны вход и список возможностей.")
                    .font(.caption).foregroundStyle(.secondary)
            } else if connector == .yandexDisk {
                YandexConnectionDetails(entry: entry, connections: connections)
                    // The connection object reads `enabled` once.
                    .id(entry.id + (entry.enabled ? "-on" : "-off"))
            } else if connector == .networkDrive {
                Text("Папки сетевого диска выбираются в редакторе сценария. QADREN подключает диск сам, с сохранёнными логином и паролем. До подтверждённой операции файлы на нём не меняются.")
                    .font(.caption).foregroundStyle(.secondary)
            } else {
                Text(entry.enabled
                    ? "Папки выбираются в редакторе сценария. До подтверждённой операции QADREN не передаёт и не удаляет файлы."
                    : "Подключение выключено: сценарии с ним не строят план и не переносят файлы.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if canRemove {
                Button("Удалить подключение", systemImage: "trash", role: .destructive) { confirmRemoval = true }
            }
        }
        .padding(16)
        .background(tint.opacity(0.05), in: shape)
        .overlay(shape.strokeBorder(tint.opacity(0.45), lineWidth: 1))
        .overlay(alignment: .leading) {
            // The connector color marks the left edge of the card.
            UnevenRoundedRectangle(topLeadingRadius: 10, bottomLeadingRadius: 10)
                .fill(tint)
                .frame(width: 4)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("\(entry.name), \(connector?.title ?? entry.adapter)")
        .confirmationDialog("Удалить подключение «\(entry.name)»?", isPresented: $confirmRemoval) {
            Button("Удалить подключение", role: .destructive, action: onRemove)
        } message: {
            Text(removalMessage)
        }
    }

    private var header: some View {
        HStack(alignment: .center, spacing: 12) {
            Image(systemName: connector?.symbol ?? "externaldrive")
                .font(.title2)
                .foregroundStyle(tint)
                .frame(width: 44, height: 44)
                .background(tint.opacity(0.15), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
            VStack(alignment: .leading, spacing: 3) {
                Text(entry.name).font(.title3.weight(.semibold)).lineLimit(1)
                HStack(spacing: 6) {
                    Text(connector?.title ?? entry.adapter)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(tint)
                        .padding(.horizontal, 7).padding(.vertical, 2)
                        .background(tint.opacity(0.15), in: Capsule())
                    Text("ID " + String(entry.id.prefix(8)))
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                        .help(entry.id)
                }
            }
            Spacer()
            if [.networkDrive, .yandexDisk, .ispmanager].contains(connector) && entry.enabled {
                let state = SMBConnectionDetails.status(entry.status)
                QDStatusLabel(kind: state.kind, text: state.text)
            } else if entry.enabled {
                QDStatusLabel(kind: .ready, text: "Включено")
            } else {
                QDStatusLabel(kind: .offline, text: "Выключено")
            }
        }
    }
}

/// The connectors that the user can add now (D-020, D-021, D-023). "This
/// Mac" exists once and is created automatically.
private struct ConnectorListView: View {
    @ObservedObject var connections: StorageConnections
    let choose: (StorageConnector) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Добавить подключение").font(.largeTitle.bold())
            Text("Выберите коннектор — тип хранилища.").foregroundStyle(.secondary)
        }
        ForEach(StorageConnector.allCases) { connector in
            row(connector)
        }
    }

    private func row(_ connector: StorageConnector) -> some View {
        let shape = RoundedRectangle(cornerRadius: 10, style: .continuous)
        let state = availability(connector)
        return HStack(alignment: .center, spacing: 12) {
            Image(systemName: connector.symbol)
                .font(.title2)
                .foregroundStyle(connector.tint)
                .frame(width: 44, height: 44)
                .background(connector.tint.opacity(0.15), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
            VStack(alignment: .leading, spacing: 3) {
                Text(connector.title).font(.headline)
                Text(connector.summary).font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                QDStatusLabel(kind: state.kind, text: state.text)
            }
            Spacer(minLength: 12)
            Button("Подключить") { choose(connector) }
                .disabled(connector == .thisMac)
                .help(state.text)
        }
        .padding(12)
        .background(connector.tint.opacity(0.05), in: shape)
        .overlay(shape.strokeBorder(connector.tint.opacity(0.35), lineWidth: 1))
    }

    private func availability(_ connector: StorageConnector) -> (kind: QDStatusKind, text: String) {
        switch connector {
        case .thisMac:
            let name = connections.entries.first { $0.adapter == connector.rawValue }?.name ?? connector.title
            return (.ready, "Уже подключён как «\(name)». Второе подключение не нужно.")
        case .networkDrive:
            return (.ready, "Доступен. Подключений может быть сколько угодно.")
        case .yandexDisk:
            return (.ready, "Доступен как источник: с Яндекс Диска на этот Mac. Подключений может быть сколько угодно.")
        case .ispmanager:
            return (.needsAction, "Первый этап: вход и список возможностей панели. Сценарии резервного копирования — на следующем этапе.")
        }
    }
}

/// Sign-in and account of one Yandex Disk connection (D-023). The token stays
/// in the Keychain; the card shows only the login and the disk space.
private struct YandexConnectionDetails: View {
    let entry: StorageEntry
    let connections: StorageConnections
    @StateObject private var yandex: YandexConnection

    init(entry: StorageEntry, connections: StorageConnections) {
        self.entry = entry
        self.connections = connections
        _yandex = StateObject(wrappedValue: YandexConnection(connectionID: entry.id, enabled: entry.enabled))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                GridRow {
                    Text("Аккаунт").foregroundStyle(.secondary)
                    Text(yandex.accountLabel ?? (yandex.connected ? "Вход выполнен" : "Вход не выполнен"))
                }
                if let quota = yandex.quota {
                    GridRow {
                        Text("Место").foregroundStyle(.secondary)
                        Text("Занято \(Self.bytes(quota.used_space)) из \(Self.bytes(quota.total_space))")
                    }
                }
            }
            HStack(spacing: 10) {
                Button(yandex.connected ? "Войти заново" : "Войти в Яндекс", systemImage: "person.badge.key") {
                    yandex.connect()
                }
                .disabled(!entry.enabled || yandex.busy || yandex.clientID.isEmpty)
                if yandex.connected {
                    Button("Выйти", systemImage: "rectangle.portrait.and.arrow.right") { yandex.disconnect() }
                        .disabled(yandex.busy)
                }
                if yandex.busy || yandex.metadataBusy { ProgressView().controlSize(.small) }
            }
            Text(statusText).font(.caption).foregroundStyle(.secondary)
            if let error = yandex.metadataError {
                Text("Яндекс Диск: " + error).font(.caption).foregroundStyle(.orange)
            }
            Text(entry.enabled
                ? "Папки Диска выбираются в редакторе сценария. Яндекс Диск — только источник: QADREN читает файлы и ничего не меняет на Диске."
                : "Подключение выключено: сценарии с ним не строят план и не переносят файлы.")
                .font(.caption).foregroundStyle(.secondary)
        }
        // Sign-in and account checks write the status to the Registry. The
        // card header and the dashboard read it from there.
        .onChange(of: yandex.status) { _ in connections.statusDidChange() }
    }

    private static func bytes(_ value: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: value), countStyle: .file)
    }

    private var statusText: String {
        switch yandex.status {
        case "Enter registered Yandex OAuth Client ID": "В этой сборке не задан Client ID приложения Яндекса."
        case "Registered connection", "Connected to Yandex Disk", "Connected; account verified": "Вход выполнен."
        case "Authorization required or incomplete", "Authorization incomplete; reconnect required",
             "No token in Keychain", "Credential reference mismatch; reconnect required":
            "Нужен вход в Яндекс."
        case "Waiting for Yandex authorization": "Ждём вход в браузере…"
        case "Authentication cancelled or callback unavailable": "Вход отменён."
        case "Yandex authorization requires renewal": "Яндекс просит войти заново."
        case "Different Yandex account; create a separate connection", "Yandex account mismatch; connection blocked":
            "Это другой аккаунт Яндекса. Для него создайте отдельное подключение."
        case "Disconnected locally (Yandex token not revoked)":
            "Выход выполнен на этом Mac. Файлы на Диске не изменены."
        default: yandex.status
        }
    }
}

/// The address and the login of one SMB share connection (D-021), with the
/// login check and the way to the edit screen. The password is never shown.
private struct SMBConnectionDetails: View {
    let entry: StorageEntry
    @ObservedObject var connections: StorageConnections
    let onEdit: () -> Void
    @State private var login: String?
    @State private var checking = false
    @State private var result: SMBCheckOutcome?
    @State private var problem: String?
    /// The share is mounted at another place; a click can take it over.
    @State private var elsewhere: String?

    static func status(_ status: String) -> (kind: QDStatusKind, text: String) {
        switch status {
        case "connected": (.ready, "Подключено")
        case "requires_auth": (.needsAction, "Нужен вход")
        case "offline": (.offline, "Нет связи")
        case "error": (.failed, "Ошибка")
        default: (.unknown, "Не проверено")
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                GridRow {
                    Text("Адрес").foregroundStyle(.secondary)
                    Text(entry.endpoint_url ?? "Не задан")
                        .font(.body.monospaced())
                        .textSelection(.enabled)
                        .lineLimit(1).truncationMode(.middle)
                }
                GridRow {
                    Text("Вход").foregroundStyle(.secondary)
                    Text(login.map { "Логин «\($0)», пароль сохранён в Keychain" } ?? "Логин и пароль не сохранены")
                        .foregroundStyle(login == nil ? .orange : .primary)
                }
            }
            HStack(spacing: 10) {
                Button("Проверить вход", systemImage: "checkmark.shield") { check() }
                    .disabled(checking || login == nil || !entry.enabled)
                Button("Изменить адрес и вход", systemImage: "key", action: onEdit)
                    .disabled(checking)
                if checking {
                    ProgressView().controlSize(.small)
                    Text("Подключаюсь к серверу…").font(.caption).foregroundStyle(.secondary)
                }
            }
            if let result {
                SettingsNotice(
                    text: result.message,
                    symbol: result.registryStatus == "connected" ? "checkmark.circle" : "exclamationmark.triangle"
                )
            }
            if let problem {
                SettingsNotice(text: problem, symbol: "exclamationmark.triangle") {
                    if let elsewhere, !SMBMounts.isTimeMachine(elsewhere) {
                        Button("Отключить там и подключить для QADREN") { takeOver(from: elsewhere) }
                            .disabled(checking)
                    }
                }
            }
        }
        .onAppear(perform: loadLogin)
    }

    private func loadLogin() {
        do {
            login = try SMBKeychain.read(entry.id)?.login
            problem = nil
        } catch {
            login = nil
            problem = "Не удалось прочитать логин и пароль из Keychain: " + error.localizedDescription
        }
    }

    private func check() {
        guard entry.endpoint_url != nil, login != nil else { return loadLogin() }
        run { await SMBMounts.ensureMounted(entry) }
    }

    private func takeOver(from path: String) {
        run { await SMBMounts.takeOver(entry, from: path) }
    }

    /// The check is the QADREN mount that Scenarios use (D-022).
    private func run(_ mount: @escaping () async -> SMBMounts.Result) {
        checking = true
        result = nil
        problem = nil
        elsewhere = nil
        Task {
            let mount = await mount()
            checking = false
            switch mount {
            case .mounted:
                result = .connected
                connections.recordSMBCheck(id: entry.id, outcome: .connected)
            case let .failed(outcome):
                result = outcome
                connections.recordSMBCheck(id: entry.id, outcome: outcome)
            case let .mountedElsewhere(path):
                elsewhere = path
                problem = mount.message
            case .noCredentials:
                problem = mount.message
            }
        }
    }
}

/// Adds an SMB share connection, or changes the address, the login, and the
/// password of an existing one. "Проверить и сохранить" signs in first; a
/// failed check offers to save without the check.
private struct SMBConnectionForm: View {
    @ObservedObject var connections: StorageConnections
    let editing: StorageEntry?
    let onDone: () -> Void
    let onCancel: () -> Void
    @State private var name = "Сетевой диск"
    @State private var address = ""
    @State private var login = ""
    @State private var password = ""
    @State private var checking = false
    @State private var outcome: SMBCheckOutcome?
    @State private var problem: String?
    @State private var mounted: [SMBShareChoice] = []
    @State private var pickNote: String?
    @StateObject private var browser = SMBServerBrowser()

    private var addressValid: Bool { SMBAddress.mountURL(address) != nil }
    private var complete: Bool {
        addressValid && !login.trimmingCharacters(in: .whitespaces).isEmpty
            && (editing != nil || !name.trimmingCharacters(in: .whitespaces).isEmpty)
    }
    private var credentials: SMBCredentials {
        SMBCredentials(login: login.trimmingCharacters(in: .whitespaces), password: password)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 12) {
                Image(systemName: StorageConnector.networkDrive.symbol)
                    .font(.title2)
                    .foregroundStyle(StorageConnector.networkDrive.tint)
                    .frame(width: 44, height: 44)
                    .background(StorageConnector.networkDrive.tint.opacity(0.15), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                VStack(alignment: .leading, spacing: 3) {
                    Text(editing == nil ? "Сетевой диск (SMB)" : "Адрес и вход «\(editing?.name ?? "")»")
                        .font(.largeTitle.bold())
                    Text("Общая папка на сервере по протоколу SMB.").foregroundStyle(.secondary)
                }
            }
            sharePicker
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                if editing == nil {
                    GridRow {
                        Text("Название").foregroundStyle(.secondary)
                        TextField("Название подключения", text: $name).textFieldStyle(.roundedBorder)
                    }
                }
                GridRow {
                    Text("Адрес").foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        TextField("smb://nas.local/Общая папка", text: $address)
                            .textFieldStyle(.roundedBorder)
                            .font(.body.monospaced())
                        Text(address.isEmpty || addressValid
                            ? "Сервер и общая папка, например smb://192.168.1.10/Документы."
                            : QADRENUserMessage.text(for: "invalid_smb_address"))
                            .font(.caption)
                            .foregroundStyle(address.isEmpty || addressValid ? Color.secondary : Color.orange)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                GridRow {
                    Text("Логин").foregroundStyle(.secondary)
                    TextField("Имя пользователя на сервере", text: $login)
                        .textFieldStyle(.roundedBorder)
                        .textContentType(.username)
                }
                GridRow {
                    Text("Пароль").foregroundStyle(.secondary)
                    SecureField("Пароль", text: $password)
                        .textFieldStyle(.roundedBorder)
                        .textContentType(.password)
                }
            }
            Label("QADREN запоминает логин и пароль в Keychain этого Mac и сам входит с ними на сервер. В базу QADREN и в журналы они не попадают.", systemImage: "key.fill")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let outcome, outcome.registryStatus != "connected" {
                SettingsNotice(text: outcome.message + " Можно исправить данные или сохранить подключение без проверки.", symbol: "exclamationmark.triangle")
            }
            if let problem {
                SettingsNotice(text: problem, symbol: "exclamationmark.triangle")
            }
            HStack {
                if checking {
                    ProgressView().controlSize(.small)
                    Text("Подключаюсь к серверу…").font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Отмена", action: onCancel)
                    .keyboardShortcut(.cancelAction)
                if outcome != nil && outcome?.registryStatus != "connected" {
                    Button("Сохранить без проверки") { save(outcome) }
                        .disabled(checking || !complete)
                }
                Button("Проверить и сохранить", action: checkAndSave)
                    .keyboardShortcut(.defaultAction)
                    .disabled(checking || !complete)
            }
        }
        .frame(maxWidth: 640, alignment: .leading)
        .onAppear {
            load()
            mounted = SMBShareChoice.mounted()
            browser.start()
        }
        .onDisappear { browser.stop() }
        .onReceive(NSWorkspace.shared.notificationCenter.publisher(for: NSWorkspace.didMountNotification)) { _ in
            mounted = SMBShareChoice.mounted()
        }
        .onReceive(NSWorkspace.shared.notificationCenter.publisher(for: NSWorkspace.didUnmountNotification)) { _ in
            mounted = SMBShareChoice.mounted()
        }
        .onChange(of: address) { _ in outcome = nil }
        .onChange(of: login) { _ in outcome = nil }
        .onChange(of: password) { _ in outcome = nil }
    }

    /// Mouse-only choice, as in Finder: shares that macOS has mounted, servers
    /// that Bonjour finds in the network, and the system folder panel.
    private var sharePicker: some View {
        let shape = RoundedRectangle(cornerRadius: 8, style: .continuous)
        return VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Выберите диск мышью").font(.headline)
                Spacer()
                Button("Выбрать папку…", systemImage: "folder") { pickFolder() }
                    .help("Папка внутри подключённой сетевой папки")
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    if !mounted.isEmpty {
                        pickerCaption("Подключены в macOS — щёлкните, чтобы выбрать")
                        ForEach(mounted) { choice in
                            pickerRow(
                                symbol: "externaldrive.connected.to.line.below",
                                title: choice.title, detail: choice.server,
                                selected: choice.address == address.trimmingCharacters(in: .whitespaces),
                                action: { apply(choice) }
                            )
                        }
                    }
                    pickerCaption(browser.servers.isEmpty
                        ? "Ищу серверы в сети…"
                        : "Серверы в сети — щёлкните, macOS спросит логин и общую папку")
                    ForEach(browser.servers, id: \.self) { server in
                        pickerRow(
                            symbol: "server.rack", title: server, detail: "Открыть в macOS",
                            selected: false, action: { SMBServerBrowser.open(server) }
                        )
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(height: 150)
            .padding(6)
            .background(.quaternary.opacity(0.5), in: shape)
            if let pickNote {
                Text(pickNote).font(.caption).foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Text("Или введите адрес вручную ниже.").font(.caption).foregroundStyle(.secondary)
        }
    }

    private func pickerCaption(_ text: String) -> some View {
        Text(text).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            .padding(.horizontal, 6).padding(.top, 6).padding(.bottom, 2)
    }

    private func pickerRow(
        symbol: String, title: String, detail: String, selected: Bool, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: symbol)
                    .foregroundStyle(StorageConnector.networkDrive.tint)
                    .frame(width: 20)
                Text(title).lineLimit(1)
                Spacer(minLength: 8)
                Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                if selected {
                    Image(systemName: "checkmark").foregroundStyle(StorageConnector.networkDrive.tint)
                }
            }
            .padding(.horizontal, 6).padding(.vertical, 5)
            .contentShape(Rectangle())
            .background(
                selected ? StorageConnector.networkDrive.tint.opacity(0.15) : .clear,
                in: RoundedRectangle(cornerRadius: 6, style: .continuous)
            )
        }
        .buttonStyle(.plain)
    }

    private func apply(_ choice: SMBShareChoice) {
        address = choice.address
        if let login = choice.login, self.login.isEmpty || editing == nil { self.login = login }
        if editing == nil, name == "Сетевой диск" || name.isEmpty { name = choice.title }
        pickNote = nil
    }

    private func pickFolder() {
        // The key window is the Settings window of this form.
        SMBServerBrowser.chooseFolder(attachedTo: NSApp.keyWindow) { choice in
            if let choice {
                apply(choice)
            } else {
                pickNote = "Эта папка не на сетевом диске SMB. Сначала подключите сервер: щёлкните его в списке серверов."
            }
        }
    }

    private func load() {
        guard let editing else { return }
        address = editing.endpoint_url ?? ""
        if let saved = try? SMBKeychain.read(editing.id) {
            login = saved.login
            password = saved.password
        }
    }

    private func checkAndSave() {
        checking = true
        problem = nil
        let address = address
        let credentials = credentials
        Task {
            let result = await SMBShareCheck.check(address: address, credentials: credentials)
            checking = false
            outcome = result
            if result.registryStatus == "connected" { save(result) }
        }
    }

    private func save(_ result: SMBCheckOutcome?) {
        do {
            if let editing {
                try connections.updateSMBShare(
                    id: editing.id, address: address, credentials: credentials, outcome: result
                )
            } else {
                try connections.addSMBShare(
                    name: name, address: address, credentials: credentials, outcome: result
                )
            }
            onDone()
        } catch {
            problem = error.localizedDescription
        }
    }
}

/// The panel URL and the login of one ISPmanager connection (D-027), the
/// sign-in check, and the capability list of the last check. The password is
/// never shown.
private struct ISPmanagerConnectionDetails: View {
    let entry: StorageEntry
    @ObservedObject var connections: StorageConnections
    let onEdit: () -> Void
    @State private var credentials: LoginCredentials?
    @State private var settings = ISPmanagerSettings.defaults
    @State private var checking = false
    @State private var report: ISPmanagerCheckReport?
    @State private var problem: String?
    @State private var confirmChangedKey = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                GridRow {
                    Text("Панель").foregroundStyle(.secondary)
                    Text(entry.endpoint_url ?? "Не задана")
                        .font(.body.monospaced())
                        .textSelection(.enabled)
                        .lineLimit(1).truncationMode(.middle)
                }
                GridRow {
                    Text("Вход").foregroundStyle(.secondary)
                    Text(credentials.map { "Логин «\($0.login)», пароль сохранён в Keychain" } ?? "Логин и пароль не сохранены")
                        .foregroundStyle(credentials == nil ? .orange : .primary)
                }
                GridRow {
                    Text("SSH").foregroundStyle(.secondary)
                    Text(settings.host_key_sha256.map { "Порт \(settings.ssh_port), ключ сервера подтверждён: \($0)" }
                        ?? "Порт \(settings.ssh_port), ключ сервера не подтверждён")
                        .font(.callout)
                        .textSelection(.enabled)
                        .lineLimit(1).truncationMode(.middle)
                }
            }
            HStack(spacing: 10) {
                Button("Проверить вход и возможности", systemImage: "checkmark.shield") { check() }
                    .disabled(checking || credentials == nil || !entry.enabled)
                Button("Изменить адрес и вход", systemImage: "key", action: onEdit)
                    .disabled(checking)
                if checking {
                    ProgressView().controlSize(.small)
                    Text("Читаю списки панели…").font(.caption).foregroundStyle(.secondary)
                }
            }
            if let report {
                SettingsNotice(
                    text: report.message,
                    symbol: report.status == "connected" ? "checkmark.circle" : "exclamationmark.triangle"
                )
                if let transport = report.file_transport {
                    SettingsNotice(
                        text: transport.message(port: settings.ssh_port),
                        symbol: transport.isSFTP ? "lock.shield" : "arrow.triangle.branch"
                    ) {
                        if transport.reason == "host_key_unconfirmed", let fingerprint = transport.fingerprint {
                            Button("Подтвердить ключ сервера") { confirm(fingerprint) }
                                .disabled(checking)
                        }
                        if transport.reason == "host_key_changed" {
                            Button("Подтвердить новый ключ…") { confirmChangedKey = true }
                                .disabled(checking)
                        }
                    }
                }
                if !report.categories.isEmpty {
                    ISPmanagerCapabilityList(categories: report.categories)
                }
            }
            if let problem {
                SettingsNotice(text: problem, symbol: "exclamationmark.triangle")
            }
        }
        .onAppear(perform: load)
        .confirmationDialog("Принять новый ключ SSH-сервера?", isPresented: $confirmChangedKey) {
            if let fingerprint = report?.file_transport?.fingerprint {
                Button("Принять новый ключ") { confirm(fingerprint) }
            }
        } message: {
            Text("Принимайте новый ключ, только если вы знаете, что ключ сервера сменили. Иначе это может быть подмена сервера, и пароль уйдёт чужому серверу.")
        }
    }

    private func load() {
        do {
            credentials = try ISPmanagerKeychain.read(entry.id)
            settings = try StorageRegistryBridge.ispmanagerSettings(id: entry.id)
            // The last check stays visible after a restart.
            if report == nil { report = settings.last_check }
            problem = nil
        } catch {
            credentials = nil
            problem = "Не удалось прочитать настройки подключения: " + error.localizedDescription
        }
    }

    private func check() {
        guard let url = entry.endpoint_url, let credentials else { return load() }
        checking = true
        report = nil
        let settings = settings
        Task {
            let result = await ISPmanagerCheck.run(url: url, credentials: credentials, settings: settings)
            checking = false
            report = result
            connections.recordISPmanagerCheck(id: entry.id, outcome: result)
        }
    }

    /// Stores the fingerprint that the user confirmed, then checks again so
    /// that SFTP signs in with the confirmed key.
    private func confirm(_ fingerprint: String) {
        do {
            try connections.confirmISPmanagerHostKey(id: entry.id, fingerprint: fingerprint)
            load()
            check()
        } catch {
            problem = "Не удалось сохранить ключ сервера: " + error.localizedDescription
        }
    }
}

/// The 18 resource categories of D-027 with the state that the check saw.
/// A readable list is not a verified export, and the text says so.
private struct ISPmanagerCapabilityList: View {
    let categories: [ISPmanagerCategoryReport]

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 8, style: .continuous)
        VStack(alignment: .leading, spacing: 6) {
            Text("Возможности аккаунта").font(.headline)
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 6) {
                ForEach(categories) { category in
                    let state = ISPmanagerCategory.state(category)
                    GridRow {
                        Text(ISPmanagerCategory.title(category.category))
                        QDStatusLabel(kind: state.kind, text: state.text)
                        // A failed row shows the error type itself, not only
                        // in the tooltip, so that the reason is visible.
                        Text(category.state == "verified_metadata_only"
                            ? category.functions.map(\.function).joined(separator: ", ")
                            : detail(category, separator: "; "))
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(2).truncationMode(.tail)
                            .textSelection(.enabled)
                            .help(detail(category, separator: "\n"))
                    }
                }
            }
            Text("Проверка только читает списки. Экспорт содержимого будет проверяться отдельно для каждой категории.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(10)
        .background(.quaternary.opacity(0.4), in: shape)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Возможности аккаунта ISPmanager")
    }

    /// Function results for the tooltip: no values from the panel.
    private func detail(_ category: ISPmanagerCategoryReport, separator: String) -> String {
        category.functions
            .map { "\($0.function): \($0.error ?? $0.result)" }
            .joined(separator: separator)
    }
}

/// Adds an ISPmanager panel connection, or changes the URL, the login, and
/// the password of an existing one. "Проверить и сохранить" signs in first;
/// a failed check offers to save without the check.
private struct ISPmanagerConnectionForm: View {
    @ObservedObject var connections: StorageConnections
    let editing: StorageEntry?
    let onDone: () -> Void
    let onCancel: () -> Void
    @State private var name = "ISPmanager"
    @State private var url = ""
    @State private var sshPort = "22"
    @State private var login = ""
    @State private var password = ""
    @State private var saved = ISPmanagerSettings.defaults
    @State private var checking = false
    @State private var outcome: ISPmanagerCheckReport?
    @State private var problem: String?

    private var urlValid: Bool { ISPmanagerAddress.isValid(url) }
    private var port: UInt16? { UInt16(sshPort.trimmingCharacters(in: .whitespaces)).flatMap { $0 > 0 ? $0 : nil } }
    /// The confirmed host key stays only for the same panel URL and port.
    private var settings: ISPmanagerSettings {
        let samePlace = editing?.endpoint_url == url.trimmingCharacters(in: .whitespaces)
            && saved.ssh_port == port
        return ISPmanagerSettings(ssh_port: port ?? 22, host_key_sha256: samePlace ? saved.host_key_sha256 : nil)
    }
    private var complete: Bool {
        urlValid && port != nil && !login.trimmingCharacters(in: .whitespaces).isEmpty && !password.isEmpty
            && (editing != nil || !name.trimmingCharacters(in: .whitespaces).isEmpty)
    }
    private var credentials: LoginCredentials {
        LoginCredentials(login: login.trimmingCharacters(in: .whitespaces), password: password)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 12) {
                Image(systemName: StorageConnector.ispmanager.symbol)
                    .font(.title2)
                    .foregroundStyle(StorageConnector.ispmanager.tint)
                    .frame(width: 44, height: 44)
                    .background(StorageConnector.ispmanager.tint.opacity(0.15), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                VStack(alignment: .leading, spacing: 3) {
                    Text(editing == nil ? "ISPmanager" : "Адрес и вход «\(editing?.name ?? "")»")
                        .font(.largeTitle.bold())
                    Text("Панель хостинга ISPmanager 6. Только чтение: QADREN ничего не меняет на сервере.")
                        .foregroundStyle(.secondary)
                }
            }
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                if editing == nil {
                    GridRow {
                        Text("Название").foregroundStyle(.secondary)
                        TextField("Название подключения", text: $name).textFieldStyle(.roundedBorder)
                    }
                }
                GridRow {
                    Text("Адрес").foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        TextField("https://panel.example.com:1500/ispmgr", text: $url)
                            .textFieldStyle(.roundedBorder)
                            .font(.body.monospaced())
                        Text(url.isEmpty || urlValid
                            ? "Адрес панели с портом и путём, например https://panel.example.com:1500/ispmgr."
                            : QADRENUserMessage.text(for: "invalid_panel_url"))
                            .font(.caption)
                            .foregroundStyle(url.isEmpty || urlValid ? Color.secondary : Color.orange)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                GridRow {
                    Text("Порт SSH").foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        TextField("22", text: $sshPort)
                            .textFieldStyle(.roundedBorder)
                            .font(.body.monospaced())
                            .frame(width: 90)
                        Text(port == nil
                            ? "Порт — число от 1 до 65535."
                            : "Если у аккаунта есть SSH, файлы читаются по SFTP с этим логином и паролем. Иначе — через API панели.")
                            .font(.caption)
                            .foregroundStyle(port == nil ? Color.orange : Color.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                GridRow {
                    Text("Логин").foregroundStyle(.secondary)
                    TextField("Пользователь панели", text: $login)
                        .textFieldStyle(.roundedBorder)
                        .textContentType(.username)
                }
                GridRow {
                    Text("Пароль").foregroundStyle(.secondary)
                    SecureField("Пароль", text: $password)
                        .textFieldStyle(.roundedBorder)
                        .textContentType(.password)
                }
            }
            Label("После сохранения нажмите «Проверить вход и возможности» на карточке: там один раз подтверждается ключ SSH-сервера.", systemImage: "lock.shield")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Label("QADREN запоминает логин и пароль в Keychain этого Mac и передаёт их панели только в теле HTTPS-запроса. В адрес, базу QADREN и журналы они не попадают. QADREN входит только с правами этого аккаунта.", systemImage: "key.fill")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let outcome, outcome.status != "connected" {
                SettingsNotice(text: outcome.message + " Можно исправить данные или сохранить подключение без проверки.", symbol: "exclamationmark.triangle")
            }
            if let problem {
                SettingsNotice(text: problem, symbol: "exclamationmark.triangle")
            }
            HStack {
                if checking {
                    ProgressView().controlSize(.small)
                    Text("Вхожу в панель и читаю списки…").font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Отмена", action: onCancel)
                    .keyboardShortcut(.cancelAction)
                if outcome != nil && outcome?.status != "connected" {
                    Button("Сохранить без проверки") { save(outcome) }
                        .disabled(checking || !complete)
                }
                Button("Проверить и сохранить", action: checkAndSave)
                    .keyboardShortcut(.defaultAction)
                    .disabled(checking || !complete)
            }
        }
        .frame(maxWidth: 640, alignment: .leading)
        .onAppear(perform: load)
        .onChange(of: url) { _ in outcome = nil }
        .onChange(of: sshPort) { _ in outcome = nil }
        .onChange(of: login) { _ in outcome = nil }
        .onChange(of: password) { _ in outcome = nil }
    }

    private func load() {
        guard let editing else { return }
        url = editing.endpoint_url ?? ""
        if let settings = try? StorageRegistryBridge.ispmanagerSettings(id: editing.id) {
            saved = settings
            sshPort = String(settings.ssh_port)
        }
        if let credentials = try? ISPmanagerKeychain.read(editing.id) {
            login = credentials.login
            password = credentials.password
        }
    }

    private func checkAndSave() {
        checking = true
        problem = nil
        let url = url.trimmingCharacters(in: .whitespaces)
        let credentials = credentials
        let settings = settings
        Task {
            let result = await ISPmanagerCheck.run(url: url, credentials: credentials, settings: settings)
            checking = false
            outcome = result
            if result.status == "connected" { save(result) }
        }
    }

    private func save(_ result: ISPmanagerCheckReport?) {
        let url = url.trimmingCharacters(in: .whitespaces)
        do {
            if let editing {
                try connections.updateISPmanager(
                    id: editing.id, url: url, sshPort: port ?? 22, credentials: credentials, outcome: result
                )
            } else {
                try connections.addISPmanager(
                    name: name, url: url, sshPort: port ?? 22, credentials: credentials, outcome: result
                )
            }
            onDone()
        } catch {
            problem = error.localizedDescription
        }
    }
}

private struct ScenariosSettingsView: View {
    @ObservedObject var scenarios: ScenarioSettings
    let connections: [StorageEntry]
    let createScenario: () -> Void
    let editScenario: (ScenarioEntry) -> Void
    let editSchedules: (ScenarioEntry) -> Void
    let previewScenario: (ScenarioEntry) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Сценарии обмена данными").font(.largeTitle.bold())
                    Text("Конфигурации сохраняются в Core; сами файлы этот экран не копирует.")
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button("Создать сценарий", action: createScenario)
                    .disabled(supportedConnections.isEmpty)
            }
            if let error = scenarios.error {
                SettingsNotice(text: error, symbol: "exclamationmark.triangle")
            }
            if scenarios.entries.isEmpty {
                SettingsEmptyState(
                    title: "Сценариев пока нет",
                    detail: "Выберите подключения, каталоги и направление обмена. После создания сценария можно просмотреть план, запустить его и настроить расписание.",
                    symbol: "arrow.triangle.branch"
                ) {
                        Button("Создать сценарий", action: createScenario)
                            .disabled(supportedConnections.isEmpty)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(scenarios.entries) { scenario in
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(scenario.name).font(.headline)
                                Text("Ревизия \(scenario.revision) · \(directionTitle(scenario.direction))")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                            Toggle("Включён", isOn: Binding(
                                get: { scenario.enabled },
                                set: { scenarios.setEnabled(scenario.id, enabled: $0) }
                            ))
                            .labelsHidden()
                            Button("Изменить") { editScenario(scenario) }
                                .disabled(!isSupportedScenario(scenario))
                                .help(isSupportedScenario(scenario)
                                      ? "Изменить сценарий"
                                      : "Сценарий использует подключение, которое QADREN не поддерживает")
                            Button("Расписания") { editSchedules(scenario) }
                                .disabled(!isSupportedScenario(scenario))
                                .help(!isSupportedScenario(scenario)
                                      ? "Сценарий использует подключение, которое QADREN не поддерживает"
                                      : isLocalScenario(scenario)
                                      ? "Настроить расписания сценария"
                                      : "Настроить расписания. Вход в Яндекс фоновая служба получает от QADREN: после перезапуска Mac запуск ждёт, пока вы откроете QADREN")
                            Button("Проверить план") { previewScenario(scenario) }
                                .disabled(!isSupportedScenario(scenario))
                                .help(isSupportedScenario(scenario)
                                      ? "Прочитать только метаданные обеих сторон и показать план без передачи файлов"
                                      : "Сценарий использует подключение, которое QADREN не поддерживает")
                        }
                        EndpointLine(
                            title: "Источник",
                            connection: connectionName(scenario.left_connection_id),
                            root: scenario.left_root
                        )
                        EndpointLine(
                            title: "Назначение",
                            connection: connectionName(scenario.right_connection_id),
                            root: scenario.right_root
                        )
                        Text(scenarioScopeSummary(scenario))
                            .font(.caption).foregroundStyle(.secondary)
                            .help(scenario.selected_paths.joined(separator: "\n"))
                        Text(scenarioStatus(scenario))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 6)
                }
                .listStyle(.inset)
            }
        }
        .padding(24)
    }

    private func connectionName(_ id: String) -> String {
        guard let entry = connections.first(where: { $0.id == id }) else {
            return "Недоступное подключение · \(id.prefix(8))"
        }
        return entry.name + " · " + adapterTitle(entry.adapter)
    }

    private var supportedConnections: [StorageEntry] {
        connections.filter { $0.adapter == "local_filesystem" || $0.adapter == "smb_share" }
    }

    /// A one-way Scenario from Yandex Disk to this Mac (D-023), or a local one.
    private func isSupportedScenario(_ scenario: ScenarioEntry) -> Bool {
        isLocalScenario(scenario) || YandexScenarioAccess.sourceConnectionID(scenario, entries: connections) != nil
    }

    /// Both endpoints are on this Mac: "This Mac" or a network drive that
    /// QADREN mounts (D-022).
    private func isLocalScenario(_ scenario: ScenarioEntry) -> Bool {
        let endpointIDs = [scenario.left_connection_id, scenario.right_connection_id]
        return endpointIDs.allSatisfy { id in
            ["local_filesystem", "smb_share"].contains(connections.first(where: { $0.id == id })?.adapter)
        }
    }

    private func scenarioScopeSummary(_ scenario: ScenarioEntry) -> String {
        let objects = scenario.selected_paths.isEmpty
            ? "Вся папка"
            : "Выбрано объектов: \(scenario.selected_paths.count)"
        let deletions = scenario.deletion_mode == ScenarioDeletionMode.simple.rawValue
            ? "удаления: простая синхронизация"
            : "удаления: параноик"
        let retries = scenario.retry_until_success ? "повтор до успеха" : "повтор 5 раз"
        return objects + " · " + deletions + " · " + retries
    }

    private func scenarioStatus(_ scenario: ScenarioEntry) -> String {
        if !isSupportedScenario(scenario) {
            return "Сценарий использует подключение, которое QADREN не поддерживает. Его запуск, изменение и расписания недоступны; запись сохранена локально."
        }
        if !isLocalScenario(scenario) {
            return "Яндекс Диск → Mac: план и перенос запускаются из этого окна или по расписанию. После перезапуска Mac запуск по расписанию ждёт, пока вы откроете QADREN. Удаления на Mac пока недоступны."
        }
        return scenario.enabled
            ? "Проверьте план по метаданным, затем подтвердите отдельный перенос или пакет обычных действий. Конфликты обрабатываются отдельно."
            : "Автоматические триггеры для этого сценария выключены."
    }

    private func adapterTitle(_ adapter: String) -> String {
        switch adapter {
        case "yandex_disk": "Яндекс Диск"
        case "smb_share": StorageConnector.networkDrive.title
        default: adapter
        }
    }

    private func directionTitle(_ direction: String) -> String {
        switch direction {
        case "left_to_right", "right_to_left": "Односторонняя"
        case "bidirectional": "Двусторонняя"
        default: direction
        }
    }
}

private struct ScenarioDryRunView: View {
    let scenario: ScenarioEntry
    let done: () -> Void
    @State private var plan: [ScenarioPlanItem] = []
    @State private var isLoading = false
    @State private var error: String?
    @State private var hasRequestedPreview = false
    @State private var pendingUpdate: ScenarioPlanItem?
    @State private var conflictTasks: [ScenarioConflictTask] = []
    @State private var leftSnapshot: [ScenarioSnapshotItem] = []
    @State private var rightSnapshot: [ScenarioSnapshotItem] = []
    @State private var pendingConflictReplace: ScenarioConflictTask?
    @State private var pendingConflictDelete: ScenarioConflictTask?
    @State private var activeBatch: ScenarioBatchProgress?
    @State private var batchTask: Task<Void, Never>?
    @State private var batchProgressTask: Task<Void, Never>?
    @State private var resumedCreateBatches: [ScenarioBatchProgress] = []
    @State private var workerOwnedCreateBatch = false
    /// True when the last plan read failed. An empty plan is then not
    /// evidence of "no changes".
    @State private var planUnavailable = false
    /// Progress of the Worker snapshots and plan (D-024).
    @State private var planProgress: String?
    /// When the shown plan was built.
    @State private var planBuiltAt: Date?
    /// The complete stored plan that "Синхронизировать" executes (D-025).
    @State private var storedPlan: WorkerPlanInfo?
    @State private var confirmSourceWins = false
    /// A stored plan with more rows than the screen loads at once (D-024).
    @State private var largePlan: WorkerPlanInfo?
    @State private var largeRows: [WorkerPlanRow] = []
    @State private var largeFilter = ""
    @State private var largeSearch = ""
    @State private var largeOffset = 0
    private static let largePlanRowLimit = 2_000
    private static let largePageSize = 200

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .firstTextBaseline) {
                Button("Назад", systemImage: "chevron.left", action: done)
                VStack(alignment: .leading, spacing: 3) {
                    Text("План сценария").font(.largeTitle.bold())
                    Text("Ревизия \(scenario.revision) · \(scenario.name)")
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button("Проверить план", systemImage: "arrow.triangle.2.circlepath") {
                    // A network drive is mounted before Core reads it (D-022).
                    isLoading = true
                    Task {
                        if let message = await SMBMounts.prepare(
                            connectionIDs: [scenario.left_connection_id, scenario.right_connection_id]
                        ) {
                            isLoading = false
                            hasRequestedPreview = true
                            plan = []
                            planUnavailable = true
                            error = message
                            return
                        }
                        loadPlan()
                    }
                }
                    .disabled(isLoading)
            }
            Text("Кнопка читает только имена, размеры и MD5 выбранных объектов на обеих сторонах. Она не открывает содержимое, не создаёт Job и не передаёт файлы. Большая папка на Яндекс Диске читается долго, иногда десятки минут.")
                .font(.callout).foregroundStyle(.secondary)
            if let planBuiltAt, !isLoading {
                Text("Показан план от \(planBuiltAt.formatted(date: .abbreviated, time: .shortened)). Чтобы учесть новые изменения, нажмите «Проверить план».")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if let error {
                SettingsNotice(text: error, symbol: "exclamationmark.triangle")
            }
            if let skippedNotice {
                SettingsNotice(text: skippedNotice, symbol: "link")
            }
            if isLoading {
                ProgressView(planProgress ?? "Чтение метаданных и построение плана…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let largePlan {
                largePlanSection(largePlan)
            } else if !hasRequestedPreview {
                SettingsEmptyState(
                    title: "План ещё не проверен",
                    detail: "Проверка выполняется только по вашему явному действию и не меняет файлы или очередь.",
                    symbol: "doc.text.magnifyingglass"
                ) {
                    Button("Проверить план", action: loadPlan)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if plan.isEmpty && planUnavailable {
                SettingsEmptyState(
                    title: "План не построен",
                    detail: "Метаданные не прочитаны, поэтому QADREN не знает, есть ли изменения. Устраните причину выше и нажмите «Проверить план».",
                    symbol: "exclamationmark.triangle"
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if plan.isEmpty {
                SettingsEmptyState(
                    title: "Изменения не найдены",
                    detail: "По доступным метаданным план не содержит действий. Это не является проверкой равенства содержимого файлов.",
                    symbol: "checkmark.circle"
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                let creates = plan.filter { $0.action.kind == "create" }
                let updates = plan.filter { $0.action.kind == "update" }
                let ruleDeletes = plan.filter { $0.action.kind == "delete" }
                // All Creates, Updates, and rule Deletes run through one
                // Scenario Run (D-025): no limit on the number of files, and
                // the write-root lock applies.
                if let storedPlan, !creates.isEmpty || !updates.isEmpty || !ruleDeletes.isEmpty {
                    HStack {
                        synchronizeButton(storedPlan)
                    }
                }
                if let activeBatch {
                    GroupBox("Выполнение пакета") {
                        VStack(alignment: .leading, spacing: 8) {
                            ProgressView(value: Double(activeBatch.processed), total: Double(activeBatch.total))
                            Text(workerOwnedCreateBatch
                                ? "Подтверждено \(activeBatch.processed) из \(activeBatch.total) · Local Worker выполняет пакет."
                                : "Обработано \(activeBatch.processed) из \(activeBatch.total) · \(activeBatch.currentPath)")
                                .font(.caption.monospaced())
                            if activeBatch.failures > 0 {
                                Text("Ошибок: \(activeBatch.failures). Остальные элементы продолжаются.")
                                    .font(.caption)
                                    .foregroundStyle(.orange)
                            }
                            Text(workerOwnedCreateBatch
                                ? activeBatch.cancellationRequested
                                ? "Отмена сохранена. QADREN завершит текущую операцию и не запустит следующий файл."
                                : "Закрытие окна не отменяет пакет. После перезапуска Worker продолжит только файлы без зафиксированного исхода."
                                : activeBatch.cancellationRequested
                                ? "Отмена запрошена. QADREN завершит текущую операцию и не запустит следующий файл."
                                : "Обычные переносы продолжаются при ошибке одного файла. Очередь конфликтов работает отдельно.")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            if workerOwnedCreateBatch {
                                Button("Остановить после текущего файла", role: .cancel) { cancelWorkerCreateBatch() }
                                    .disabled(activeBatch.cancellationRequested)
                            } else {
                                Button("Остановить после текущего файла", role: .cancel) { cancelActiveBatch() }
                                    .disabled(activeBatch.cancellationRequested)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityElement(children: .contain)
                        .accessibilityLabel("Выполнение пакета сценария")
                    }
                }
                if !resumedCreateBatches.isEmpty {
                    GroupBox("Выполнение пакета") {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Пакет выполняет фоновая служба QADREN. Закрытие окна и выход из приложения его не останавливают.")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            ForEach(resumedCreateBatches) { batch in
                                VStack(alignment: .leading, spacing: 4) {
                                    ProgressView(value: Double(batch.processed), total: Double(batch.total))
                                    Text("Подтверждено \(batch.processed) из \(batch.total)"
                                        + (batch.paused ? " · на паузе" : " · \(batch.currentPath)"))
                                        .font(.caption.monospaced())
                                        .lineLimit(1).truncationMode(.middle)
                                    if batch.retryWaiting > 0 {
                                        Text("Ждут автоматического повтора: \(batch.retryWaiting)"
                                            + (batch.nextRetry.map { ", следующий в \($0.formatted(date: .omitted, time: .shortened))" } ?? "")
                                            + ". Это временные ошибки до записи файла, например ошибка 500 у Яндекса.")
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    }
                                    if batch.failures > 0 {
                                        Text("Не выполнено: \(batch.failures). Причины: \(batch.failureReasons.prefix(4).joined(separator: ", ")).")
                                            .font(.caption)
                                            .foregroundStyle(.orange)
                                    }
                                    if batch.cancellationRequested {
                                        Text("Отмена сохранена. QADREN завершит текущую операцию и не запустит следующий файл.")
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    } else {
                                        HStack {
                                            Button(batch.paused ? "Продолжить" : "Пауза после текущего файла") {
                                                setResumedBatchPaused(batch, paused: !batch.paused)
                                            }
                                            Button("Остановить после текущего файла", role: .cancel) {
                                                cancelResumedWorkerCreateBatch(batch)
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityElement(children: .contain)
                        .accessibilityLabel("Восстановленное выполнение пакета сценария")
                    }
                }
                List(plan) { item in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text(item.relative_path).font(.body.monospaced())
                            Spacer()
                            Text(actionTitle(item.action)).font(.caption.weight(.semibold))
                        }
                        Text(scenario.direction == "bidirectional"
                            ? "Слева: \(sideTitle(item.left)) · Справа: \(sideTitle(item.right))"
                            : "Источник: \(sideTitle(item.left)) · Назначение: \(sideTitle(item.right))")
                            .font(.caption).foregroundStyle(.secondary)
                        if let reason = item.action.reason {
                            Text(reasonTitle(reason)).font(.caption).foregroundStyle(.orange)
                        }
                        if item.action.kind == "create" {
                            Button("Создать копию") { executeCreate(item) }
                                .buttonStyle(.bordered)
                                .disabled(activeBatch != nil || !resumedCreateBatches.isEmpty)
                        }
                        if item.action.kind == "update" {
                            Button("Заменить файл", role: .destructive) { pendingUpdate = item }
                                .buttonStyle(.bordered)
                                .disabled(activeBatch != nil || !resumedCreateBatches.isEmpty)
                        }
                        if item.action.kind == "conflict" || item.action.kind == "delete_requires_decision" {
                            Button("Добавить в очередь конфликтов") { queueConflict(item) }
                                .buttonStyle(.bordered)
                        }
                    }
                    .padding(.vertical, 4)
                }
                .listStyle(.inset)
            }
            if !conflictTasks.isEmpty {
                Divider()
                VStack(alignment: .leading, spacing: 10) {
                    Text("Альтернативная очередь конфликтов").font(.title2.bold())
                    Text("Эта очередь независима от обычных переносов. Выберите источник, затем решение. Замена из источника требует отдельного подтверждения.")
                        .font(.callout).foregroundStyle(.secondary)
                    ForEach(conflictTasks) { task in
                        ScenarioConflictTaskView(
                            scenario: scenario,
                            task: task,
                            saveDecision: saveConflictDecision,
                            executeReplace: { pendingConflictReplace = $0 },
                            executeDelete: { pendingConflictDelete = $0 },
                            refreshTask: refreshConflictTask
                        )
                    }
                }
            }
        }
        .padding(24)
        .onAppear {
            loadConflictTasks()
            resumePendingCreateBatches()
            // Show the last stored plan, or follow a plan in progress; a new
            // walk starts only from "Проверить план" (D-024).
            isLoading = true
            Task { await loadPlanSides(startNew: false) }
        }
        .onDisappear {
            batchProgressTask?.cancel()
            batchProgressTask = nil
        }
        .alert("Заменить существующий файл?", isPresented: Binding(
            get: { pendingUpdate != nil }, set: { if !$0 { pendingUpdate = nil } }
        )) {
            Button("Отмена", role: .cancel) { pendingUpdate = nil }
            Button("Заменить файл", role: .destructive) { executePendingUpdate() }
        } message: {
            Text("macOS может скачать исходный файл из File Provider на этот Mac. QADREN заменит только этот существующий файл назначения и сохранит Job с точной ревизией сценария.")
        }
        .alert("Выполнить решение конфликта?", isPresented: Binding(
            get: { pendingConflictReplace != nil }, set: { if !$0 { pendingConflictReplace = nil } }
        )) {
            Button("Отмена", role: .cancel) { pendingConflictReplace = nil }
            Button(
                pendingConflictReplace?.decision == "replace_if_newer" ? "Проверить и заменить" : "Заменить файл",
                role: .destructive
            ) { executePendingConflictReplace() }
        } message: {
            Text(
                pendingConflictReplace?.decision == "replace_if_newer"
                    ? "QADREN ещё раз проверит обе стороны. Он заменит файл только если источник новее. При изменении задача станет устаревшей, а замена не начнётся."
                    : "QADREN ещё раз проверит обе стороны по сохранённому снимку. При изменении задача станет устаревшей, а замена не начнётся."
            )
        }
        .alert("Удалить файл как в источнике?", isPresented: Binding(
            get: { pendingConflictDelete != nil }, set: { if !$0 { pendingConflictDelete = nil } }
        )) {
            Button("Отмена", role: .cancel) { pendingConflictDelete = nil }
            Button("Удалить файл", role: .destructive) { executePendingConflictDelete() }
        } message: {
            Text(
                "QADREN повторно проверит снимок. Если источник всё ещё не содержит файл, "
                    + "будет удалён только \(deleteConflictDestinationDescription). "
                    + "Другие файлы не изменятся."
            )
        }
    }

    private func loadPlan() {
        isLoading = true
        error = nil
        planUnavailable = false
        // A Yandex Disk side reads the network, so the plan loads in a task.
        Task { await loadPlanSides() }
    }

    /// Metadata of one Scenario side: a local scan, or the Yandex Disk API
    /// off the main thread (D-023).
    private static func snapshot(
        root: String, connectionID: String, selection: [String] = []
    ) async throws -> [ScenarioSnapshotItem] {
        let adapter = (try? StorageRegistryBridge.list())?.first { $0.id == connectionID }?.adapter
        guard adapter == "yandex_disk" else {
            return try StorageRegistryBridge.localScenarioSnapshot(root: root, connectionID: connectionID)
        }
        return try await Task.detached(priority: .userInitiated) {
            // Only the selected objects: a large Disk is not walked for them.
            try StorageRegistryBridge.yandexScenarioSnapshot(
                root: root, connectionID: connectionID, selection: selection
            )
        }.value
    }

    /// The Worker walks both sides, stores the snapshots, and builds the
    /// plan as a stream (D-024). A plan of at most 2,000 rows opens in the
    /// full screen with all row actions; a larger one opens in pages.
    private func loadPlanSides(startNew: Bool = true) async {
        var shown = true
        defer { isLoading = false; hasRequestedPreview = hasRequestedPreview || shown; planProgress = nil }
        largePlan = nil
        storedPlan = nil
        largeRows = []
        do {
            // A plan for a superseded revision must not offer actions.
            if let current = try StorageRegistryBridge.listScenarios().first(where: { $0.id == self.scenario.id }),
               current.revision != self.scenario.revision {
                plan = []
                planUnavailable = true
                error = "Сценарий изменён (ревизия \(current.revision)). Этот план построен для ревизии \(self.scenario.revision). Вернитесь к списку сценариев и откройте план заново."
                return
            }
            try StorageRegistryBridge.dispatchPendingScenarioCopyBaselines()
            let scenario = scenario
            let token = try YandexScenarioAccess.token(for: scenario)
            let planID: String
            if startNew {
                planID = try await Task.detached {
                    try WorkerIPC.startPlan(scenarioID: scenario.id, revision: scenario.revision, accessToken: token)
                }.value
            } else {
                let latest = try? await Task.detached {
                    try WorkerIPC.planStatus(scenarioID: scenario.id, revision: scenario.revision)
                }.value
                guard let latest = latest?.plan, latest.state != "failed" else {
                    shown = false
                    return
                }
                planID = latest.id
            }
            var status: WorkerPlanStatus
            repeat {
                try await Task.sleep(for: .seconds(1))
                status = try await Task.detached {
                    try WorkerIPC.planStatus(scenarioID: scenario.id, revision: scenario.revision, planID: planID)
                }.value
                planProgress = Self.planProgressText(status)
            } while ["waiting", "building"].contains(status.plan?.state ?? "")
            // A newer plan of the same revision replaced this one: follow it.
            if status.plan?.error == "superseded" {
                await loadPlanSides(startNew: false)
                return
            }
            guard let info = status.plan, info.id == planID, info.state == "complete" else {
                let reason = status.left?.last_error ?? status.right?.last_error ?? status.plan?.error ?? "plan_unavailable"
                throw StorageError.operation(QADRENUserMessage.text(for: reason))
            }
            planBuiltAt = info.finished_at.map { Date(timeIntervalSince1970: TimeInterval($0)) }
            storedPlan = info
            let total = info.counts?.values.reduce(0) { $0 + $1.count } ?? 0
            if total <= Self.largePlanRowLimit {
                var rows: [WorkerPlanRow] = []
                while rows.count < total {
                    let offset = rows.count
                    let page = try await Task.detached {
                        try WorkerIPC.planPage(planID: planID, action: nil, search: nil, offset: offset, limit: 1_000)
                    }.value
                    if page.isEmpty { break }
                    rows += page
                }
                plan = rows.filter { $0.action.kind != "no_action" || total <= Self.largePlanRowLimit }.map(\.item)
                leftSnapshot = rows.compactMap(\.left_file)
                rightSnapshot = rows.compactMap(\.right_file)
            } else {
                plan = []
                leftSnapshot = []
                rightSnapshot = []
                largeFilter = info.counts?["create"] != nil ? "create" : ""
                largeSearch = ""
                largeOffset = 0
                largePlan = info
                await loadLargePage()
            }
            loadConflictTasks()
        } catch {
            plan = []
            planUnavailable = true
            self.error = "Не удалось прочитать метаданные или построить план: " + error.localizedDescription
        }
    }

    private static func planProgressText(_ status: WorkerPlanStatus) -> String {
        func side(_ title: String, _ info: WorkerSnapshotInfo?) -> String {
            guard let info else { return "\(title): ожидание" }
            let state = info.state == "complete" ? "готово" : "читаю"
            let wait = info.last_error.map { " · пауза: \(QADRENUserMessage.text(for: $0))" } ?? ""
            return "\(title): \(state), файлов \(info.files), папок \(info.folders_done) из \(info.folders_done + info.folders_pending)\(wait)"
        }
        let building = status.plan?.state == "building" ? "\nСтрою план…" : ""
        return side("Источник", status.left) + "\n" + side("Назначение", status.right) + building
    }

    /// One page of a large plan with the current filter and search.
    private func loadLargePage() async {
        guard let largePlan else { return }
        let (id, filter, search, offset) = (largePlan.id, largeFilter, largeSearch, largeOffset)
        do {
            largeRows = try await Task.detached {
                try WorkerIPC.planPage(
                    planID: id, action: filter.isEmpty ? nil : filter,
                    search: search.isEmpty ? nil : search, offset: offset, limit: Self.largePageSize
                )
            }.value
        } catch {
            self.error = "Не удалось прочитать страницу плана: " + error.localizedDescription
        }
    }

    private static func bytes(_ value: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: value, countStyle: .file)
    }

    @ViewBuilder
    private func largePlanSection(_ info: WorkerPlanInfo) -> some View {
        let counts = info.counts ?? [:]
        let kinds: [(String, String)] = [
            ("create", "Создать"), ("update", "Заменить"), ("delete", "Удалить по правилу"),
            ("delete_requires_decision", "Удаление — нужно решение"), ("conflict", "Конфликт"),
            ("needs_verification", "Требует проверки"), ("no_action", "Без действия"),
        ]
        VStack(alignment: .leading, spacing: 12) {
            GroupBox("Большой план") {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(kinds.filter { counts[$0.0] != nil }, id: \.0) { kind, title in
                        let count = counts[kind]!
                        Text("\(title): \(count.count)" + (count.bytes > 0 ? " · \(Self.bytes(count.bytes))" : ""))
                            .font(.callout.monospacedDigit())
                    }
                    if (counts["conflict"]?.count ?? 0) + (counts["needs_verification"]?.count ?? 0)
                        + (counts["delete_requires_decision"]?.count ?? 0) > 0 {
                        Text("Конфликты, удаления с отдельным решением и объекты, которые требуют проверки, синхронизация не выполняет.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    if ["create", "update", "delete"].contains(where: { (counts[$0]?.count ?? 0) > 0 }) {
                        synchronizeButton(info)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            HStack {
                Picker("Действие", selection: $largeFilter) {
                    Text("Все").tag("")
                    ForEach(kinds.filter { counts[$0.0] != nil }, id: \.0) { kind, title in
                        Text(title).tag(kind)
                    }
                }
                .fixedSize()
                TextField("Поиск по пути", text: $largeSearch)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit { largeOffset = 0; Task { await loadLargePage() } }
                Button("Назад", systemImage: "chevron.left") {
                    largeOffset = max(0, largeOffset - Self.largePageSize); Task { await loadLargePage() }
                }
                .labelStyle(.iconOnly)
                .disabled(largeOffset == 0)
                Text("\(largeOffset + 1)–\(largeOffset + largeRows.count)")
                    .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                Button("Вперёд", systemImage: "chevron.right") {
                    largeOffset += Self.largePageSize; Task { await loadLargePage() }
                }
                .labelStyle(.iconOnly)
                .disabled(largeRows.count < Self.largePageSize)
            }
            .onChange(of: largeFilter) { _ in largeOffset = 0; Task { await loadLargePage() } }
            List(largeRows, id: \.relative_path) { row in
                HStack {
                    Text(row.relative_path).font(.callout.monospaced()).lineLimit(1).truncationMode(.middle)
                    Spacer()
                    Text(actionTitle(row.action)).font(.caption.weight(.semibold))
                }
            }
            .listStyle(.inset(alternatesRowBackgrounds: true))
        }
    }

    /// "Синхронизировать": one run of all Create, Update, and Delete rows of
    /// the stored plan (D-025). A one-way Update or Delete first needs the
    /// one-time confirmation of "the source always wins".
    private func synchronizeButton(_ info: WorkerPlanInfo) -> some View {
        Button("Синхронизировать") { synchronize(info) }
            .buttonStyle(.borderedProminent)
            .disabled(activeBatch != nil || !resumedCreateBatches.isEmpty)
            .confirmationDialog("Источник главный", isPresented: $confirmSourceWins) {
                Button("Подтвердить и синхронизировать") {
                    do {
                        _ = try StorageRegistryBridge.scenarioSourceWins(scenario: scenario, confirm: true)
                        executeLargeCreates(info)
                    } catch {
                        self.error = "Не удалось сохранить подтверждение: " + error.localizedDescription
                    }
                }
            } message: {
                Text("В этом сценарии источник всегда главный. QADREN заменяет файлы назначения версией из источника, также если файл изменили в назначении. При «Простой синхронизации» QADREN удаляет в назначении файлы, которые удалены в источнике.\n\nЗаменённая версия и удалённые файлы попадают в Корзину, если она есть на томе назначения. На сетевом диске Корзины обычно нет, тогда замена и удаление безвозвратны.\n\nПодтверждение действует, пока не изменятся папка назначения или выбранные объекты.")
            }
    }

    private func synchronize(_ info: WorkerPlanInfo) {
        let counts = info.counts ?? [:]
        let replacesOrDeletes = (counts["update"]?.count ?? 0) + (counts["delete"]?.count ?? 0) > 0
        if scenario.direction != "bidirectional", replacesOrDeletes,
           (try? StorageRegistryBridge.scenarioSourceWins(scenario: scenario, confirm: false)) != true {
            confirmSourceWins = true
            return
        }
        executeLargeCreates(info)
    }

    /// The Worker makes one batch of the stored plan and runs it.
    private func executeLargeCreates(_ info: WorkerPlanInfo) {
        let scenario = scenario
        Task {
            do {
                let token = try YandexScenarioAccess.token(for: scenario)
                _ = try await Task.detached {
                    try WorkerIPC.executePlanCreates(planID: info.id, accessToken: token)
                }.value
                resumePendingCreateBatches()
            } catch WorkerIPCError.rejected("run_active") {
                // D-025: show the active run instead of a second run.
                self.error = QADRENUserMessage.text(for: "run_active")
                resumePendingCreateBatches()
            } catch {
                self.error = "Фоновая служба не приняла пакет: " + error.localizedDescription
            }
        }
    }

    private func queueConflict(_ item: ScenarioPlanItem) {
        let kind = item.action.kind == "conflict" ? "content" : "deletion"
        do {
            _ = try StorageRegistryBridge.queueScenarioConflict(
                scenario: scenario, relativePath: item.relative_path, kind: kind,
                left: leftSnapshot.first { $0.relative_path.samePathBytes(item.relative_path) },
                right: rightSnapshot.first { $0.relative_path.samePathBytes(item.relative_path) }
            )
            loadConflictTasks()
        } catch {
            self.error = "Не удалось добавить конфликт в отдельную очередь: " + error.localizedDescription
        }
    }

    private func loadConflictTasks() {
        do {
            conflictTasks = try StorageRegistryBridge.listScenarioConflicts(scenario: scenario)
        } catch {
            self.error = "Не удалось прочитать очередь конфликтов: " + error.localizedDescription
        }
    }

    private func saveConflictDecision(_ task: ScenarioConflictTask, _ sourceIsLeft: Bool, _ decision: String) {
        do {
            try StorageRegistryBridge.selectScenarioConflict(
                id: task.id, sourceIsLeft: sourceIsLeft, decision: decision
            )
            loadConflictTasks()
        } catch {
            self.error = "Не удалось сохранить решение конфликта: " + error.localizedDescription
        }
    }

    private func refreshConflictTask(_ task: ScenarioConflictTask) {
        guard !leftSnapshot.isEmpty || !rightSnapshot.isEmpty else {
            error = "Сначала обновите план сценария. Новый снимок нужен для повторного решения конфликта."
            return
        }
        do {
            _ = try StorageRegistryBridge.queueScenarioConflict(
                scenario: scenario, relativePath: task.relative_path, kind: task.kind,
                left: leftSnapshot.first { $0.relative_path.samePathBytes(task.relative_path) },
                right: rightSnapshot.first { $0.relative_path.samePathBytes(task.relative_path) }
            )
            loadConflictTasks()
        } catch {
            self.error = "Не удалось обновить снимок конфликта: " + error.localizedDescription
        }
    }

    private func executePendingConflictReplace() {
        guard let task = pendingConflictReplace else { return }
        pendingConflictReplace = nil
        do {
            try StorageRegistryBridge.executeScenarioConflictReplace(id: task.id)
            loadConflictTasks()
        } catch {
            self.error = "Не удалось выполнить решение конфликта: " + error.localizedDescription
            loadConflictTasks()
        }
    }

    private func executePendingConflictDelete() {
        guard let task = pendingConflictDelete else { return }
        pendingConflictDelete = nil
        do {
            try StorageRegistryBridge.executeScenarioConflictDelete(id: task.id)
            loadConflictTasks()
        } catch {
            self.error = "Не удалось удалить файл по решению конфликта: " + error.localizedDescription
            loadConflictTasks()
        }
    }

    private var deleteConflictDestinationDescription: String {
        guard let task = pendingConflictDelete, let sourceIsLeft = task.source_is_left else {
            return "выбранный файл"
        }
        let root = sourceIsLeft ? scenario.right_root : scenario.left_root
        return root.hasSuffix("/") ? root + task.relative_path : root + "/" + task.relative_path
    }

    /// Describes a finished Worker-owned Create batch from its durable receipt.
    /// A cancelled child is not a failure: it never started.
    nonisolated static func createBatchOutcome(_ summary: ScenarioBatchSummary) -> String? {
        let unfinished = summary.total - summary.completed - summary.cancelled
        var parts: [String] = []
        if summary.cancelled > 0 {
            parts.append("Пакет остановлен по запросу: создано \(summary.completed) из \(summary.total). Остальные файлы (\(summary.cancelled)) не запускались.")
        }
        if unfinished > 0 {
            let shown = summary.unfinished_sample.joined(separator: ", ")
            let rest = unfinished > summary.unfinished_sample.count ? " и ещё \(unfinished - summary.unfinished_sample.count)" : ""
            let reasons = summary.reasonTexts.prefix(4)
            parts.append("Не выполнены файлы (\(unfinished)): \(shown)\(rest)."
                + (reasons.isEmpty ? "" : " Причины: \(reasons.joined(separator: ", ")).")
                + " Временные ошибки уже повторялись автоматически; файлы остаются в плане.")
        }
        return parts.isEmpty ? nil : parts.joined(separator: " ")
    }

    private func resumePendingCreateBatches() {
        batchProgressTask?.cancel()
        batchProgressTask = Task {
            while !Task.isCancelled {
                // D-024: counts from the database, never all children.
                let receipts = (try? StorageRegistryBridge.scenarioCreateBatchSummaries(scenario: scenario)) ?? []
                let finishedIDs = resumedCreateBatches.compactMap(\.receiptID)
                    .filter { id in !receipts.contains { $0.id == id } }
                resumedCreateBatches = receipts.map(ScenarioBatchProgress.init(summary:))
                if resumedCreateBatches.isEmpty {
                    batchProgressTask = nil
                    let outcomes = finishedIDs
                        .compactMap { try? StorageRegistryBridge.scenarioCreateBatchSummary(id: $0) }
                        .compactMap(Self.createBatchOutcome)
                    loadPlan()
                    // loadPlan() clears the previous notice. Show the batch
                    // outcome after it.
                    if !outcomes.isEmpty { error = outcomes.joined(separator: "\n") }
                    return
                }
                // A summary of a very large batch takes about 0.1 s (D-024 check).
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    private func cancelActiveBatch() {
        guard let activeBatch else { return }
        batchTask?.cancel()
        self.activeBatch = ScenarioBatchProgress(
            total: activeBatch.total, processed: activeBatch.processed, failures: activeBatch.failures,
            currentPath: activeBatch.currentPath, cancellationRequested: true
        )
    }

    private func cancelWorkerCreateBatch() {
        guard let receiptID = activeBatch?.receiptID else { return }
        do {
            try StorageRegistryBridge.requestScenarioCreateBatchCancellation(id: receiptID)
            guard let activeBatch else { return }
            self.activeBatch = ScenarioBatchProgress(
                total: activeBatch.total, processed: activeBatch.processed, failures: activeBatch.failures,
                currentPath: activeBatch.currentPath, cancellationRequested: true, receiptID: receiptID
            )
        } catch {
            self.error = "Не удалось остановить пакет: " + error.localizedDescription
        }
    }

    private func setResumedBatchPaused(_ batch: ScenarioBatchProgress, paused: Bool) {
        guard let receiptID = batch.receiptID else { return }
        do {
            try StorageRegistryBridge.setScenarioCreateBatchPaused(id: receiptID, paused: paused)
        } catch {
            self.error = "Не удалось изменить паузу пакета: " + error.localizedDescription
        }
    }

    private func cancelResumedWorkerCreateBatch(_ batch: ScenarioBatchProgress) {
        guard let receiptID = batch.receiptID else { return }
        do {
            try StorageRegistryBridge.requestScenarioCreateBatchCancellation(id: receiptID)
            resumedCreateBatches = resumedCreateBatches.map { current in
                guard current.receiptID == receiptID else { return current }
                return ScenarioBatchProgress(
                    total: current.total, processed: current.processed, failures: current.failures,
                    currentPath: current.currentPath, cancellationRequested: true, receiptID: receiptID
                )
            }
        } catch {
            self.error = "Не удалось остановить пакет: " + error.localizedDescription
        }
    }

    private func executeCreate(_ item: ScenarioPlanItem) {
        do {
            try StorageRegistryBridge.executeScenarioCreate(scenario: scenario, item: item)
            loadPlan()
        } catch {
            self.error = "Не удалось создать копию по сценарию: " + error.localizedDescription
        }
    }

    private func executePendingUpdate() {
        guard let item = pendingUpdate else { return }
        pendingUpdate = nil
        do {
            let planned = plannedSnapshots(for: item)
            try StorageRegistryBridge.executeScenarioUpdate(
                scenario: scenario, item: item,
                plannedSource: planned.source, plannedDestination: planned.destination
            )
            loadPlan()
        } catch {
            self.error = "Не удалось обновить файл по сценарию: " + error.localizedDescription
        }
    }

    /// Plan metadata of the source and destination file for one plan row.
    private func plannedSnapshots(
        for item: ScenarioPlanItem
    ) -> (source: ScenarioSnapshotItem?, destination: ScenarioSnapshotItem?) {
        let left = leftSnapshot.first { $0.relative_path.samePathBytes(item.relative_path) }
        let right = rightSnapshot.first { $0.relative_path.samePathBytes(item.relative_path) }
        return item.action.from_left == false ? (right, left) : (left, right)
    }

    private func sideTitle(_ value: String) -> String {
        switch value {
        case "unchanged": "без изменений"
        case "added": "добавлен"
        case "removed": "отсутствует"
        case "changed": "изменён"
        default: "требует проверки"
        }
    }

    /// Core reasons are stable English codes. Show known reasons in Russian
    /// and keep an unknown reason visible instead of hiding it.
    /// Truthful scope: count the links and special files that the scan skipped.
    private var skippedNotice: String? {
        let left = leftSnapshot.filter { $0.special == true }.count
        let right = rightSnapshot.filter { $0.special == true }.count
        guard left + right > 0 else { return nil }
        let sides = scenario.direction == "bidirectional"
            ? "слева \(left), справа \(right)"
            : "в источнике \(left), в назначении \(right)"
        return "Пропущено объектов, которые QADREN не синхронизирует (символические ссылки и специальные файлы): \(sides). Они не копируются, не изменяются и не удаляются."
    }

    private func reasonTitle(_ reason: String) -> String {
        switch reason {
        case "destination changed since the confirmed baseline":
            "Файл назначения изменился после последней подтверждённой синхронизации."
        case "both endpoints changed or lack a shared confirmed content identity":
            "Файл изменён с обеих сторон, или у сторон нет общей подтверждённой версии."
        case "metadata does not prove the confirmed object identity":
            "По метаданным нельзя доказать, что это тот же подтверждённый файл."
        case "source observation is incomplete":
            "Метаданные источника неполные."
        case "names differ only by letter case or Unicode form and can name the same file":
            "Имена отличаются только регистром букв или формой Unicode. На этом Mac они могут означать один и тот же файл, поэтому QADREN не копирует их автоматически."
        default:
            QADRENUserMessage.text(for: reason)
        }
    }

    private func actionTitle(_ action: ScenarioPlanAction) -> String {
        switch action.kind {
        case "create": return action.from_left == true ? "создать справа" : "создать слева"
        case "update": return action.from_left == true ? "обновить справа" : "обновить слева"
        case "delete_requires_decision": return "нужно отдельное решение"
        case "delete": return action.on_left == true ? "удалить слева по правилу" : "удалить справа по правилу"
        case "conflict": return "конфликт"
        case "needs_verification": return "нужна проверка"
        default: return "без действия"
        }
    }
}

private struct ScenarioBatchProgress: Identifiable {
    let total: Int
    let processed: Int
    let failures: Int
    let currentPath: String
    let cancellationRequested: Bool
    let receiptID: String?
    /// Files that wait for an automatic retry (D-024).
    var retryWaiting: Int = 0
    var nextRetry: Date?
    var paused = false
    /// Last failure codes of the finished failed files, most frequent first.
    var failureReasons: [String] = []

    init(
        total: Int,
        processed: Int,
        failures: Int,
        currentPath: String,
        cancellationRequested: Bool,
        receiptID: String? = nil
    ) {
        self.total = total
        self.processed = processed
        self.failures = failures
        self.currentPath = currentPath
        self.cancellationRequested = cancellationRequested
        self.receiptID = receiptID
    }

    /// Progress of one Worker-owned batch from its summary (D-024).
    init(summary: ScenarioBatchSummary) {
        self.init(
            total: summary.total,
            processed: summary.completed,
            failures: summary.failed + summary.unknown,
            currentPath: summary.current_path ?? "",
            cancellationRequested: summary.cancellation_requested,
            receiptID: summary.id
        )
        retryWaiting = summary.retry_waiting
        nextRetry = summary.next_retry_at.map { Date(timeIntervalSince1970: TimeInterval($0)) }
        paused = summary.paused
        failureReasons = summary.reasonTexts
    }

    var id: String { receiptID ?? currentPath }
}

private struct ScenarioConflictTaskView: View {
    let scenario: ScenarioEntry
    let task: ScenarioConflictTask
    let saveDecision: (ScenarioConflictTask, Bool, String) -> Void
    let executeReplace: (ScenarioConflictTask) -> Void
    let executeDelete: (ScenarioConflictTask) -> Void
    let refreshTask: (ScenarioConflictTask) -> Void
    @State private var sourceIsLeft: Bool?

    init(
        scenario: ScenarioEntry,
        task: ScenarioConflictTask,
        saveDecision: @escaping (ScenarioConflictTask, Bool, String) -> Void,
        executeReplace: @escaping (ScenarioConflictTask) -> Void,
        executeDelete: @escaping (ScenarioConflictTask) -> Void,
        refreshTask: @escaping (ScenarioConflictTask) -> Void
    ) {
        self.scenario = scenario
        self.task = task
        self.saveDecision = saveDecision
        self.executeReplace = executeReplace
        self.executeDelete = executeDelete
        self.refreshTask = refreshTask
        _sourceIsLeft = State(initialValue: task.source_is_left)
    }

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text(task.relative_path).font(.body.monospaced())
                    Spacer()
                    Text(task.kind == "content" ? "конфликт содержимого" : "конфликт удаления")
                        .font(.caption.weight(.semibold))
                }
                Picker("Источник", selection: $sourceIsLeft) {
                    Text("Выберите источник").tag(Optional<Bool>.none)
                    Text("Левая сторона: \(scenario.left_root)").tag(Optional(true))
                    Text("Правая сторона: \(scenario.right_root)").tag(Optional(false))
                }
                .pickerStyle(.menu)
                Text("Сначала явно укажите сторону-источник. Это обязательно и для двустороннего сценария.")
                    .font(.caption).foregroundStyle(.secondary)
                let isEditable = task.state == "pending" || task.state == "selected"
                if task.kind == "content" {
                    HStack {
                        Button("Заменить из источника", role: .destructive) {
                            save("replace_from_source")
                        }
                        Button("Заменить, если новее") { save("replace_if_newer") }
                        Button("Пропустить перенос") { save("skip_transfer") }
                    }
                    .disabled(sourceIsLeft == nil || !isEditable)
                } else {
                    HStack {
                        Button("Удалить как в источнике", role: .destructive) {
                            save("delete_as_source")
                        }
                        Button("Не удалять файл") { save("keep_file") }
                    }
                    .disabled(sourceIsLeft == nil || !isEditable)
                }
                if let decision = task.decision {
                    Text("Сохранённое решение: \(decisionTitle(decision))")
                        .font(.caption).foregroundStyle(.secondary)
                }
                if task.state == "selected", task.decision == "replace_from_source" || task.decision == "replace_if_newer" {
                    Button(task.decision == "replace_if_newer" ? "Выполнить проверку и замену" : "Выполнить замену из источника", role: .destructive) {
                        executeReplace(task)
                    }
                }
                if task.state == "selected", task.decision == "delete_as_source" {
                    Button("Подтвердить удаление файла", role: .destructive) {
                        executeDelete(task)
                    }
                }
                if task.state == "stale" {
                    Text("Снимок устарел. Обновите план и создайте новую задачу из текущих метаданных.")
                        .font(.caption).foregroundStyle(.orange)
                    Button("Обновить снимок конфликта") { refreshTask(task) }
                }
            }
        }
    }

    private func save(_ decision: String) {
        guard let sourceIsLeft else { return }
        saveDecision(task, sourceIsLeft, decision)
    }

    private func decisionTitle(_ decision: String) -> String {
        switch decision {
        case "replace_from_source": "заменить из источника"
        case "replace_if_newer": "заменить, если новее"
        case "skip_transfer": "пропустить перенос"
        case "delete_as_source": "удалить как в источнике"
        case "keep_file": "не удалять файл"
        default: decision
        }
    }
}

private struct ScenarioSchedulesView: View {
    let scenario: ScenarioEntry
    let done: () -> Void
    @StateObject private var schedules = ScenarioScheduleSettings()
    @State private var timezone = TimeZone.current.identifier
    @State private var everyHours = false
    @State private var intervalHours = 6
    @State private var daysMask: UInt8 = ScheduleWording.allDays
    @State private var time = Calendar.current.date(bySettingHour: 3, minute: 0, second: 0, of: Date()) ?? Date()
    @State private var confirmStop = false
    @State private var loginState = LoginHandoff.state

    private static let timezones = TimeZone.knownTimeZoneIdentifiers.sorted()

    /// A Yandex Disk or SMB endpoint needs credentials that only the app
    /// can give to the Worker.
    private var needsLoginHandoff: Bool {
        let entries = (try? StorageRegistryBridge.list()) ?? []
        return [scenario.left_connection_id, scenario.right_connection_id].contains { id in
            ["yandex_disk", "smb_share"].contains(entries.first { $0.id == id }?.adapter ?? "")
        }
    }

    var body: some View {
        // One scroll for the whole screen: no inner scroll areas.
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                HStack(alignment: .firstTextBaseline) {
                    Button("Назад", systemImage: "chevron.left", action: done)
                    VStack(alignment: .leading, spacing: 3) {
                        Text("Расписания").font(.largeTitle.bold())
                        Text(scenario.name + " · ревизия " + String(scenario.revision))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                }

                SettingsNotice(
                    text: "Запуски по расписанию выполняет фоновая служба QADREN. Окно приложения можно не открывать.",
                    symbol: "clock.badge.checkmark"
                )

                if needsLoginHandoff, !schedules.entries.isEmpty, loginState != .enabled {
                    SettingsNotice(
                        text: loginState == .requiresApproval
                        ? "Запуск при входе в систему ждёт разрешения в Системных настройках › Объекты входа. Пока его нет, после перезагрузки Mac запуски этого сценария ждут, пока вы откроете QADREN."
                        : "Этому сценарию нужны данные для входа. После перезагрузки Mac его запуски ждут, пока вы откроете QADREN. Включите «Запускать QADREN при входе в систему», чтобы они шли сами.",
                        symbol: "person.badge.key"
                    ) {
                        if loginState == .requiresApproval {
                            Button("Открыть настройки") { LoginHandoff.openLoginItemsSettings() }
                        } else {
                            Button("Включить") {
                                do {
                                    try LoginHandoff.setEnabled(true)
                                } catch {
                                    schedules.report(error.localizedDescription)
                                }
                                loginState = LoginHandoff.state
                            }
                        }
                    }
                }

                if let run = schedules.runState.active {
                    GroupBox("Текущий запуск") {
                        VStack(alignment: .leading, spacing: 10) {
                            HStack(alignment: .center, spacing: 12) {
                                Image(systemName: "arrow.triangle.2.circlepath")
                                    .foregroundStyle(QDTheme.cyan)
                                    .accessibilityHidden(true)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(runTitle(run))
                                    Text(runDetail(run))
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                                Spacer()
                                Button("Остановить", role: .cancel) { confirmStop = true }
                                    .disabled(schedules.isStopping)
                            }
                            Text("Пока идёт этот запуск, слоты расписаний пропускаются. Новый запуск начнётся со следующего слота после его окончания.")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        .padding(8)
                    }
                }
                GroupBox("Автоматизация") {
                    VStack(alignment: .leading, spacing: 10) {
                        Toggle("Приостановить автоматические триггеры", isOn: Binding(
                            get: { schedules.automation.locally_paused },
                            set: { schedules.setLocallyPaused(scenarioID: scenario.id, paused: $0) }
                        ))
                        Text("Приостановка сохраняется локально, очищает только ожидающий автоматический запуск и не меняет существующие Job. Ручные запросы остаются отдельным путём.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                }
                GroupBox("Новое расписание") {
                    VStack(alignment: .leading, spacing: 12) {
                        Picker("Вид", selection: $everyHours) {
                            Text("Каждый день / по дням недели").tag(false)
                            Text("Каждые N часов").tag(true)
                        }
                        .pickerStyle(.radioGroup)
                        if everyHours {
                            Stepper(value: $intervalHours, in: 1...24) {
                                Text("Каждые \(intervalHours) ч")
                            }
                        }
                        DatePicker(
                            everyHours ? "Начиная с" : "Время",
                            selection: $time,
                            displayedComponents: .hourAndMinute
                        )
                        LabeledContent(everyHours ? "Активные дни" : "Дни") {
                            dayToggles
                        }
                        Picker("Часовой пояс", selection: $timezone) {
                            ForEach(Self.timezones, id: \.self) { Text($0).tag($0) }
                        }
                        Text(draftTitle)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        HStack {
                            Spacer()
                            Button("Добавить расписание", action: create)
                                .disabled(daysMask == 0)
                        }
                    }
                    .padding(8)
                }

                if let error = schedules.error {
                    SettingsNotice(text: error, symbol: "exclamationmark.triangle")
                }

                if !schedules.upcoming.isEmpty {
                    GroupBox("Ближайшие запуски") {
                        VStack(alignment: .leading, spacing: 8) {
                            ForEach(schedules.upcoming.prefix(5)) { occurrence in
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(ScheduleWording.dateTitle(occurrence.due_at))
                                    if let entry = schedules.entries.first(where: { $0.id == occurrence.schedule_id }) {
                                        Text(ScheduleWording.title(entry))
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    }
                                }
                            }
                            Text("Если в момент слота идёт предыдущий запуск этого сценария, слот пропускается.")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }

                if schedules.entries.isEmpty {
                    SettingsEmptyState(
                        title: "Расписаний пока нет",
                        detail: "Добавьте одно или несколько расписаний: по дням недели или каждые N часов.",
                        symbol: "calendar"
                    )
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 24)
                } else {
                    GroupBox("Расписания") {
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(schedules.entries) { entry in
                                if entry.id != schedules.entries.first?.id { Divider() }
                                HStack(spacing: 12) {
                                    Image(systemName: entry.enabled ? "calendar.badge.clock" : "calendar.badge.exclamationmark")
                                        .foregroundStyle(entry.enabled ? QDTheme.cyan : .secondary)
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(ScheduleWording.title(entry))
                                        if entry.enabled, let next = entry.next_at {
                                            Text("Следующий запуск: " + ScheduleWording.dateTitle(next))
                                                .font(.caption)
                                                .foregroundStyle(.secondary)
                                        }
                                        if let last = entry.last_slot {
                                            if last.skipped_active_run {
                                                Label(
                                                    "Пропущено: идёт предыдущий запуск · "
                                                        + ScheduleWording.slotTitle(last.occurrence_key),
                                                    systemImage: "forward.end"
                                                )
                                                .font(.caption)
                                                .foregroundStyle(QDTheme.warning)
                                            } else {
                                                Text("Последний слот запустил синхронизацию: "
                                                    + ScheduleWording.slotTitle(last.occurrence_key))
                                                    .font(.caption)
                                                    .foregroundStyle(.secondary)
                                            }
                                        }
                                    }
                                    Spacer()
                                    Toggle("Включено", isOn: Binding(
                                        get: { entry.enabled },
                                        set: { schedules.setEnabled(scenarioID: scenario.id, scheduleID: entry.id, enabled: $0) }
                                    ))
                                    .labelsHidden()
                                    Button("Удалить", role: .destructive) {
                                        schedules.remove(scenarioID: scenario.id, scheduleID: entry.id)
                                    }
                                }
                                .padding(.vertical, 8)
                            }
                        }
                        .padding(.horizontal, 8)
                    }
                }
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .onAppear { schedules.refresh(scenarioID: scenario.id) }
        .task {
            // The Worker advances runs in the background; keep the state current.
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(5))
                schedules.refresh(scenarioID: scenario.id)
                loginState = LoginHandoff.state
            }
        }
        .confirmationDialog("Остановить текущий запуск?", isPresented: $confirmStop) {
            Button("Остановить запуск") { schedules.stopRun(scenarioID: scenario.id) }
        } message: {
            Text("Скопированные файлы останутся. Файл, который копируется сейчас, будет завершён. Остальные файлы этого запуска не скопируются; следующий слот расписания начнёт новый запуск.")
        }
    }

    private func runTitle(_ run: ScenarioRunState.Run) -> String {
        let trigger = run.trigger == "schedule" ? "Запуск по расписанию" : "Ручной запуск"
        let state = run.state == "planning" ? "снимки и план" : "копирование файлов"
        return trigger + " · " + state
    }

    private func runDetail(_ run: ScenarioRunState.Run) -> String {
        let started = Date(timeIntervalSince1970: TimeInterval(run.created_at))
            .formatted(date: .abbreviated, time: .shortened)
        guard let reason = run.waiting_reason else { return "Начат " + started }
        let waiting = switch reason {
        case "needs_sign_in": "Нужен вход: откройте QADREN"
        case "waiting_for_scenario":
            run.waiting_for_scenario.map { "ждёт сценарий «\($0)»" } ?? QADRENUserMessage.text(for: reason)
        default: QADRENUserMessage.text(for: reason)
        }
        return "Начат " + started + " · ждёт: " + waiting
    }

    /// Seven day buttons; "Каждый день" selects all of them.
    private var dayToggles: some View {
        HStack(spacing: 4) {
            ForEach(0..<7, id: \.self) { day in
                let bit = UInt8(1) << UInt8(day)
                Toggle(ScheduleWording.dayNames[day], isOn: Binding(
                    get: { daysMask & bit != 0 },
                    set: { daysMask = $0 ? daysMask | bit : daysMask & ~bit }
                ))
                .toggleStyle(.button)
            }
            Button("Каждый день") { daysMask = ScheduleWording.allDays }
                .disabled(daysMask == ScheduleWording.allDays)
        }
    }

    private var draftComponents: (hour: UInt8, minute: UInt8) {
        let components = Calendar.current.dateComponents([.hour, .minute], from: time)
        return (UInt8(components.hour ?? 0), UInt8(components.minute ?? 0))
    }

    private var draftTitle: String {
        guard daysMask != 0 else { return "Выберите хотя бы один день." }
        let (hour, minute) = draftComponents
        return ScheduleWording.title(
            intervalHours: everyHours ? UInt8(intervalHours) : nil,
            daysMask: daysMask, hour: hour, minute: minute, timezone: timezone
        )
    }

    private func create() {
        let (hour, minute) = draftComponents
        schedules.create(
            scenarioID: scenario.id,
            timezone: timezone,
            intervalHours: everyHours ? UInt8(intervalHours) : 0,
            daysMask: daysMask,
            hour: hour,
            minute: minute
        )
    }
}

/// Plain Russian wording of schedules, for example "Пн–Пт в 03:00
/// (Europe/Moscow)" or "Каждые 6 ч с 00:00 (UTC)".
enum ScheduleWording {
    static let allDays: UInt8 = 0x7f
    static let dayNames = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"]

    static func title(_ entry: ScenarioScheduleEntry) -> String {
        title(
            intervalHours: entry.interval_hours, daysMask: entry.days_mask,
            hour: entry.hour, minute: entry.minute, timezone: entry.timezone
        )
    }

    static func title(
        intervalHours: UInt8?, daysMask: UInt8, hour: UInt8, minute: UInt8, timezone: String
    ) -> String {
        rule(intervalHours: intervalHours, daysMask: daysMask, hour: hour, minute: minute)
            + " (" + timezone + ")"
    }

    /// The rule without the time zone, for a short line: "Пн–Пт в 03:00".
    static func rule(_ entry: ScenarioScheduleEntry) -> String {
        rule(intervalHours: entry.interval_hours, daysMask: entry.days_mask, hour: entry.hour, minute: entry.minute)
    }

    static func rule(intervalHours: UInt8?, daysMask: UInt8, hour: UInt8, minute: UInt8) -> String {
        let time = String(format: "%02d:%02d", hour, minute)
        if let intervalHours {
            return "Каждые \(intervalHours) ч с " + time
                + (daysMask == allDays ? "" : ", " + days(daysMask))
        }
        return (daysMask == allDays ? "Каждый день" : days(daysMask)) + " в " + time
    }

    /// Selected days; a run of three or more days is a range: "Пн–Пт, Вс".
    static func days(_ mask: UInt8) -> String {
        var parts: [String] = []
        var day = 0
        while day < 7 {
            guard mask & (1 << UInt8(day)) != 0 else { day += 1; continue }
            var end = day
            while end + 1 < 7, mask & (1 << UInt8(end + 1)) != 0 { end += 1 }
            if end - day >= 2 {
                parts.append(dayNames[day] + "–" + dayNames[end])
            } else {
                parts.append(contentsOf: dayNames[day...end])
            }
            day = end + 1
        }
        return parts.joined(separator: ", ")
    }

    /// An RFC 3339 instant in the local time of this Mac.
    static func dateTitle(_ value: String) -> String {
        guard let date = ISO8601DateFormatter().date(from: value) else { return value }
        return date.formatted(date: .abbreviated, time: .shortened)
    }

    /// A slot key: "2026-10-25 02:30@Europe/Berlin" or "every-6h@2026-10-25T10:00Z".
    static func slotTitle(_ key: String) -> String {
        guard let at = key.firstIndex(of: "@") else { return key }
        if key.hasPrefix("every-") {
            let instant = String(key[key.index(after: at)...]).replacingOccurrences(of: "Z", with: ":00Z")
            return dateTitle(instant)
        }
        return String(key[..<at])
    }
}

private struct ScenarioEditorView: View {
    let connections: [StorageEntry]
    let scenario: ScenarioEntry?
    let saveScenario: (String, String, String, String, String, String, ScenarioScopeDraft) -> String?
    let removeScenario: (String) -> String?
    let cancel: () -> Void
    @State private var confirmRemoval = false
    @State private var name = ""
    @State private var leftID = ""
    /// An empty root means that the user has not chosen a folder yet.
    @State private var leftRoot = ""
    @State private var rightID = ""
    @State private var rightRoot = ""
    @State private var direction = "left_to_right"
    @State private var scope = ScenarioScopeDraft()
    @State private var confirmSimpleDeletions = false
    @State private var error: String?
    /// Scenarios whose roots overlap these roots (D-025 section 11).
    @State private var overlaps: [String] = []

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                HStack(alignment: .firstTextBaseline) {
                    Button("Назад", systemImage: "chevron.left", action: cancel)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(scenario == nil ? "Новый сценарий" : "Изменить сценарий").font(.largeTitle.bold())
                        Text("Сценарий хранит обе папки, выбранные объекты, направление и правило удалений.")
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button(scenario == nil ? "Создать сценарий" : "Сохранить изменения", action: save)
                        .keyboardShortcut(.defaultAction)
                        .disabled(!canCreate)
                }
                GroupBox("Основные параметры") {
                    VStack(alignment: .leading, spacing: 14) {
                        TextField("Например, «Документы → Архив»", text: $name)
                            .textFieldStyle(.roundedBorder)
                            .accessibilityLabel("Название сценария")
                        ChoiceCards(
                            options: [
                                .init(value: "left_to_right", title: "В одну сторону",
                                      detail: "Из источника в назначение.", symbol: "arrow.right"),
                            ] + (sourceIsYandex ? [] : [
                                .init(value: "bidirectional", title: "В обе стороны",
                                      detail: "Конфликты — только с вашим решением.", symbol: "arrow.left.arrow.right"),
                            ]),
                            selection: direction,
                            choose: { direction = $0 }
                        )
                        if sourceIsYandex {
                            Text("С Яндекс Диском пока доступен только режим «В одну сторону»: с Диска на этот Mac или на сетевой диск.")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
                HStack(alignment: .top, spacing: 16) {
                    ScenarioFolderPanel(
                        role: direction == "bidirectional" ? "Левая папка" : "Источник",
                        symbol: "arrow.right.circle",
                        connections: connections,
                        connectionID: $leftID,
                        root: $leftRoot,
                        selection: $scope.selectedPaths,
                        showsSelection: true,
                        excludesYandex: direction == "bidirectional"
                    )
                    ScenarioFolderPanel(
                        role: direction == "bidirectional" ? "Правая папка" : "Назначение",
                        symbol: "arrow.left.circle",
                        connections: connections,
                        connectionID: $rightID,
                        root: $rightRoot,
                        selection: .constant([]),
                        showsSelection: false,
                        excludesYandex: true
                    )
                }
                if !overlaps.isEmpty {
                    SettingsNotice(
                        text: "Папки пересекаются со сценарием "
                            + overlaps.map { "«\($0)»" }.joined(separator: ", ")
                            + ". Их запуски не выполняются одновременно: запуск, который начался позже, ждёт окончания другого.",
                        symbol: "square.on.square.intersection.dashed"
                    )
                }
                ScenarioDeletionModeBox(
                    mode: scope.deletionMode,
                    bidirectional: direction == "bidirectional",
                    networkDrive: [leftID, rightID].contains { id in
                        connections.first { $0.id == id }?.adapter == "smb_share"
                    },
                    choose: { mode in
                        if mode == .simple && scope.deletionMode != .simple {
                            confirmSimpleDeletions = true
                        } else {
                            scope.deletionMode = mode
                        }
                    }
                )
                ScenarioRetryBox(retryUntilSuccess: $scope.retryUntilSuccess)
                Text("Создание сохраняет только конфигурацию. Передача файлов начинается позднее, только из проверенного плана и после отдельного подтверждения.")
                    .font(.caption).foregroundStyle(.secondary)
                if let error {
                    SettingsNotice(text: error, symbol: "exclamationmark.triangle")
                }
                if scenario != nil {
                    Button("Удалить сценарий", systemImage: "trash", role: .destructive) { confirmRemoval = true }
                }
            }
            .padding(24)
        }
        .confirmationDialog("Удалить сценарий «\(scenario?.name ?? name)»?", isPresented: $confirmRemoval) {
            Button("Удалить сценарий", role: .destructive) {
                if let scenario { error = removeScenario(scenario.id) }
            }
        } message: {
            Text("QADREN удалит с этого Mac сценарий, его расписания, историю запусков и планы. Файлы в источнике и назначении не изменятся.")
        }
        .task(id: [leftID, leftRoot, rightID, rightRoot, direction]) {
            guard !leftRoot.isEmpty, !rightRoot.isEmpty else {
                overlaps = []
                return
            }
            overlaps = (try? StorageRegistryBridge.scenarioRootOverlaps(
                scenarioID: scenario?.id, leftID: leftID, leftRoot: leftRoot,
                rightID: rightID, rightRoot: rightRoot, direction: direction
            )) ?? []
        }
        .onAppear {
            if let scenario {
                name = scenario.name
                leftID = scenario.left_connection_id
                leftRoot = scenario.left_root
                rightID = scenario.right_connection_id
                rightRoot = scenario.right_root
                direction = scenario.direction
                scope = ScenarioScopeDraft(
                    selectedPaths: Set(scenario.selected_paths),
                    deletionMode: ScenarioDeletionMode(rawValue: scenario.deletion_mode) ?? .paranoid,
                    retryUntilSuccess: scenario.retry_until_success
                )
            } else if let first = connections.first(where: { $0.adapter != "yandex_disk" }) {
                leftID = first.id
                rightID = first.id
            }
        }
        .onChange(of: leftID) { _ in
            // A Yandex Disk source supports one way only (D-023).
            if sourceIsYandex { direction = "left_to_right" }
        }
        .alert("Включить простую синхронизацию?", isPresented: $confirmSimpleDeletions) {
            Button("Отмена", role: .cancel) {}
            Button("Включить", role: .destructive) { scope.deletionMode = .simple }
        } message: {
            Text("Файлы, удалённые в источнике, будут удаляться и в назначении без вопроса — если они уже синхронизированы и не менялись. Удаление идёт в Корзину, без Корзины — навсегда. Больше 20 удалений — только с подтверждением.")
        }
    }

    private var sourceIsYandex: Bool {
        connections.first { $0.id == leftID }?.adapter == "yandex_disk"
    }

    private var canCreate: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !leftID.isEmpty && !rightID.isEmpty
            && !leftRoot.isEmpty && !rightRoot.isEmpty
            && (leftID != rightID || leftRoot != rightRoot)
    }
    private func save() {
        error = saveScenario(name, leftID, leftRoot, rightID, rightRoot, direction, scope)
    }
}

/// The deletion rule of a Scenario revision (D-019).
private struct ScenarioDeletionModeBox: View {
    let mode: ScenarioDeletionMode
    let bidirectional: Bool
    let networkDrive: Bool
    let choose: (ScenarioDeletionMode) -> Void

    var body: some View {
        GroupBox("Удаления") {
            VStack(alignment: .leading, spacing: 10) {
                ChoiceCards(
                    options: [
                        .init(value: ScenarioDeletionMode.paranoid, title: "Параноик",
                              detail: "Каждое удаление — с вашим подтверждением.", symbol: "hand.raised"),
                        .init(value: ScenarioDeletionMode.simple, title: "Простая синхронизация",
                              detail: bidirectional
                                  ? "Удалено в одной папке — удаляется и в другой."
                                  : "Удалено в источнике — удаляется в назначении.",
                              symbol: "trash"),
                    ],
                    selection: mode,
                    choose: choose
                )
                if mode == .simple {
                    Label("В Корзину; без Корзины — навсегда. Больше 20 удалений — только с подтверждением.", systemImage: "info.circle")
                        .font(.caption).foregroundStyle(.secondary)
                    if networkDrive {
                        Label("На сетевом диске Корзины обычно нет: удалённые там файлы не восстановить из Корзины.", systemImage: "exclamationmark.triangle")
                            .font(.caption).foregroundStyle(.orange)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// The D-024 retry mode of a Scenario. It is outside the revision.
private struct ScenarioRetryBox: View {
    @Binding var retryUntilSuccess: Bool

    var body: some View {
        GroupBox("Передача файлов") {
            VStack(alignment: .leading, spacing: 8) {
                Toggle("Повторять недоступные файлы до успеха", isOn: $retryUntilSuccess)
                Text(retryUntilSuccess
                    ? "Файл, который не скачался из-за временной ошибки (например, ошибка 500 у Яндекса), фоновая служба повторяет через 1 мин, 5 мин, 30 мин, 2 ч и 12 ч, а затем раз в сутки — пока он не скачается или вы не остановите пакет. Файл при неудачной попытке не записывается."
                    : "Файл, который не скачался из-за временной ошибки, фоновая служба повторяет через 1 мин, 5 мин, 30 мин, 2 ч и 12 ч. После этого он отмечается как не выполненный и остаётся в плане.")
                    .font(.caption).foregroundStyle(.secondary)
                Text("Эта настройка не создаёт новую ревизию: идущий пакет продолжается и применяет новое значение при следующей ошибке.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// Inline, metadata-only browser of one Scenario endpoint. The user moves
/// through folders with breadcrumbs, makes the visible folder the root, and,
/// in the source panel, selects objects inside the root with checkboxes.
private struct ScenarioFolderPanel: View {
    let role: String
    let symbol: String
    let connections: [StorageEntry]
    @Binding var connectionID: String
    @Binding var root: String
    @Binding var selection: Set<String>
    let showsSelection: Bool
    /// Yandex Disk can be only the source of a one-way Scenario (D-023).
    var excludesYandex = false
    @StateObject private var browser = ScenarioFolderBrowser()
    @State private var showsPathField = false
    @State private var pathDraft = ""
    @State private var shownConnectionID = ""
    @State private var showedInitialRoot = false
    /// One undo step for the root and its selection.
    @State private var previousRoot: (root: String, selection: Set<String>)?
    @State private var mounting = false
    @State private var mountResult: SMBMounts.Result?

    private var selectedConnection: StorageEntry? {
        connections.first { $0.id == connectionID }
    }
    private var isNetworkDrive: Bool { selectedConnection?.adapter == "smb_share" }
    private var isYandex: Bool { selectedConnection?.adapter == "yandex_disk" }
    private var pickable: [StorageEntry] {
        excludesYandex ? connections.filter { $0.adapter != "yandex_disk" || $0.id == connectionID } : connections
    }

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Label(role, systemImage: symbol).font(.headline).lineLimit(1)
                    Spacer(minLength: 8)
                    Picker("Подключение", selection: $connectionID) {
                        ForEach(pickable) { entry in
                            Label(entry.name, systemImage: StorageConnector(adapter: entry.adapter)?.symbol ?? "externaldrive")
                                .tag(entry.id)
                        }
                    }
                    .pickerStyle(.menu)
                    .labelsHidden()
                    .fixedSize()
                    .help("Подключение")
                    .onChange(of: connectionID) { newID in
                        // The editor fills the connection of an existing
                        // Scenario after this panel appears. Only a change
                        // from one chosen connection to another resets the
                        // root, which belongs to the old connection.
                        if !shownConnectionID.isEmpty && shownConnectionID != newID {
                            changeRoot(to: "")
                        }
                        shownConnectionID = newID
                        showConnection()
                    }
                }
                breadcrumbs
                if showsPathField {
                    HStack {
                        TextField("Путь к папке", text: $pathDraft, prompt: Text("/Users/имя/Documents"))
                            .textFieldStyle(.roundedBorder)
                            .font(.callout.monospaced())
                            .accessibilityLabel("Путь к папке")
                            .onSubmit(openDraftPath)
                        Button("Открыть путь", action: openDraftPath)
                            .disabled(!pathDraft.hasPrefix("/") && !pathDraft.hasPrefix("~"))
                    }
                }
                Divider()
                if isNetworkDrive && mounting {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text("Подключаю сетевой диск…").foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, minHeight: 280)
                } else if isNetworkDrive, let mountResult, let message = mountResult.message {
                    SettingsNotice(text: message, symbol: "exclamationmark.triangle") {
                        VStack(alignment: .trailing, spacing: 6) {
                            if case let .mountedElsewhere(path) = mountResult, let entry = selectedConnection,
                               !SMBMounts.isTimeMachine(path) {
                                Button("Отключить там и подключить для QADREN") {
                                    mounting = true
                                    Task {
                                        let result = await SMBMounts.takeOver(entry, from: path)
                                        mounting = false
                                        showMounted(result)
                                    }
                                }
                            }
                            Button("Повторить") { showConnection() }
                        }
                    }
                    .frame(height: 280, alignment: .top)
                } else if let error = browser.error {
                    SettingsNotice(text: error, symbol: "exclamationmark.triangle")
                        .frame(height: 280, alignment: .top)
                } else {
                    List(browser.entries) { entry in
                        row(entry)
                    }
                    .listStyle(.inset(alternatesRowBackgrounds: true))
                    .frame(height: 280)
                    .overlay {
                        if browser.entries.isEmpty {
                            Text("Папка пуста").foregroundStyle(.secondary)
                        }
                    }
                }
                Divider()
                rootLine
                if showsSelection {
                    selectionSummary
                }
                if selectedConnection?.enabled == false {
                    Text("Подключение выключено. Сценарий можно сохранить, но проверка плана и перенос станут доступны только после включения подключения.")
                        .font(.caption).foregroundStyle(.orange)
                }
                if isNetworkDrive {
                    Text("Сетевой диск: на нём обычно нет Корзины, поэтому удаления там окончательные.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                if isYandex {
                    Text("Яндекс Диск — источник: файлы скачиваются отсюда, на Диске ничего не меняется. Удаления на Mac по этому сценарию пока не выполняются.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onAppear(perform: showConnection)
        .onChange(of: root) { newRoot in
            // The root of an existing Scenario arrives after onAppear.
            if !showedInitialRoot, !newRoot.isEmpty, browser.path != newRoot, !isNetworkDrive {
                browser.open(newRoot)
            }
            showedInitialRoot = true
        }
    }

    private var breadcrumbs: some View {
        HStack(spacing: 6) {
            Button("Вверх", systemImage: "chevron.up") { browser.up() }
                .labelStyle(.iconOnly)
                .disabled(!browser.canGoUp)
                .help("На уровень выше")
            Button("Домой", systemImage: "house") {
                browser.open(isNetworkDrive || isYandex ? browser.base : NSHomeDirectory())
            }
                .labelStyle(.iconOnly)
                .help(isYandex ? "Корень Яндекс Диска" : isNetworkDrive ? "Общая папка сетевого диска" : "Домашняя папка")
            if !isNetworkDrive && !isYandex {
                Button("Перейти к папке", systemImage: "text.cursor") {
                    pathDraft = browser.path
                    showsPathField.toggle()
                }
                .labelStyle(.iconOnly)
                .help("Ввести путь к папке")
            }
            // A long path folds its first crumbs into a "…" menu, so the
            // visible folder always stays visible.
            ViewThatFits(in: .horizontal) {
                crumbRow(hiding: 0)
                crumbRow(hiding: max(0, browser.crumbs.count - 3))
                crumbRow(hiding: max(0, browser.crumbs.count - 2))
                crumbRow(hiding: max(0, browser.crumbs.count - 1))
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func crumbRow(hiding hidden: Int) -> some View {
        let crumbs = browser.crumbs
        return HStack(spacing: 2) {
            if hidden > 0 {
                Menu("…") {
                    ForEach(crumbs.prefix(hidden)) { crumb in
                        Button(crumb.title) { browser.open(crumb.path) }
                    }
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .help("Папки выше")
            }
            ForEach(Array(crumbs.dropFirst(hidden).enumerated()), id: \.element.id) { index, crumb in
                if index > 0 || hidden > 0 {
                    Image(systemName: "chevron.compact.right").foregroundStyle(.tertiary)
                }
                Button(crumb.title) { browser.open(crumb.path) }
                    .buttonStyle(.borderless)
                    .fontWeight(crumb.path == browser.path ? .semibold : .regular)
                    .foregroundStyle(crumb.path == browser.path ? .primary : .secondary)
                    .lineLimit(1)
                    .fixedSize()
                    .help(crumb.path)
            }
        }
    }

    @ViewBuilder
    private func row(_ entry: LocalWorkspaceEntry) -> some View {
        let absolute = ScenarioFolderBrowser.join(browser.path, entry.name)
        let relative = ScenarioFolderBrowser.relative(absolute, to: root)
        HStack(spacing: 8) {
            if showsSelection {
                if let relative {
                    let implied = ScenarioFolderBrowser.isImplied(relative, by: selection)
                    Toggle(isOn: Binding(
                        get: { implied || selection.contains(relative) },
                        set: { isOn in
                            if isOn { selection.insert(relative) } else { selection.remove(relative) }
                        }
                    )) { EmptyView() }
                        .toggleStyle(.checkbox)
                        .labelsHidden()
                        .accessibilityLabel("Выбрать " + entry.name)
                        .disabled(implied)
                        .help(implied ? "Входит в выбранную папку" : "Синхронизировать этот объект")
                } else {
                    // Keeps the names aligned outside the root.
                    Color.clear.frame(width: 14, height: 14)
                }
            }
            Image(systemName: entry.is_directory ? "folder.fill" : "doc")
                .foregroundStyle(entry.is_directory ? Color.accentColor : Color.secondary)
            Text(entry.name).lineLimit(1).truncationMode(.middle)
            if absolute == root {
                Text("корень").font(.caption2.weight(.semibold))
                    .padding(.horizontal, 6).padding(.vertical, 1)
                    .background(.tint.opacity(0.18), in: Capsule())
            }
            Spacer(minLength: 8)
            if entry.is_directory {
                Button("Открыть", systemImage: "chevron.right") { browser.openChild(entry.name) }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                    .help("Открыть папку")
            } else {
                Text(ByteCountFormatter.string(fromByteCount: Int64(clamping: entry.size), countStyle: .file))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
        }
        .contentShape(Rectangle())
        // Simultaneous: a plain double-tap gesture makes every single click,
        // including the checkbox, wait for the double-click interval.
        .simultaneousGesture(TapGesture(count: 2).onEnded {
            if entry.is_directory { browser.openChild(entry.name) }
        })
    }

    private func openDraftPath() {
        let path = (pathDraft.trimmingCharacters(in: .whitespaces) as NSString).expandingTildeInPath
        guard path.hasPrefix("/") else { return }
        browser.open(path.count > 1 && path.hasSuffix("/") ? String(path.dropLast()) : path)
        showsPathField = false
    }

    private var rootLine: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text("Корневая папка").font(.caption).foregroundStyle(.secondary)
                Spacer()
                if let previous = previousRoot {
                    Button("Отменить", systemImage: "arrow.uturn.backward") {
                        root = previous.root
                        selection = previous.selection
                        previousRoot = nil
                        if !previous.root.isEmpty { browser.open(previous.root) }
                    }
                    .help("Вернуть прежнюю корневую папку и выбор")
                }
                if !root.isEmpty {
                    Button("Показать корень", systemImage: "scope") { browser.open(root) }
                        .disabled(browser.path == root)
                    Button("Сбросить", systemImage: "xmark.circle") { changeRoot(to: "") }
                        .help("Убрать корневую папку")
                }
            }
            .buttonStyle(.borderless)
            .labelStyle(.titleAndIcon)
            .font(.caption)
            Text(root.isEmpty ? "не выбрана" : displayedRoot)
                .font(.caption.monospaced())
                .foregroundStyle(root.isEmpty ? .secondary : .primary)
                .lineLimit(1)
                .truncationMode(.middle)
                .textSelection(.enabled)
                .help(root)
            Button {
                changeRoot(to: browser.path)
            } label: {
                Label("Сделать корнем «\(folderTitle(browser.path))»", systemImage: "folder.badge.checkmark")
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            .disabled(browser.path == root)
            .help("Открытая папка станет корневой папкой этой стороны сценария")
        }
    }

    /// An SMB root reads as "<connection>: /folder", not as the path of the
    /// QADREN mount directory.
    private var displayedRoot: String {
        if isYandex, let entry = selectedConnection, root.hasPrefix("disk:/") {
            return entry.name + ": " + root.dropFirst("disk:".count)
        }
        guard isNetworkDrive, let entry = selectedConnection,
              let base = SMBMounts.path(for: entry.id), root.hasPrefix(base) else { return root }
        let inner = String(root.dropFirst(base.count))
        return entry.name + ": " + (inner.isEmpty ? "/" : inner)
    }

    /// Show the files of the selected connection. A network drive is mounted
    /// into its QADREN directory first (D-022).
    private func showConnection() {
        guard let entry = selectedConnection else { return }
        if entry.adapter == "yandex_disk" {
            mountResult = nil
            // A root from before D-023 is not a Yandex Disk path.
            if !root.isEmpty && !root.hasPrefix("disk:/") { changeRoot(to: "") }
            browser.rebase(
                "disk:/", title: entry.name,
                open: root.hasPrefix("disk:/") ? root : "disk:/", yandexConnectionID: entry.id
            )
            return
        }
        guard entry.adapter == "smb_share" else {
            mountResult = nil
            browser.rebase("/", title: "Этот Mac", open: root.isEmpty ? NSHomeDirectory() : root)
            return
        }
        mounting = true
        mountResult = nil
        Task {
            let result = await SMBMounts.ensureMounted(entry)
            mounting = false
            showMounted(result)
        }
    }

    private func showMounted(_ result: SMBMounts.Result) {
        mountResult = result
        guard result == .mounted, let entry = selectedConnection,
              let base = SMBMounts.path(for: entry.id) else { return }
        let inside = root == base || root.hasPrefix(base + "/")
        browser.rebase(base, title: entry.name, open: inside ? root : base)
    }

    /// Change the root and keep one undo step. The selection is relative to
    /// the root, so a new root starts with an empty selection.
    private func changeRoot(to newRoot: String) {
        previousRoot = (root, selection)
        root = newRoot
        selection = []
    }

    private func folderTitle(_ path: String) -> String {
        if path == "/" { return "Этот Mac" }
        if path == browser.base { return selectedConnection?.name ?? path }
        return path.split(separator: "/").last.map(String.init) ?? path
    }

    @ViewBuilder
    private var selectionSummary: some View {
        if root.isEmpty {
            Text("Откройте нужную папку и нажмите «Сделать корнем». Файлы не читаются и не передаются при выборе.")
                .font(.caption).foregroundStyle(.secondary)
        } else if selection.isEmpty {
            Text("Синхронизируется вся корневая папка. Отметьте объекты, чтобы синхронизировать только их.")
                .font(.caption).foregroundStyle(.secondary)
        } else {
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text("Выбрано объектов: \(selection.count)").font(.caption.weight(.semibold))
                    Spacer()
                    Button("Снять выбор") { selection = [] }
                        .buttonStyle(.borderless)
                        .font(.caption)
                }
                Text(selection.sorted().prefix(4).joined(separator: ", ") + (selection.count > 4 ? " …" : ""))
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                Text("Новые объекты в выбранных папках входят в сценарий. Переименованный объект и новые объекты рядом с выбранными не входят.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}

/// A choice between a few options as equal cards. The selected card has an
/// accent border and a check mark; there is no filled segment.
private struct ChoiceCards<Value: Hashable>: View {
    struct Option: Identifiable {
        let value: Value
        let title: String
        let detail: String
        let symbol: String
        var id: Value { value }
    }

    let options: [Option]
    let selection: Value
    let choose: (Value) -> Void

    var body: some View {
        HStack(spacing: 10) {
            ForEach(options) { option in
                card(option, selected: option.value == selection)
            }
        }
    }

    private func card(_ option: Option, selected: Bool) -> some View {
        let shape = RoundedRectangle(cornerRadius: 8, style: .continuous)
        return Button { choose(option.value) } label: {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: option.symbol)
                    .font(.title3)
                    .foregroundStyle(selected ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                    .frame(width: 24)
                VStack(alignment: .leading, spacing: 3) {
                    Text(option.title).font(.headline)
                    Text(option.detail)
                        .font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(selected ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(selected ? AnyShapeStyle(.tint.opacity(0.10)) : AnyShapeStyle(.quaternary.opacity(0.5)), in: shape)
            .overlay(shape.strokeBorder(selected ? AnyShapeStyle(.tint) : AnyShapeStyle(.quaternary), lineWidth: 1))
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(option.title)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

private struct EndpointLine: View {
    let title: String
    let connection: String
    let root: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(title).font(.caption).foregroundStyle(.secondary).frame(width: 82, alignment: .leading)
            Text(connection).lineLimit(1)
            Text(root).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
            Spacer(minLength: 0)
        }
        .help(root)
    }
}

private struct SettingsNotice<Actions: View>: View {
    let text: String
    let symbol: String
    @ViewBuilder let actions: Actions

    init(text: String, symbol: String, @ViewBuilder actions: () -> Actions = { EmptyView() }) {
        self.text = text
        self.symbol = symbol
        self.actions = actions()
    }

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: symbol).foregroundStyle(.orange)
            Text(text).font(.callout)
            Spacer(minLength: 0)
            actions
        }
        .padding(12)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

private struct SettingsEmptyState<Actions: View>: View {
    let title: String
    let detail: String
    let symbol: String
    @ViewBuilder let actions: Actions

    init(
        title: String,
        detail: String,
        symbol: String,
        @ViewBuilder actions: () -> Actions = { EmptyView() }
    ) {
        self.title = title
        self.detail = detail
        self.symbol = symbol
        self.actions = actions()
    }

    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: symbol).font(.system(size: 28)).foregroundStyle(.secondary)
            Text(title).font(.headline)
            Text(detail).font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
            actions
        }
        .padding(28)
        .frame(maxWidth: .infinity)
    }
}

/// "Запускать QADREN при входе в систему" (issue #33). The switch shows the
/// real state of the login LaunchAgent, including "requires approval".
struct LaunchAtLoginSection: View {
    @State private var state = LoginHandoff.state
    @State private var error: String?

    static let explanation = "Фоновая служба не может сама прочитать пароли и токены QADREN из Связки ключей: у приложения нет подписи Apple Developer ID. Приложение передаёт их службе, и служба хранит их только в памяти. После перезагрузки Mac память пуста. С этой настройкой QADREN при входе в систему передаёт службе данные для входа и сразу закрывается, без окна. Без неё сценарии с Яндекс Диском и сетевыми дисками ждут, пока вы откроете QADREN."

    var body: some View {
        Section("Запуск") {
            Toggle("Запускать QADREN при входе в систему", isOn: Binding(
                get: { state != .off },
                set: { enabled in
                    do {
                        try LoginHandoff.setEnabled(enabled)
                        error = nil
                    } catch {
                        self.error = error.localizedDescription
                    }
                    state = LoginHandoff.state
                }
            ))
            if state == .requiresApproval {
                HStack {
                    Label("Нужно разрешение в Системных настройках › Объекты входа", systemImage: "exclamationmark.triangle")
                        .foregroundStyle(QDTheme.warning)
                    Spacer()
                    Button("Открыть") { LoginHandoff.openLoginItemsSettings() }
                }
                .font(.callout)
            }
            if let error {
                Text(error).font(.caption).foregroundStyle(QDTheme.warning)
            }
            Text(Self.explanation)
                .font(.caption).foregroundStyle(.secondary)
        }
        .onAppear { state = LoginHandoff.state }
    }
}

struct ApplicationSettingsView: View {
    @AppStorage("qadren.theme") private var themeRaw = QDThemeMode.dark.rawValue
    @AppStorage(QDGlitchSetting.key) private var glitchEffects = false

    var body: some View {
        Form {
            LaunchAtLoginSection()
            Section("Оформление") {
                Picker("Тема", selection: $themeRaw) {
                    ForEach(QDThemeMode.allCases) { mode in
                        Text(mode.title).tag(mode.rawValue)
                    }
                }
                Text("Применяется ко всем окнам QADREN и не меняет подключения, сценарии или Worker.")
                    .font(.caption).foregroundStyle(.secondary)
                Toggle("Глитч-эффекты на контрольной панели", isOn: $glitchEffects)
                Text("Короткие редкие помехи в заголовках виджетов, чтобы картинка панели не стояла неподвижно часами. Не работают, если в macOS включено «Уменьшение движения».")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("Состояние приложения") {
                LabeledContent("Настройки", value: "Отдельное окно")
                LabeledContent("Выполнение сценариев", value: "Local Filesystem · Local Worker")
            }
            AppDataSection()
            Section("О приложении") {
                HStack(spacing: 10) {
                    Image(nsImage: NSApp.applicationIconImage)
                        .resizable().frame(width: 28, height: 28)
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 1) {
                        Text("QADREN \(appVersion)").font(.body.weight(.medium))
                        Text("Core \(CoreBridge.version) · автор Xsoid")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 8)
                    Link("xsoid.net", destination: URL(string: "https://xsoid.net")!)
                        .font(.callout)
                }
                .textSelection(.enabled)
            }
        }
        .formStyle(.grouped)
        .padding(20)
        .navigationTitle("Настройки приложения")
    }

    /// Marketing version and build number from Info.plist.
    private var appVersion: String {
        let info = Bundle.main.infoDictionary
        let version = info?["CFBundleShortVersionString"] as? String ?? "—"
        let build = info?["CFBundleVersion"] as? String ?? "—"
        return "\(version) (сборка \(build))"
    }
}

/// Where QADREN keeps its own state on this Mac and how much it takes.
/// Synchronized files are not counted.
private struct AppDataSection: View {
    @State private var locations: [AppDataLocation] = []
    @State private var scanning = false

    var body: some View {
        Section {
            if locations.isEmpty {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Считаю размер…").font(.callout).foregroundStyle(.secondary)
                }
            } else {
                ForEach(locations) { location in
                    AppDataRow(location: location)
                }
            }
        } header: {
            HStack(spacing: 6) {
                Text("Служебные данные")
                Spacer()
                if !locations.isEmpty {
                    Text("Всего \(AppDataFootprint.format(AppDataFootprint.total(locations)))")
                        .monospacedDigit().foregroundStyle(.secondary)
                }
                Button {
                    refresh()
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .buttonStyle(.borderless)
                .disabled(scanning)
                .help("Пересчитать размер")
            }
        } footer: {
            Text("Папки, которые QADREN создаёт для себя. Синхронизируемые файлы сюда не входят.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .onAppear(perform: refresh)
    }

    private func refresh() {
        guard !scanning else { return }
        scanning = true
        Task {
            let result = await Task.detached(priority: .utility) { AppDataFootprint.scan() }.value
            locations = result
            scanning = false
        }
    }
}

private struct AppDataRow: View {
    let location: AppDataLocation

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: location.symbol)
                .foregroundStyle(.secondary)
                .frame(width: 16)
            VStack(alignment: .leading, spacing: 1) {
                Text(location.title)
                if let path = displayPath {
                    Text(path)
                        .font(.caption.monospaced()).foregroundStyle(.secondary)
                        .lineLimit(1).truncationMode(.middle)
                        .help(location.url?.path ?? "")
                }
                if let breakdown = location.breakdown {
                    Text(breakdown).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        .help(breakdown)
                }
            }
            .help(location.note ?? "")
            Spacer(minLength: 8)
            Text(sizeText)
                .monospacedDigit()
                .foregroundStyle(isMissing ? .tertiary : .secondary)
            if let url = location.url {
                Button {
                    NSWorkspace.shared.activateFileViewerSelecting([url])
                } label: {
                    Image(systemName: "arrow.up.forward.square")
                }
                .buttonStyle(.borderless)
                .disabled(isMissing)
                .help("Показать в Finder")
            }
        }
        .contextMenu {
            if let url = location.url {
                Button("Скопировать путь") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(url.path, forType: .string)
                }
            }
        }
    }

    private var displayPath: String? {
        location.url.map { ($0.path as NSString).abbreviatingWithTildeInPath }
    }

    private var isMissing: Bool { location.size == .missing }

    private var sizeText: String {
        switch location.size {
        case .bytes(let value): AppDataFootprint.format(value)
        case .notCounted(let note): note
        case .missing: "Не создано"
        }
    }
}

private extension View {
    /// Content never scrolls under the titlebar. The macOS scroll edge
    /// effect is drawn only as wide as the scroll view, so it stopped short
    /// of the window edges and changed when the scroller appeared on hover.
    /// Without it, with clipping, and without the toolbar background and its
    /// separator line, the titlebar has one plain fill.
    @ViewBuilder
    func contentBelowTitlebar() -> some View {
        if #available(macOS 26.0, *) {
            scrollEdgeEffectHidden(true, for: .top)
                .clipped()
                .toolbarBackgroundVisibility(.hidden, for: .windowToolbar)
        } else {
            clipped()
        }
    }

    /// The short section list never scrolls under the titlebar, so its top
    /// edge separator only draws a stray line.
    @ViewBuilder
    func withoutTopScrollEdge() -> some View {
        if #available(macOS 26.0, *) {
            scrollEdgeEffectHidden(true, for: .top)
        } else {
            self
        }
    }

    /// The system button moves between the sidebar and the toolbar and can
    /// fall into the toolbar overflow menu. The titlebar button replaces it.
    @ViewBuilder
    func withoutSystemSidebarToggle() -> some View {
        if #available(macOS 14.0, *) {
            toolbar(removing: .sidebarToggle)
        } else {
            self
        }
    }
}
