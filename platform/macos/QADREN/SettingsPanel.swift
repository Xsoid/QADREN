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
        direction: String
    ) -> String? {
        do {
            _ = try StorageRegistryBridge.createScenario(
                name: name,
                leftID: leftID,
                leftRoot: leftRoot,
                rightID: rightID,
                rightRoot: rightRoot,
                direction: direction
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
        direction: String
    ) -> String? {
        do {
            try StorageRegistryBridge.updateScenario(
                id: id,
                name: name,
                leftID: leftID,
                leftRoot: leftRoot,
                rightID: rightID,
                rightRoot: rightRoot,
                direction: direction
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
    @StateObject private var scenarios = ScenarioSettings()
    @State private var selection: SettingsPanelSection? = .connections
    @State private var isEditingScenario = false
    @State private var editedScenario: ScenarioEntry?
    @State private var schedulesScenario: ScenarioEntry?
    @State private var previewScenario: ScenarioEntry?

    var body: some View {
        NavigationSplitView {
            List(selection: $selection) {
                Section("Настройка") {
                    ForEach(SettingsPanelSection.allCases) { section in
                        Label(section.title, systemImage: section.symbol).tag(section)
                    }
                }
            }
            .listStyle(.sidebar)
            .navigationTitle("Настройки")
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
        }
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                switch selection ?? .connections {
                case .connections:
                    Button("Добавить этот Mac", systemImage: "plus") { connections.addLocalComputer() }
                case .scenarios:
                    if schedulesScenario == nil {
                        Button("Создать сценарий", systemImage: "plus", action: beginScenarioCreation)
                    }
                case .application:
                    EmptyView()
                }
                Button("Обновить", systemImage: "arrow.clockwise") {
                    connections.refresh()
                    scenarios.refresh()
                }
            }
        }
        .onAppear {
            connections.refresh()
            scenarios.refresh()
        }
    }

    private var scenarioEndpoints: [StorageEntry] {
        var endpoints = connections.entries.filter {
            $0.enabled && $0.adapter == "local_filesystem"
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

    private func saveScenario(
        _ name: String, _ leftID: String, _ leftRoot: String,
        _ rightID: String, _ rightRoot: String, _ direction: String
    ) -> String? {
        do {
            let localID: String?
            if leftID == "local-computer" || rightID == "local-computer" {
                localID = try (connections.entries.first { $0.adapter == "local_filesystem" && $0.enabled }?.id
                    ?? StorageRegistryBridge.addLocalComputer())
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
                    rightID: resolvedRightID, rightRoot: rightRoot, direction: direction
                )
            } else {
                result = scenarios.create(
                    name: name, leftID: resolvedLeftID, leftRoot: leftRoot,
                    rightID: resolvedRightID, rightRoot: rightRoot, direction: direction
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

private struct ConnectionsSettingsView: View {
    @ObservedObject var connections: StorageConnections

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Подключения").font(.largeTitle.bold())
                    Text("Независимые точки синхронизации и их локальные полномочия")
                        .foregroundStyle(.secondary)
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
                if connections.entries.isEmpty && !connections.hasLegacyYandexToken {
                    SettingsEmptyState(
                        title: "Подключений пока нет",
                    detail: "Добавьте этот Mac как отдельную локальную точку синхронизации. Яндекс Диск отложен до P3.",
                        symbol: "externaldrive.badge.plus"
                    )
                    .frame(maxWidth: .infinity, minHeight: 280)
                }
                ForEach(connections.entries) { entry in
                    GroupBox {
                        if entry.adapter == "yandex_disk" {
                            ParkedYandexConnectionCard(
                                entry: entry,
                                onRemove: { connections.remove(entry.id) },
                                onRename: { connections.rename(entry.id, name: $0) },
                                onSetEnabled: { connections.setEnabled(entry.id, enabled: $0) }
                            )
                            .id(entry.id + (entry.enabled ? ":enabled" : ":disabled"))
                        } else if entry.adapter == "local_filesystem" {
                            LocalConnectionCard(
                                entry: entry,
                                onRemove: { connections.remove(entry.id) },
                                onRename: { connections.rename(entry.id, name: $0) },
                                onSetEnabled: { connections.setEnabled(entry.id, enabled: $0) }
                            )
                        } else {
                            LabeledContent("Адаптер", value: entry.adapter)
                            Text("Недоступно в активном Release 1; QADREN не выполняет для него I/O.")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    } label: {
                        Label(entry.name, systemImage: "externaldrive")
                    }
                }
                Text("Удаление меняет только локальную запись QADREN и её credential; объекты в облаке не удаляются.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            .frame(maxWidth: 900, alignment: .leading)
            .padding(24)
        }
    }
}

/// D-018 parks Yandex work through P2. Existing local records and Keychain
/// credentials remain manageable, but this screen must not offer OAuth or API
/// calls as a current local-release capability.
private struct ParkedYandexConnectionCard: View {
    let entry: StorageEntry
    let onRemove: () -> Void
    let onRename: (String) -> Void
    let onSetEnabled: (Bool) -> Void
    @State private var name: String
    @State private var confirmRemoval = false

    init(entry: StorageEntry, onRemove: @escaping () -> Void, onRename: @escaping (String) -> Void, onSetEnabled: @escaping (Bool) -> Void) {
        self.entry = entry
        self.onRemove = onRemove
        self.onRename = onRename
        self.onSetEnabled = onSetEnabled
        _name = State(initialValue: entry.name)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Яндекс Диск · подключение " + String(entry.id.prefix(8)))
                .font(.caption).foregroundStyle(.secondary)
            HStack {
                TextField("Название подключения", text: $name)
                Button("Переименовать") { onRename(name) }
                    .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || name == entry.name)
                Toggle("Включено", isOn: Binding(get: { entry.enabled }, set: { onSetEnabled($0) }))
            }
            SettingsNotice(
                text: "Интеграция Яндекс Диска отложена до P3. QADREN сохраняет локальную запись и Keychain credential, но не запускает OAuth или API-вызовы из этого окна.",
                symbol: "pause.circle"
            )
            Button("Удалить локальное подключение", role: .destructive) { confirmRemoval = true }
        }
        .confirmationDialog("Удалить это локальное подключение QADREN?", isPresented: $confirmRemoval) {
            Button("Удалить подключение", role: .destructive, action: onRemove)
        } message: {
            Text("Будут удалены только локальная запись QADREN и её Keychain credential. Файлы на Яндекс Диске не изменятся.")
        }
    }
}

private struct LocalConnectionCard: View {
    let entry: StorageEntry
    let onRemove: () -> Void
    let onRename: (String) -> Void
    let onSetEnabled: (Bool) -> Void
    @State private var name: String
    @State private var confirmRemoval = false

    init(entry: StorageEntry, onRemove: @escaping () -> Void, onRename: @escaping (String) -> Void, onSetEnabled: @escaping (Bool) -> Void) {
        self.entry = entry; self.onRemove = onRemove; self.onRename = onRename; self.onSetEnabled = onSetEnabled
        _name = State(initialValue: entry.name)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Локальное подключение · " + String(entry.id.prefix(8)))
                .font(.caption).foregroundStyle(.secondary)
            HStack {
                TextField("Название подключения", text: $name)
                Button("Переименовать") { onRename(name) }
                    .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || name == entry.name)
                Toggle("Включено", isOn: Binding(get: { entry.enabled }, set: { onSetEnabled($0) }))
            }
            Text("Корневая папка выбирается в редакторе сценария. До запуска сценария QADREN не передаёт и не удаляет файлы.")
                .font(.caption).foregroundStyle(.secondary)
            Button("Удалить", role: .destructive) { confirmRemoval = true }
        }
        .confirmationDialog("Удалить локальное подключение QADREN?", isPresented: $confirmRemoval) {
            Button("Удалить подключение", role: .destructive, action: onRemove)
        } message: {
            Text("Будет удалена только локальная запись QADREN. Файлы на этом Mac не изменятся.")
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
                    detail: "Выберите источники, каталоги и направление обмена. Планирование и запуск появятся после готовности Core.",
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
        connections.filter { $0.enabled && $0.adapter == "local_filesystem" }
    }

    private func isLocalScenario(_ scenario: ScenarioEntry) -> Bool {
        let endpointIDs = [scenario.left_connection_id, scenario.right_connection_id]
        return endpointIDs.allSatisfy { id in
            connections.first(where: { $0.id == id })?.adapter == "local_filesystem"
        }
    }

    private func scenarioStatus(_ scenario: ScenarioEntry) -> String {
        if !isLocalScenario(scenario) {
            return "Сценарий использует Яндекс Диск и отложен до P3. Его запуск, изменение и расписания недоступны; запись сохранена локально."
        }
        return scenario.enabled
            ? "Можно проверить план только по метаданным. Запуск и передача файлов пока недоступны."
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
    @State private var pendingCreate: ScenarioPlanItem?
    @State private var pendingUpdate: ScenarioPlanItem?
    @State private var conflictTasks: [ScenarioConflictTask] = []
    @State private var leftSnapshot: [ScenarioSnapshotItem] = []
    @State private var rightSnapshot: [ScenarioSnapshotItem] = []
    @State private var pendingConflictReplace: ScenarioConflictTask?

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
            } else if plan.isEmpty {
                SettingsEmptyState(
                    title: "Изменения не найдены",
                    detail: "По доступным метаданным план не содержит действий. Это не является проверкой равенства содержимого файлов.",
                    symbol: "checkmark.circle"
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(plan) { item in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text(item.relative_path).font(.body.monospaced())
                            Spacer()
                            Text(actionTitle(item.action)).font(.caption.weight(.semibold))
                        }
                        Text("Источник: \(sideTitle(item.left)) · Назначение: \(sideTitle(item.right))")
                            .font(.caption).foregroundStyle(.secondary)
                        if let reason = item.action.reason {
                            Text(reason).font(.caption).foregroundStyle(.orange)
                        }
                        if item.action.kind == "create" {
                            Button("Создать копию") { pendingCreate = item }
                                .buttonStyle(.bordered)
                        }
                        if item.action.kind == "update" {
                            Button("Заменить файл", role: .destructive) { pendingUpdate = item }
                                .buttonStyle(.bordered)
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
                            executeReplace: { pendingConflictReplace = $0 }
                        )
                    }
                }
            }
        }
        .padding(24)
        .onAppear { loadConflictTasks() }
        .alert("Разрешить загрузку и создание копии?", isPresented: Binding(
            get: { pendingCreate != nil }, set: { if !$0 { pendingCreate = nil } }
        )) {
            Button("Отмена", role: .cancel) { pendingCreate = nil }
            Button("Загрузить и создать") { executePendingCreate() }
        } message: {
            Text("macOS может скачать исходный файл из File Provider на этот Mac. QADREN создаст только этот файл, не заменит файл назначения и сохранит Job с точной ревизией сценария.")
        }
        .alert("Разрешить загрузку и замену файла?", isPresented: Binding(
            get: { pendingUpdate != nil }, set: { if !$0 { pendingUpdate = nil } }
        )) {
            Button("Отмена", role: .cancel) { pendingUpdate = nil }
            Button("Загрузить и заменить", role: .destructive) { executePendingUpdate() }
        } message: {
            Text("macOS может скачать исходный файл из File Provider на этот Mac. QADREN заменит только этот существующий файл назначения и сохранит Job с точной ревизией сценария.")
        }
        .alert("Выполнить решение конфликта?", isPresented: Binding(
            get: { pendingConflictReplace != nil }, set: { if !$0 { pendingConflictReplace = nil } }
        )) {
            Button("Отмена", role: .cancel) { pendingConflictReplace = nil }
            Button(
                pendingConflictReplace?.decision == "replace_if_newer" ? "Проверить и заменить" : "Загрузить и заменить",
                role: .destructive
            ) { executePendingConflictReplace() }
        } message: {
            Text(
                pendingConflictReplace?.decision == "replace_if_newer"
                    ? "QADREN ещё раз проверит обе стороны. Он заменит файл только если источник новее. При изменении задача станет устаревшей, а замена не начнётся."
                    : "QADREN ещё раз проверит обе стороны по сохранённому снимку. При изменении задача станет устаревшей, а замена не начнётся."
            )
        }
    }

    private func loadPlan() {
        isLoading = true
        error = nil
        defer { isLoading = false; hasRequestedPreview = true }
        do {
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
            self.error = "Не удалось прочитать метаданные или построить план: " + error.localizedDescription
        }
    }

    private func queueConflict(_ item: ScenarioPlanItem) {
        let kind = item.action.kind == "conflict" ? "content" : "deletion"
        do {
            _ = try StorageRegistryBridge.queueScenarioConflict(
                scenario: scenario, relativePath: item.relative_path, kind: kind,
                left: leftSnapshot.first { $0.relative_path == item.relative_path },
                right: rightSnapshot.first { $0.relative_path == item.relative_path }
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

    private func executePendingCreate() {
        guard let item = pendingCreate else { return }
        pendingCreate = nil
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
            try StorageRegistryBridge.executeScenarioUpdate(scenario: scenario, item: item)
            loadPlan()
        } catch {
            self.error = "Не удалось обновить файл по сценарию: " + error.localizedDescription
        }
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

    private func actionTitle(_ action: ScenarioPlanAction) -> String {
        switch action.kind {
        case "create": return action.from_left == true ? "создать справа" : "создать слева"
        case "update": return action.from_left == true ? "обновить справа" : "обновить слева"
        case "delete_requires_decision": return "нужно отдельное решение"
        case "conflict": return "конфликт"
        case "needs_verification": return "нужна проверка"
        default: return "без действия"
        }
    }
}

private struct ScenarioConflictTaskView: View {
    let scenario: ScenarioEntry
    let task: ScenarioConflictTask
    let saveDecision: (ScenarioConflictTask, Bool, String) -> Void
    let executeReplace: (ScenarioConflictTask) -> Void
    @State private var sourceIsLeft: Bool?

    init(
        scenario: ScenarioEntry,
        task: ScenarioConflictTask,
        saveDecision: @escaping (ScenarioConflictTask, Bool, String) -> Void,
        executeReplace: @escaping (ScenarioConflictTask) -> Void
    ) {
        self.scenario = scenario
        self.task = task
        self.saveDecision = saveDecision
        self.executeReplace = executeReplace
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
                if task.kind == "content" {
                    HStack {
                        Button("Заменить из источника", role: .destructive) {
                            save("replace_from_source")
                        }
                        Button("Заменить, если новее") { save("replace_if_newer") }
                        Button("Пропустить перенос") { save("skip_transfer") }
                    }
                    .disabled(sourceIsLeft == nil)
                } else {
                    HStack {
                        Button("Удалить как в источнике", role: .destructive) {
                            save("delete_as_source")
                        }
                        Button("Не удалять файл") { save("keep_file") }
                    }
                    .disabled(sourceIsLeft == nil)
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
                        Text("Это read-only расчёт по сохранённым настройкам. Запуск и подтверждение кандидата пока не реализованы.")
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
    let saveScenario: (String, String, String, String, String, String) -> String?
    let cancel: () -> Void
    @State private var name = ""
    @State private var leftID = ""
    @State private var leftRoot = "/"
    @State private var rightID = ""
    @State private var rightRoot = "/"
    @State private var direction = "left_to_right"
    @State private var error: String?

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .firstTextBaseline) {
                Button("Назад", systemImage: "chevron.left", action: cancel)
                VStack(alignment: .leading, spacing: 3) {
                    Text(scenario == nil ? "Новый сценарий" : "Изменить сценарий").font(.largeTitle.bold())
                    Text("Изменения создают новую ревизию только для будущих заданий.")
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button(scenario == nil ? "Создать" : "Сохранить", action: save)
                    .keyboardShortcut(.defaultAction).disabled(!canCreate)
            }
            .padding(24)

            Form {
                TextField("Название сценария", text: $name)
                Picker("Тип синхронизации", selection: $direction) {
                    Text("Односторонняя").tag("left_to_right")
                    Text("Двусторонняя").tag("bidirectional")
                }
                Text("Создание сохранит только конфигурацию и ревизию 1. Запуск, расписания и передача файлов не выполняются.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            .formStyle(.grouped)
            .padding(.horizontal, 24)

            HStack(spacing: 0) {
                ScenarioEndpointPicker(
                    role: "Источник", connections: connections,
                    selection: $leftID, root: $leftRoot
                )
                Divider()
                ScenarioEndpointPicker(
                    role: "Назначение", connections: connections,
                    selection: $rightID, root: $rightRoot
                )
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(24)
            if let error { Text(error).font(.caption).foregroundStyle(.red).padding([.horizontal, .bottom], 24) }
        }
        .onAppear {
            if let scenario {
                name = scenario.name
                leftID = scenario.left_connection_id
                leftRoot = scenario.left_root
                rightID = scenario.right_connection_id
                rightRoot = scenario.right_root
                direction = scenario.direction
            } else if let first = connections.first {
                leftID = first.id
                rightID = first.id
            }
        }
    }

    private var canCreate: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !leftID.isEmpty && !rightID.isEmpty
            && !leftRoot.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !rightRoot.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && (leftID != rightID || leftRoot != rightRoot)
    }
    private func save() {
        error = saveScenario(name, leftID, leftRoot, rightID, rightRoot, direction)
    }
}

private struct ScenarioEndpointPicker: View {
    let role: String
    let connections: [StorageEntry]
    @Binding var selection: String
    @Binding var root: String
    @State private var items: [DiskItem] = []
    @State private var nextOffset: UInt64?
    @State private var loading = false
    @State private var error: String?
    @StateObject private var localWorkspace = LocalWorkspace()

    private var connection: StorageEntry? { connections.first { $0.id == selection } }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(role).font(.headline)
            Picker(role, selection: $selection) {
                ForEach(connections) { entry in
                    Text(entry.name + " · " + adapterTitle(entry.adapter)).tag(entry.id)
                }
            }
            .labelsHidden()
            .onChange(of: selection) { _ in root = "/"; items = []; nextOffset = nil; error = nil; loadDirectory() }
            HStack {
                Text(root).font(.caption.monospaced()).lineLimit(1).help(root)
                Spacer()
                if connection?.adapter == "local_filesystem" {
                    Button("Выбрать папку…") { chooseLocalFolder() }
                } else {
                    Button("Обновить", systemImage: "arrow.clockwise") { loadDirectory() }.disabled(loading)
                }
            }
            if connection?.adapter == "local_filesystem" {
                LocalScenarioFolderBrowser(workspace: localWorkspace, root: $root, choose: chooseLocalFolder)
            } else if loading {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let error {
                SettingsEmptyState(title: "Каталог недоступен", detail: error, symbol: "exclamationmark.triangle")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(items) { item in
                    Button {
                        guard item.kind == "dir" else { return }
                        root = item.path
                        loadDirectory()
                    } label: {
                        HStack {
                            Image(systemName: item.kind == "dir" ? "folder" : "doc")
                            Text(item.name).lineLimit(1)
                            Spacer()
                            if item.kind == "dir" { Image(systemName: "chevron.right").foregroundStyle(.secondary) }
                        }
                    }
                    .buttonStyle(.plain).disabled(item.kind != "dir")
                }
                .listStyle(.inset)
                if let nextOffset {
                    Button("Показать ещё", systemImage: "ellipsis") { loadDirectory(offset: nextOffset) }
                        .disabled(loading)
                        .accessibilityHint("Запрашивает следующую страницу метаданных каталога без загрузки содержимого файлов")
                }
            }
        }
        .padding(16)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .task(id: selection) { loadDirectory() }
    }

    private func chooseLocalFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false; panel.canChooseDirectories = true; panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        localWorkspace.selectRoot(url.path)
        root = url.path
    }
    private func loadDirectory(offset: UInt64 = 0) {
        guard let connection, connection.adapter == "yandex_disk" else { return }
        guard let token = (try? YandexKeychain.read(connection.id)) ?? nil else {
            error = "Подключите Яндекс Диск, чтобы просмотреть только метаданные каталогов."; return
        }
        if offset == 0 { items = []; nextOffset = nil }
        loading = true; error = nil
        let folder = root == "/" ? "disk:/" : root
        Task.detached {
            let result = Result { try DiskMetadataBridge.folderPage(accessValue: token, path: folder, offset: offset) }
            await MainActor.run {
                loading = false
                switch result {
                case .success(let listing):
                    items = offset == 0 ? listing.items : items + listing.items
                    let consumed = listing.offset.addingReportingOverflow(UInt64(listing.items.count))
                    nextOffset = !listing.items.isEmpty && !consumed.overflow && consumed.partialValue < listing.total ? consumed.partialValue : nil
                case .failure(let failure):
                    error = failure.localizedDescription
                }
            }
        }
    }
    private func adapterTitle(_ adapter: String) -> String { adapter == "yandex_disk" ? "Яндекс Диск" : "Этот Mac" }
}

private struct LocalScenarioFolderBrowser: View {
    @ObservedObject var workspace: LocalWorkspace
    @Binding var root: String
    let choose: () -> Void

    var body: some View {
        if workspace.root == nil {
            VStack(spacing: 8) {
                Image(systemName: "laptopcomputer").font(.system(size: 28)).foregroundStyle(.secondary)
                Text("Этот Mac").font(.headline)
                Text("Выберите локальную папку, затем можно перейти глубже по её структуре.")
                    .font(.caption).foregroundStyle(.secondary).multilineTextAlignment(.center)
                Button("Выбрать локальную папку", action: choose)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Button("Вверх", systemImage: "chevron.up") {
                        workspace.up(); root = workspace.selectedDirectory ?? root
                    }
                    .disabled(workspace.relativePath.isEmpty)
                    Text(workspace.selectedDirectory ?? root).font(.caption.monospaced()).lineLimit(1)
                    Spacer()
                    Button("Другая папка…", action: choose)
                }
                List(workspace.entries) { entry in
                    Button {
                        guard entry.is_directory else { return }
                        workspace.open(entry)
                        root = workspace.selectedDirectory ?? root
                    } label: {
                        HStack {
                            Image(systemName: entry.is_directory ? "folder" : "doc")
                            Text(entry.name)
                            Spacer()
                            if entry.is_directory { Image(systemName: "chevron.right").foregroundStyle(.secondary) }
                        }
                    }
                    .buttonStyle(.plain).disabled(!entry.is_directory)
                }
                .listStyle(.inset)
                if let error = workspace.error { Text(error).font(.caption).foregroundStyle(.orange) }
            }
        }
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
                LabeledContent("Выполнение сценариев", value: "Недоступно до готовности Core")
            }
        }
        .formStyle(.grouped)
        .padding(20)
        .navigationTitle("Настройки приложения")
    }
}
