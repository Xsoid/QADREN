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
            _ = try StorageRegistryBridge.createScenario(
                name: name,
                leftID: leftID,
                leftRoot: leftRoot,
                rightID: rightID,
                rightRoot: rightRoot,
                direction: direction,
                selectedPaths: scope.selectedPaths.sorted(),
                deletionMode: scope.deletionMode
            )
            refresh()
            return nil
        } catch {
            self.error = error.localizedDescription
            return self.error
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
    @Published private(set) var error: String?

    func refresh(scenarioID: String) {
        do {
            entries = try StorageRegistryBridge.listScenarioSchedules(scenarioID: scenarioID)
            automation = try StorageRegistryBridge.scenarioAutomation(scenarioID: scenarioID)
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

    func setLocallyPaused(scenarioID: String, paused: Bool) {
        do {
            try StorageRegistryBridge.setScenarioLocallyPaused(scenarioID: scenarioID, paused: paused)
            refresh(scenarioID: scenarioID)
        } catch {
            self.error = error.localizedDescription
        }
    }

    func create(scenarioID: String, timezone: String, weekday: UInt8, hour: UInt8, minute: UInt8) {
        do {
            try StorageRegistryBridge.createScenarioSchedule(
                scenarioID: scenarioID, timezone: timezone, weekday: weekday, hour: hour, minute: minute
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
        var endpoints = connections.entries.filter { $0.adapter == "local_filesystem" }
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
    case yandexDisk = "yandex_disk"

    var id: String { rawValue }

    init?(adapter: String) {
        self.init(rawValue: adapter)
    }

    var title: String {
        switch self {
        case .thisMac: "Этот Mac"
        case .yandexDisk: "Яндекс.Диск"
        }
    }

    var symbol: String {
        switch self {
        case .thisMac: "laptopcomputer"
        case .yandexDisk: "cloud"
        }
    }

    var tint: Color {
        switch self {
        case .thisMac: QDTheme.cyan
        case .yandexDisk: QDTheme.warning
        }
    }

    var summary: String {
        switch self {
        case .thisMac: "Папки и диски этого Mac. Одно подключение на Mac, создаётся автоматически."
        case .yandexDisk: "Облачное хранилище. Подключений может быть сколько угодно."
        }
    }
}

private struct ConnectionsSettingsView: View {
    @ObservedObject var connections: StorageConnections
    @State private var showsAddConnection = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Подключения").font(.largeTitle.bold())
                        Text("Хранилища, с которыми работают сценарии. Каждое подключение относится к одному коннектору.")
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Добавить подключение", systemImage: "plus") { showsAddConnection = true }
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
                        canRemove: canRemove(entry),
                        onRemove: { connections.remove(entry.id) },
                        onRename: { connections.rename(entry.id, name: $0) },
                        onSetEnabled: { connections.setEnabled(entry.id, enabled: $0) }
                    )
                    .id(entry.id + (entry.enabled ? ":enabled" : ":disabled") + ":" + entry.name)
                }
                Text("Удаление подключения меняет только локальную запись QADREN и её ключ в Keychain. Файлы в хранилище не удаляются.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            .frame(maxWidth: 900, alignment: .leading)
            .padding(24)
        }
        .sheet(isPresented: $showsAddConnection) {
            AddConnectionSheet(connections: connections.entries)
        }
        .onAppear {
            connections.refresh()
            // The cards load after the window cleared its initial focus.
            // Opening the section must not select a name field.
            DispatchQueue.main.async { NSApp.keyWindow?.makeFirstResponder(nil) }
        }
    }

    /// "This Mac" always exists. Only an extra Local record from older data
    /// can be removed.
    private func canRemove(_ entry: StorageEntry) -> Bool {
        guard entry.adapter == StorageConnector.thisMac.rawValue else { return true }
        return connections.entries.filter { $0.adapter == entry.adapter }.count > 1
    }
}

/// One connection with a visible boundary: connector icon and color, name,
/// status, and the actions that the connector supports now.
private struct ConnectionCard: View {
    let entry: StorageEntry
    let canRemove: Bool
    let onRemove: () -> Void
    let onRename: (String) -> Void
    let onSetEnabled: (Bool) -> Void
    @State private var name: String
    @State private var confirmRemoval = false

    init(
        entry: StorageEntry, canRemove: Bool, onRemove: @escaping () -> Void,
        onRename: @escaping (String) -> Void, onSetEnabled: @escaping (Bool) -> Void
    ) {
        self.entry = entry
        self.canRemove = canRemove
        self.onRemove = onRemove
        self.onRename = onRename
        self.onSetEnabled = onSetEnabled
        _name = State(initialValue: entry.name)
    }

    private var connector: StorageConnector? { StorageConnector(adapter: entry.adapter) }
    private var tint: Color { connector?.tint ?? .secondary }
    private var parked: Bool { connector == .yandexDisk }
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
                    .disabled(parked && !entry.enabled)
                }
            }
            if parked {
                SettingsNotice(
                    text: "Коннектор заморожен до P3. QADREN хранит запись и ключ в Keychain, но не запускает OAuth и API-вызовы.",
                    symbol: "snowflake"
                )
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
            Text(parked
                ? "Будут удалены только локальная запись QADREN и её ключ в Keychain. Файлы на Яндекс.Диске не изменятся."
                : "Будет удалена только лишняя локальная запись QADREN. Файлы на этом Mac не изменятся.")
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
            if parked {
                QDStatusLabel(kind: .paused, text: "Заморожено")
            } else if entry.enabled {
                QDStatusLabel(kind: .ready, text: "Включено")
            } else {
                QDStatusLabel(kind: .offline, text: "Выключено")
            }
        }
    }
}

