import SwiftUI

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
    @State private var isCreatingScenario = false

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
                    ScenariosSettingsView(
                        scenarios: scenarios,
                        connections: connections.entries,
                        createScenario: { isCreatingScenario = true }
                    )
                case .application:
                    ApplicationSettingsView()
                }
            }
            .frame(minWidth: 560, minHeight: 500)
        }
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                if (selection ?? .connections) == .connections {
                    Button("Добавить подключение", systemImage: "plus") { connections.addYandex() }
                } else {
                    Button("Создать сценарий", systemImage: "plus") { isCreatingScenario = true }
                        .disabled(scenarioEndpoints.isEmpty)
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
        .sheet(isPresented: $isCreatingScenario) {
            ScenarioCreationSheet(
                connections: scenarioEndpoints,
                onCreate: { name, leftID, leftRoot, rightID, rightRoot, direction in
                    scenarios.create(
                        name: name,
                        leftID: leftID,
                        leftRoot: leftRoot,
                        rightID: rightID,
                        rightRoot: rightRoot,
                        direction: direction
                    )
                }
            )
        }
    }

    private var scenarioEndpoints: [StorageEntry] {
        connections.entries.filter {
            $0.enabled && ($0.adapter == "yandex_disk" || $0.adapter == "local_filesystem")
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
                        detail: "Добавьте Яндекс Диск, чтобы сохранить отдельную точку синхронизации.",
                        symbol: "externaldrive.badge.plus"
                    )
                    .frame(maxWidth: .infinity, minHeight: 280)
                }
                ForEach(connections.entries) { entry in
                    GroupBox {
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

private struct ScenariosSettingsView: View {
    @ObservedObject var scenarios: ScenarioSettings
    let connections: [StorageEntry]
    let createScenario: () -> Void

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
                        Text(scenario.enabled
                             ? "Автоматизация и запуск ещё недоступны: Core пока не создаёт Job из Scenario."
                             : "Автоматические триггеры для этого сценария выключены.")
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
        connections.filter {
            $0.enabled && ($0.adapter == "yandex_disk" || $0.adapter == "local_filesystem")
        }
    }

    private func adapterTitle(_ adapter: String) -> String {
        adapter == "yandex_disk" ? "Яндекс Диск" : adapter
    }

    private func directionTitle(_ direction: String) -> String {
        switch direction {
        case "left_to_right": "Источник → назначение"
        case "right_to_left": "Назначение → источник"
        case "bidirectional": "Двусторонний"
        default: direction
        }
    }
}

private struct ScenarioCreationSheet: View {
    let connections: [StorageEntry]
    let onCreate: (String, String, String, String, String, String) -> String?
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var leftID = ""
    @State private var leftRoot = "/"
    @State private var rightID = ""
    @State private var rightRoot = "/"
    @State private var direction = "left_to_right"
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Новый сценарий обмена").font(.title2.bold())
            Form {
                TextField("Название", text: $name)
                Picker("Источник", selection: $leftID) {
                    ForEach(connections) { entry in
                        Text(entry.name + " · " + adapterTitle(entry.adapter)).tag(entry.id)
                    }
                }
                TextField("Каталог источника", text: $leftRoot)
                Picker("Назначение", selection: $rightID) {
                    ForEach(connections) { entry in
                        Text(entry.name + " · " + adapterTitle(entry.adapter)).tag(entry.id)
                    }
                }
                TextField("Каталог назначения", text: $rightRoot)
                Picker("Направление", selection: $direction) {
                    Text("Источник → назначение").tag("left_to_right")
                    Text("Назначение → источник").tag("right_to_left")
                    Text("Двусторонний").tag("bidirectional")
                }
            }
            Text("Создание сохраняет только конфигурацию и ревизию 1. Запуск, расписания и передача файлов не выполняются.")
                .font(.caption).foregroundStyle(.secondary)
            if let error {
                Text(error).font(.caption).foregroundStyle(.red)
            }
            HStack {
                Spacer()
                Button("Отмена") { dismiss() }
                Button("Создать") {
                    if let failure = onCreate(name, leftID, leftRoot, rightID, rightRoot, direction) {
                        error = failure
                    } else {
                        dismiss()
                    }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(!canCreate)
            }
        }
        .padding(24)
        .frame(width: 520)
        .onAppear {
            guard let first = connections.first else { return }
            leftID = first.id
            rightID = first.id
        }
    }

    private var canCreate: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !leftID.isEmpty
            && !rightID.isEmpty
            && !leftRoot.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !rightRoot.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && (leftID != rightID || leftRoot != rightRoot)
    }

    private func adapterTitle(_ adapter: String) -> String {
        adapter == "yandex_disk" ? "Яндекс Диск" : adapter
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
