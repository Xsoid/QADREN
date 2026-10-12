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
                    if isEditingScenario {
                        ScenarioEditorView(
                            connections: scenarioEndpoints,
                            createScenario: createScenario,
                            cancel: { isEditingScenario = false }
                        )
                    } else {
                        ScenariosSettingsView(
                            scenarios: scenarios,
                            connections: connections.entries,
                            createScenario: { isEditingScenario = true }
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
                    Menu("Добавить подключение", systemImage: "plus") {
                        Button("Яндекс Диск") { connections.addYandex() }
                        Button("Этот Mac") { connections.addLocalComputer() }
                    }
                case .scenarios:
                    Button("Создать сценарий", systemImage: "plus") { isEditingScenario = true }
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
            $0.enabled && ($0.adapter == "yandex_disk" || $0.adapter == "local_filesystem")
        }
        if !endpoints.contains(where: { $0.adapter == "local_filesystem" }) {
            endpoints.append(StorageEntry(
                id: "local-computer", adapter: "local_filesystem", name: "Этот Mac",
                enabled: true, status: "connected", credential_ref: nil, provider_account_id: nil
            ))
        }
        return endpoints
    }

    private func createScenario(
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
            let result = scenarios.create(
                name: name, leftID: leftID == "local-computer" ? localID! : leftID, leftRoot: leftRoot,
                rightID: rightID == "local-computer" ? localID! : rightID, rightRoot: rightRoot, direction: direction
            )
            if result == nil {
                connections.refresh()
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
                    detail: "Добавьте Яндекс Диск или этот Mac как отдельную точку синхронизации.",
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

private struct ScenarioEditorView: View {
    let connections: [StorageEntry]
    let createScenario: (String, String, String, String, String, String) -> String?
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
                    Text("Новый сценарий").font(.largeTitle.bold())
                    Text("Сначала настройте точки обмена и направление, затем выберите каталоги.")
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button("Создать", action: save).keyboardShortcut(.defaultAction).disabled(!canCreate)
            }
            .padding(24)

            Form {
                TextField("Название сценария", text: $name)
                Picker("Тип синхронизации", selection: $direction) {
                    Text("Источник → назначение").tag("left_to_right")
                    Text("Назначение → источник").tag("right_to_left")
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
            if let first = connections.first {
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
        error = createScenario(name, leftID, leftRoot, rightID, rightRoot, direction)
    }
}

private struct ScenarioEndpointPicker: View {
    let role: String
    let connections: [StorageEntry]
    @Binding var selection: String
    @Binding var root: String
    @State private var items: [DiskItem] = []
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
            .onChange(of: selection) { _ in root = "/"; items = []; error = nil; loadDirectory() }
            HStack {
                Text(root).font(.caption.monospaced()).lineLimit(1).help(root)
                Spacer()
                if connection?.adapter == "local_filesystem" {
                    Button("Выбрать папку…") { chooseLocalFolder() }
                } else {
                    Button("Обновить", systemImage: "arrow.clockwise", action: loadDirectory).disabled(loading)
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
    private func loadDirectory() {
        guard let connection, connection.adapter == "yandex_disk" else { return }
        guard let token = (try? YandexKeychain.read(connection.id)) ?? nil else {
            error = "Подключите Яндекс Диск, чтобы просмотреть только метаданные каталогов."; return
        }
        loading = true; error = nil
        let folder = root == "/" ? "disk:/" : root
        Task.detached {
            let result = Result { try DiskMetadataBridge.folder(token: token, path: folder) }
            await MainActor.run {
                loading = false
                switch result { case .success(let listing): items = listing.items; case .failure(let failure): error = failure.localizedDescription }
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