/// The connectors that the user can add now (D-020). "This Mac" exists once
/// and is created automatically; Yandex Disk is parked until P3.
private struct AddConnectionSheet: View {
    let connections: [StorageEntry]
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 4) {
                Text("Добавить подключение").font(.title2.bold())
                Text("Выберите коннектор — тип хранилища.").foregroundStyle(.secondary)
            }
            ForEach(StorageConnector.allCases) { connector in
                row(connector)
            }
            HStack {
                Spacer()
                Button("Закрыть") { dismiss() }
                    .keyboardShortcut(.cancelAction)
            }
        }
        .padding(24)
        .frame(width: 520)
        .background(InitialFocusReset().frame(width: 0, height: 0))
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
            Button("Подключить") {}
                .disabled(true)
                .help(state.text)
        }
        .padding(12)
        .background(connector.tint.opacity(0.05), in: shape)
        .overlay(shape.strokeBorder(connector.tint.opacity(0.35), lineWidth: 1))
    }

    private func availability(_ connector: StorageConnector) -> (kind: QDStatusKind, text: String) {
        switch connector {
        case .thisMac:
            let name = connections.first { $0.adapter == connector.rawValue }?.name ?? connector.title
            return (.ready, "Уже подключён как «\(name)». Второе подключение не нужно.")
        case .yandexDisk:
            return (.paused, "Заморожен до P3. Новые подключения сейчас не создаются.")
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
                                .disabled(!isLocalScenario(scenario))
                                .help(isLocalScenario(scenario)
                                      ? "Изменить локальный сценарий"
                                      : "Сценарий с Яндекс Диском отложен до P3")
                            Button("Расписания") { editSchedules(scenario) }
                                .disabled(!isLocalScenario(scenario))
                                .help(isLocalScenario(scenario)
                                      ? "Настроить расписания локального сценария"
                                      : "Сценарий с Яндекс Диском отложен до P3")
                            Button("Проверить план") { previewScenario(scenario) }
                                .disabled(!isLocalScenario(scenario))
                                .help(isLocalScenario(scenario)
                                      ? "Явно прочитать только метаданные двух локальных каталогов и показать план без передачи файлов"
                                      : "Сценарий с Яндекс Диском отложен до P3")
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
        connections.filter { $0.adapter == "local_filesystem" }
    }

    private func isLocalScenario(_ scenario: ScenarioEntry) -> Bool {
        let endpointIDs = [scenario.left_connection_id, scenario.right_connection_id]
        return endpointIDs.allSatisfy { id in
            connections.first(where: { $0.id == id })?.adapter == "local_filesystem"
        }
    }

    private func scenarioScopeSummary(_ scenario: ScenarioEntry) -> String {
        let objects = scenario.selected_paths.isEmpty
            ? "Вся папка"
            : "Выбрано объектов: \(scenario.selected_paths.count)"
        let deletions = scenario.deletion_mode == ScenarioDeletionMode.simple.rawValue
            ? "удаления: простая синхронизация"
            : "удаления: параноик"
        return objects + " · " + deletions
    }

    private func scenarioStatus(_ scenario: ScenarioEntry) -> String {
        if !isLocalScenario(scenario) {
            return "Сценарий использует Яндекс Диск и отложен до P3. Его запуск, изменение и расписания недоступны; запись сохранена локально."
        }
        return scenario.enabled
            ? "Проверьте план по метаданным, затем подтвердите отдельный перенос или пакет обычных действий. Конфликты обрабатываются отдельно."
            : "Автоматические триггеры для этого сценария выключены."
    }

    private func adapterTitle(_ adapter: String) -> String {
        adapter == "yandex_disk" ? "Яндекс Диск" : adapter
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
    @State private var pendingBatch: [ScenarioPlanItem] = []
    @State private var pendingBatchKind: String?
    @State private var activeBatch: ScenarioBatchProgress?
    @State private var batchTask: Task<Void, Never>?
    @State private var batchProgressTask: Task<Void, Never>?
    @State private var resumedCreateBatches: [ScenarioBatchProgress] = []
    @State private var workerOwnedCreateBatch = false
    /// True when the last plan read failed. An empty plan is then not
    /// evidence of "no changes".
    @State private var planUnavailable = false

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
                Button("Проверить план", systemImage: "arrow.triangle.2.circlepath") { loadPlan() }
                    .disabled(isLoading)
            }
            Text("Кнопка читает только имена и размеры файлов из выбранных локальных каталогов. Она не открывает содержимое, не вычисляет хэши, не создаёт Job и не передаёт файлы.")
                .font(.callout).foregroundStyle(.secondary)
            if let error {
                SettingsNotice(text: error, symbol: "exclamationmark.triangle")
            }
            if let skippedNotice {
                SettingsNotice(text: skippedNotice, symbol: "link")
            }
            if isLoading {
                ProgressView("Чтение метаданных и построение плана…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
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
                if scenario.deletion_mode == ScenarioDeletionMode.simple.rawValue,
                   plan.contains(where: { $0.action.kind == "delete_requires_decision" }) {
                    SettingsNotice(
                        text: "В плане больше 20 удалений. Простая синхронизация не выполняет массовое удаление сама: решите по каждому удалению отдельно.",
                        symbol: "exclamationmark.triangle"
                    )
                }
                if !creates.isEmpty || !updates.isEmpty || !ruleDeletes.isEmpty {
                    HStack {
                        if !ruleDeletes.isEmpty {
                            Button("Удалить по правилу: \(ruleDeletes.count)", role: .destructive) {
                                executeRuleDeletions()
                            }
                            .disabled(activeBatch != nil || !resumedCreateBatches.isEmpty)
                            .help("Файлы, удалённые в источнике, переносятся в Корзину. На томе без Корзины удаление безвозвратное.")
                        }
                        if !creates.isEmpty {
                            Button("Создать все: \(creates.count)") {
                                executeCreateBatch(creates)
                            }
                            .disabled(activeBatch != nil || !resumedCreateBatches.isEmpty)
                        }
                        if !updates.isEmpty {
                            Button("Заменить все: \(updates.count)", role: .destructive) {
                                pendingBatch = updates; pendingBatchKind = "update"
                            }
                            .disabled(activeBatch != nil || !resumedCreateBatches.isEmpty)
                        }
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
                            Text("Local Worker продолжает подтверждённые пакеты после закрытия Settings.")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            ForEach(resumedCreateBatches) { batch in
                                VStack(alignment: .leading, spacing: 4) {
                                    ProgressView(value: Double(batch.processed), total: Double(batch.total))
                                    Text("Подтверждено \(batch.processed) из \(batch.total) · \(batch.currentPath)")
                                        .font(.caption.monospaced())
                                    if batch.cancellationRequested {
                                        Text("Отмена сохранена. QADREN завершит текущую операцию и не запустит следующий файл.")
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    } else {
                                        Button("Остановить после текущего файла", role: .cancel) {
                                            cancelResumedWorkerCreateBatch(batch)
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
        .alert("Выполнить пакет сценария?", isPresented: Binding(
            get: { !pendingBatch.isEmpty }, set: { if !$0 { pendingBatch = []; pendingBatchKind = nil } }
        )) {
            Button("Отмена", role: .cancel) { pendingBatch = []; pendingBatchKind = nil }
            Button("Заменить файлы", role: .destructive) {
                executePendingBatch()
            }
        } message: {
            Text("Будет выполнено: \(pendingBatch.count).\n\n\(pendingBatch.map(\.relative_path).joined(separator: "\n"))\n\nДля каждого файла QADREN создаст отдельный Job и повторно проверит его условия. Конфликтные задачи в этот пакет не входят.")
        }
    }

    private func loadPlan() {
        isLoading = true
        error = nil
        planUnavailable = false
        defer { isLoading = false; hasRequestedPreview = true }
        do {
            // A plan for a superseded revision must not offer actions.
            if let current = try StorageRegistryBridge.listScenarios().first(where: { $0.id == scenario.id }),
               current.revision != scenario.revision {
                plan = []
                planUnavailable = true
                error = "Сценарий изменён (ревизия \(current.revision)). Этот план построен для ревизии \(scenario.revision). Вернитесь к списку сценариев и откройте план заново."
                return
            }
            try StorageRegistryBridge.dispatchPendingScenarioCopyBaselines()
            let left = try StorageRegistryBridge.localScenarioSnapshot(
                root: scenario.left_root, connectionID: scenario.left_connection_id
            )
            let right = try StorageRegistryBridge.localScenarioSnapshot(
                root: scenario.right_root, connectionID: scenario.right_connection_id
            )
            leftSnapshot = left
            rightSnapshot = right
            plan = try StorageRegistryBridge.dryRunScenario(
                id: scenario.id, revision: scenario.revision, left: left, right: right
            )
            loadConflictTasks()
        } catch {
            plan = []
            planUnavailable = true
            self.error = "Не удалось прочитать метаданные или построить план: " + error.localizedDescription
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

    private func executePendingBatch() {
        let items = pendingBatch
        let kind = pendingBatchKind
        pendingBatch = []
        pendingBatchKind = nil
        guard !items.isEmpty, (kind == "create" || kind == "update") else { return }
        let batchScenario = scenario
        // Capture the plan metadata now. The Worker compares it with the files.
        let plannedSnapshots = Dictionary(
            items.map { ($0.id, plannedSnapshots(for: $0)) }, uniquingKeysWith: { first, _ in first }
        )
        activeBatch = ScenarioBatchProgress(
            total: items.count, processed: 0, failures: 0, currentPath: items[0].relative_path,
            cancellationRequested: false
        )
        workerOwnedCreateBatch = kind == "create"
        if kind == "create" {
            let receiptID: String
            do {
                receiptID = try StorageRegistryBridge.confirmScenarioCreateBatch(
                    scenario: batchScenario, items: items
                )
                activeBatch = ScenarioBatchProgress(
                    total: items.count, processed: 0, failures: 0, currentPath: items[0].relative_path,
                    cancellationRequested: false, receiptID: receiptID
                )
            } catch {
                activeBatch = nil
                workerOwnedCreateBatch = false
                self.error = "Не удалось подтвердить пакет: \(error.localizedDescription)"
                return
            }
            batchProgressTask = Task {
                while !Task.isCancelled {
                    try? await Task.sleep(for: .milliseconds(200))
                    // Cancellation ends the sleep early. Do not write progress
                    // after the batch completion cleared it.
                    guard !Task.isCancelled else { return }
                    guard let receipt = try? StorageRegistryBridge.scenarioCreateBatch(id: receiptID) else {
                        continue
                    }
                    let completed = receipt.children.filter { $0.state == "completed" }.count
                    let failures = receipt.children.filter { $0.state != "pending" && $0.state != "completed" }.count
                    activeBatch = ScenarioBatchProgress(
                        total: receipt.children.count, processed: completed, failures: failures,
                        currentPath: receipt.children.first { $0.state == "pending" }?.relative_path
                            ?? receipt.children.last?.relative_path ?? "",
                        cancellationRequested: receipt.cancellation_requested, receiptID: receiptID
                    )
                    if receipt.children.allSatisfy({ $0.state != "pending" }) { return }
                }
            }
            batchTask = Task.detached(priority: .userInitiated) {
                var failure: String?
                do {
                    _ = try WorkerIPC.executeScenarioCreateBatch(receiptID: receiptID)
                } catch {
                    failure = error.localizedDescription
                }
                let receipt = try? StorageRegistryBridge.scenarioCreateBatch(id: receiptID)
                let outcome = receipt.map(Self.createBatchOutcome)
                    ?? failure.map { "Worker не подтвердил завершение пакета: \($0)" }
                await MainActor.run {
                    batchProgressTask?.cancel()
                    batchProgressTask = nil
                    activeBatch = nil
                    batchTask = nil
                    workerOwnedCreateBatch = false
                    loadPlan()
                    // loadPlan() clears the previous notice. Show the batch
                    // outcome after it and keep a plan read error visible.
                    if let outcome { error = [outcome, error].compactMap { $0 }.joined(separator: "\n") }
                }
            }
            return
        }
        batchTask = Task.detached(priority: .userInitiated) {
            var failures: [String] = []
            var processed = 0
            for (offset, item) in items.enumerated() {
                guard !Task.isCancelled else { break }
                let progressBeforeItem = ScenarioBatchProgress(
                    total: items.count, processed: processed, failures: failures.count,
                    currentPath: item.relative_path, cancellationRequested: false
                )
                await MainActor.run {
                    activeBatch = progressBeforeItem
                }
                do {
                    if kind == "create" {
                        try StorageRegistryBridge.executeScenarioCreate(scenario: batchScenario, item: item)
                    } else {
                        let planned = plannedSnapshots[item.id]
                        try StorageRegistryBridge.executeScenarioUpdate(
                            scenario: batchScenario, item: item,
                            plannedSource: planned?.source, plannedDestination: planned?.destination
                        )
                    }
                } catch {
                    failures.append(item.relative_path)
                }
                processed = offset + 1
                let progressAfterItem = ScenarioBatchProgress(
                    total: items.count, processed: processed, failures: failures.count,
                    currentPath: item.relative_path, cancellationRequested: Task.isCancelled
                )
                await MainActor.run {
                    activeBatch = progressAfterItem
                }
            }
            let finishedFailures = failures
            let wasCancelled = Task.isCancelled
            let processedCount = processed
            let total = items.count
            await MainActor.run {
                activeBatch = nil
                batchTask = nil
                workerOwnedCreateBatch = false
                loadPlan()
                if wasCancelled {
                    error = "Пакет остановлен после обработанных файлов: \(processedCount) из \(total). Незавершённые файлы не запускались."
                } else if !finishedFailures.isEmpty {
                    error = "Не удалось выполнить файлов: \(finishedFailures.joined(separator: ", ")). Остальные элементы пакета были обработаны."
                }
            }
        }
    }

    /// Describes a finished Worker-owned Create batch from its durable receipt.
    /// A cancelled child is not a failure: it never started.
    nonisolated static func createBatchOutcome(_ receipt: ScenarioCreateBatchReceipt) -> String? {
        let total = receipt.children.count
        let completed = receipt.children.filter { $0.state == "completed" }.count
        let cancelled = receipt.children.filter { $0.state == "cancelled" }.count
        let unfinished = receipt.children
            .filter { !["completed", "cancelled"].contains($0.state) }
            .map(\.relative_path)
        var parts: [String] = []
        if cancelled > 0 {
            parts.append("Пакет остановлен по запросу: создано \(completed) из \(total). Остальные файлы (\(cancelled)) не запускались.")
        }
        if !unfinished.isEmpty {
            let shown = unfinished.prefix(5).joined(separator: ", ")
            let rest = unfinished.count > 5 ? " и ещё \(unfinished.count - 5)" : ""
            parts.append("Не завершены файлы (\(unfinished.count)): \(shown)\(rest). Они не будут автоматически повторены.")
        }
        return parts.isEmpty ? nil : parts.joined(separator: " ")
    }

    private func executeCreateBatch(_ items: [ScenarioPlanItem]) {
        guard !items.isEmpty else { return }
        pendingBatch = items
        pendingBatchKind = "create"
        executePendingBatch()
    }

    private func resumePendingCreateBatches() {
        batchProgressTask?.cancel()
        batchProgressTask = Task {
            while !Task.isCancelled {
                let receipts = (try? StorageRegistryBridge.pendingScenarioCreateBatches(scenario: scenario)) ?? []
                resumedCreateBatches = receipts.map { receipt in
                    let completed = receipt.children.filter { $0.state == "completed" }.count
                    let failures = receipt.children.filter { $0.state != "pending" && $0.state != "completed" }.count
                    return ScenarioBatchProgress(
                        total: receipt.children.count,
                        processed: completed,
                        failures: failures,
                        currentPath: receipt.children.first { $0.state == "pending" }?.relative_path
                            ?? receipt.children.last?.relative_path ?? "",
                        cancellationRequested: receipt.cancellation_requested,
                        receiptID: receipt.id
                    )
                }
                if resumedCreateBatches.isEmpty {
                    batchProgressTask = nil
                    loadPlan()
                    return
                }
                try? await Task.sleep(for: .milliseconds(200))
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

    /// The Worker plans again and deletes only what the simple rule covers.
    private func executeRuleDeletions() {
        do {
            _ = try WorkerIPC.executeScenarioRuleDeletions(
                WorkerScenarioRuleDeletionsRequest(
                    scenario_id: scenario.id,
                    scenario_revision: scenario.revision,
                    batch_id: "scenario-rule-delete-" + UUID().uuidString.lowercased()
                )
            )
            loadPlan()
        } catch {
            self.error = "Удаление по правилу не выполнено: " + error.localizedDescription
            loadPlan()
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
    @State private var weekday: UInt8 = 0
    @State private var time = Date()

    private let weekdays = ["Понедельник", "Вторник", "Среда", "Четверг", "Пятница", "Суббота", "Воскресенье"]

    var body: some View {
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
                text: "Показаны только кандидаты на 14 дней. Просмотр не создаёт Job и не передаёт файлы.",
                symbol: "clock.badge.exclamationmark"
            )

            Form {
                Section("Автоматизация") {
                    Toggle("Приостановить автоматические триггеры", isOn: Binding(
                        get: { schedules.automation.locally_paused },
                        set: { schedules.setLocallyPaused(scenarioID: scenario.id, paused: $0) }
                    ))
                    Text("Приостановка сохраняется локально, очищает только ожидающий автоматический запуск и не меняет существующие Job. Ручные запросы остаются отдельным путём.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Section("Новое расписание") {
                    Picker("День", selection: $weekday) {
                        ForEach(Array(weekdays.enumerated()), id: \.offset) { index, title in
                            Text(title).tag(UInt8(index))
                        }
                    }
                    DatePicker("Время", selection: $time, displayedComponents: .hourAndMinute)
                    TextField("IANA timezone", text: $timezone)
                    HStack {
                        Spacer()
                        Button("Добавить расписание", action: create)
                            .disabled(timezone.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
            .formStyle(.grouped)

            if let error = schedules.error {
                SettingsNotice(text: error, symbol: "exclamationmark.triangle")
            }

            if !schedules.upcoming.isEmpty {
                GroupBox("Ближайшие кандидаты") {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(schedules.upcoming.prefix(5)) { occurrence in
                            VStack(alignment: .leading, spacing: 2) {
                                Text(occurrence.occurrence_key)
                                    .font(.body.monospaced())
                                Text("Расписание " + String(occurrence.schedule_id.prefix(8)))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        Text("Local Worker сохраняет наступивший weekly slot в существующую очередь Job и выполняет безопасные Create без открытого окна. Замены, удаления и конфликты остаются в WAITING_FOR_DECISION до явного решения.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }

            if schedules.entries.isEmpty {
                SettingsEmptyState(
                    title: "Расписаний пока нет",
                    detail: "Добавьте одно или несколько weekly расписаний. Их время хранится с явной IANA timezone.",
                    symbol: "calendar"
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(schedules.entries) { entry in
                    HStack(spacing: 12) {
                        Image(systemName: entry.enabled ? "calendar.badge.clock" : "calendar.badge.exclamationmark")
                            .foregroundStyle(entry.enabled ? QDTheme.cyan : .secondary)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(weekdays[Int(entry.weekday)] + " · " + timeTitle(entry))
                            Text(entry.timezone).font(.caption.monospaced()).foregroundStyle(.secondary)
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
                    .padding(.vertical, 4)
                }
                .listStyle(.inset)
            }
        }
        .padding(24)
        .onAppear { schedules.refresh(scenarioID: scenario.id) }
    }

    private func create() {
        let components = Calendar.current.dateComponents([.hour, .minute], from: time)
        guard let hour = components.hour, let minute = components.minute else { return }
        schedules.create(
            scenarioID: scenario.id,
            timezone: timezone.trimmingCharacters(in: .whitespacesAndNewlines),
            weekday: weekday,
            hour: UInt8(hour),
            minute: UInt8(minute)
        )
    }

    private func timeTitle(_ entry: ScenarioScheduleEntry) -> String {
        String(format: "%02d:%02d", entry.hour, entry.minute)
    }
}

private struct ScenarioEditorView: View {
    let connections: [StorageEntry]
    let scenario: ScenarioEntry?
    let saveScenario: (String, String, String, String, String, String, ScenarioScopeDraft) -> String?
    let cancel: () -> Void
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
                                .init(value: "bidirectional", title: "В обе стороны",
                                      detail: "Конфликты — только с вашим решением.", symbol: "arrow.left.arrow.right"),
                            ],
                            selection: direction,
                            choose: { direction = $0 }
                        )
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
                        showsSelection: true
                    )
                    ScenarioFolderPanel(
                        role: direction == "bidirectional" ? "Правая папка" : "Назначение",
                        symbol: "arrow.left.circle",
                        connections: connections,
                        connectionID: $rightID,
                        root: $rightRoot,
                        selection: .constant([]),
                        showsSelection: false
                    )
                }
                ScenarioDeletionModeBox(
                    mode: scope.deletionMode,
                    bidirectional: direction == "bidirectional",
                    choose: { mode in
                        if mode == .simple && scope.deletionMode != .simple {
                            confirmSimpleDeletions = true
                        } else {
                            scope.deletionMode = mode
                        }
                    }
                )
                Text("Создание сохраняет только конфигурацию. Передача файлов начинается позднее, только из проверенного плана и после отдельного подтверждения.")
                    .font(.caption).foregroundStyle(.secondary)
                if let error {
                    SettingsNotice(text: error, symbol: "exclamationmark.triangle")
                }
            }
            .padding(24)
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
                    deletionMode: ScenarioDeletionMode(rawValue: scenario.deletion_mode) ?? .paranoid
                )
            } else if let first = connections.first {
                leftID = first.id
                rightID = first.id
            }
        }
        .alert("Включить простую синхронизацию?", isPresented: $confirmSimpleDeletions) {
            Button("Отмена", role: .cancel) {}
            Button("Включить", role: .destructive) { scope.deletionMode = .simple }
        } message: {
            Text("Файлы, удалённые в источнике, будут удаляться и в назначении без вопроса — если они уже синхронизированы и не менялись. Удаление идёт в Корзину, без Корзины — навсегда. Больше 20 удалений — только с подтверждением.")
        }
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
                }
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
    @StateObject private var browser = ScenarioFolderBrowser()
    @State private var showsPathField = false
    @State private var pathDraft = ""
    @State private var shownConnectionID = ""
    @State private var showedInitialRoot = false
    /// One undo step for the root and its selection.
    @State private var previousRoot: (root: String, selection: Set<String>)?

    private var selectedConnection: StorageEntry? {
        connections.first { $0.id == connectionID }
    }

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Label(role, systemImage: symbol).font(.headline).lineLimit(1)
                    Spacer(minLength: 8)
                    Picker("Подключение", selection: $connectionID) {
                        ForEach(connections) { entry in
                            Text(entry.name).tag(entry.id)
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
                if let error = browser.error {
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
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onAppear { browser.open(root.isEmpty ? NSHomeDirectory() : root) }
        .onChange(of: root) { newRoot in
            // The root of an existing Scenario arrives after onAppear.
            if !showedInitialRoot, !newRoot.isEmpty, browser.path != newRoot {
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
            Button("Домой", systemImage: "house") { browser.open(NSHomeDirectory()) }
                .labelStyle(.iconOnly)
                .help("Домашняя папка")
            Button("Перейти к папке", systemImage: "text.cursor") {
                pathDraft = browser.path
                showsPathField.toggle()
            }
            .labelStyle(.iconOnly)
            .help("Ввести путь к папке")
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
            Text(root.isEmpty ? "не выбрана" : root)
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

    /// Change the root and keep one undo step. The selection is relative to
    /// the root, so a new root starts with an empty selection.
    private func changeRoot(to newRoot: String) {
        previousRoot = (root, selection)
        root = newRoot
        selection = []
    }

    private func folderTitle(_ path: String) -> String {
        path == "/" ? "Этот Mac" : (path as NSString).lastPathComponent
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

struct ApplicationSettingsView: View {
    @AppStorage("qadren.theme") private var themeRaw = QDThemeMode.dark.rawValue

    var body: some View {
        Form {
            Section("Оформление") {
                Picker("Тема", selection: $themeRaw) {
                    ForEach(QDThemeMode.allCases) { mode in
                        Text(mode.title).tag(mode.rawValue)
                    }
                }
                Text("Применяется ко всем окнам QADREN и не меняет подключения, сценарии или Worker.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("Состояние приложения") {
                LabeledContent("Настройки", value: "Отдельное окно")
                LabeledContent("Выполнение сценариев", value: "Local Filesystem · Local Worker")
            }
            Section("О приложении") {
                LabeledContent("Версия", value: appVersion)
                LabeledContent("Core", value: CoreBridge.version)
                LabeledContent("Автор", value: "Xsoid")
                LabeledContent("Сайт разработчика") {
                    Link("xsoid.net", destination: URL(string: "https://xsoid.net")!)
                }
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
