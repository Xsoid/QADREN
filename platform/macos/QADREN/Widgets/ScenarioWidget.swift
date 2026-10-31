import SwiftUI

/// File counts of one batch for the progress ring. Unknown results count as
/// errors: they need reconciliation and are not proven copies. A file that
/// waits for a retry after a failed transfer (for example a Yandex Disk 500)
/// also counts as an error until a retry copies it.
struct ScenarioFileCounts: Equatable {
    let total: Int
    let copied: Int
    let errors: Int
    /// Errors that the Worker retries later in this run.
    let retrying: Int
    let nextRetryAt: Date?
    var remaining: Int { max(0, total - copied - errors) }

    init(_ summary: ScenarioBatchSummary) {
        total = summary.total
        copied = summary.completed
        retrying = summary.retry_waiting
        errors = summary.failed + summary.unknown + summary.retry_waiting
        nextRetryAt = summary.next_retry_at.map { Date(timeIntervalSince1970: TimeInterval($0)) }
    }
}

/// One read of a Scenario for the widget. Every value comes from the local
/// Registry; nothing here starts a run or reads file content.
struct ScenarioWidgetSnapshot: Equatable {
    struct Choice: Equatable, Identifiable {
        let id: String
        let name: String
    }

    /// One side of the Scenario: its connection and root.
    struct Endpoint: Equatable {
        let connection: String
        let kind: String
        let root: String
        /// Selected objects inside the root; 0 means the whole root.
        let selected: Int
    }

    var choices: [Choice] = []
    var name: String?
    /// One-way Scenarios: `from` is the source. Two-way: the two sides.
    var from: Endpoint?
    var to: Endpoint?
    var twoWay = false
    var enabled = false
    var found = false
    var run: ScenarioRunState?
    var files: ScenarioFileCounts?
    var batchPaused = false
    var batchStopping = false
    var currentPath: String?
    var nextRunAt: Date?
    var hasSchedule = false
    var schedulePaused = false
    var error: String?

    static func capture(scenarioID: String?) -> (ScenarioWidgetSnapshot, ScenarioEntry?) {
        var snapshot = ScenarioWidgetSnapshot()
        let scenarios: [ScenarioEntry]
        do {
            scenarios = try StorageRegistryBridge.listScenarios()
        } catch {
            snapshot.error = error.localizedDescription
            return (snapshot, nil)
        }
        snapshot.choices = scenarios.map { Choice(id: $0.id, name: $0.name) }
        guard let scenarioID, let entry = scenarios.first(where: { $0.id == scenarioID }) else {
            return (snapshot, nil)
        }
        snapshot.found = true
        snapshot.name = entry.name
        snapshot.enabled = entry.enabled
        let connections = (try? StorageRegistryBridge.list()) ?? []
        func endpoint(_ connectionID: String, _ root: String, selected: Int) -> Endpoint {
            let connection = connections.first { $0.id == connectionID }
            return Endpoint(
                connection: connection?.name ?? "Подключение не найдено",
                kind: connection.flatMap { StorageConnector(adapter: $0.adapter)?.title } ?? "",
                root: root,
                selected: selected
            )
        }
        // The selection limits what the source side offers.
        let reversed = entry.direction == "right_to_left"
        let selected = entry.selected_paths.count
        let left = endpoint(entry.left_connection_id, entry.left_root, selected: reversed ? 0 : selected)
        let right = endpoint(entry.right_connection_id, entry.right_root, selected: reversed ? selected : 0)
        snapshot.twoWay = entry.direction == "bidirectional"
        (snapshot.from, snapshot.to) = reversed ? (right, left) : (left, right)
        do {
            let run = try StorageRegistryBridge.scenarioRunState(scenarioID: scenarioID)
            snapshot.run = run
            // An active run shows its own batch; otherwise the last result.
            let batchID = run.active != nil ? run.active?.batch_id : run.last?.batch_id
            if let batchID, let summary = try? StorageRegistryBridge.scenarioCreateBatchSummary(id: batchID) {
                snapshot.files = ScenarioFileCounts(summary)
                snapshot.batchPaused = summary.paused
                snapshot.batchStopping = summary.cancellation_requested
                snapshot.currentPath = summary.current_path
            }
            let schedules = try StorageRegistryBridge.listScenarioSchedules(scenarioID: scenarioID).filter(\.enabled)
            snapshot.hasSchedule = !schedules.isEmpty
            let parser = ISO8601DateFormatter()
            snapshot.nextRunAt = schedules.compactMap { $0.next_at.flatMap(parser.date(from:)) }.min()
            snapshot.schedulePaused = (try? StorageRegistryBridge.scenarioAutomation(scenarioID: scenarioID))?.locally_paused ?? false
        } catch {
            snapshot.error = error.localizedDescription
        }
        return (snapshot, entry)
    }
}

/// Periodic read-only feed for one Scenario widget.
@MainActor
final class ScenarioWidgetFeed: ObservableObject {
    @Published private(set) var snapshot = ScenarioWidgetSnapshot()
    @Published private(set) var loaded = false
    /// Minutes without a change of the copied and failed counts or of the
    /// current file during an executing run; `nil` while progress moves.
    @Published private(set) var stalledMinutes: Int?
    private(set) var entry: ScenarioEntry?
    private var progressKey: String?
    private var progressChanged = Date()
    static let stallThreshold: TimeInterval = 180

    func follow(scenarioID: String?) async {
        while !Task.isCancelled {
            await refresh(scenarioID: scenarioID)
            try? await Task.sleep(for: .seconds(3))
        }
    }

    func refresh(scenarioID: String?) async {
        let (snapshot, entry) = await Task.detached(priority: .utility) {
            ScenarioWidgetSnapshot.capture(scenarioID: scenarioID)
        }.value
        self.snapshot = snapshot
        self.entry = entry
        loaded = true
        let executing = snapshot.run?.active?.state == "executing" && !snapshot.batchPaused
        let key = executing
            ? "\(snapshot.files?.copied ?? -1)|\(snapshot.files?.errors ?? -1)|\(snapshot.currentPath ?? "")"
            : nil
        let now = Date()
        if key != progressKey {
            progressKey = key
            progressChanged = now
        }
        let idle = now.timeIntervalSince(progressChanged)
        stalledMinutes = key != nil && idle >= Self.stallThreshold ? Int(idle / 60) : nil
    }
}

/// The stage of the Scenario: what the large status line says.
struct ScenarioStage: Equatable {
    /// Colors the frame. Only a failed run or a required user action does
    /// that; failed files never turn the whole widget red.
    let frame: QDStatusKind
    /// Colors the status line.
    let tone: QDStatusKind
    let headline: String
    let detail: String
    let active: Bool
    /// The Worker reports work in progress now (not waiting, not paused).
    var moving = false

    static func evaluate(_ snapshot: ScenarioWidgetSnapshot) -> ScenarioStage {
        if let active = snapshot.run?.active {
            if let reason = active.waiting_reason {
                let detail = switch reason {
                case "needs_sign_in": "Нужен вход в Яндекс: откройте QADREN."
                case "waiting_for_scenario":
                    active.waiting_for_scenario.map { "Ждёт завершения сценария «\($0)»." } ?? QADRENUserMessage.text(for: reason)
                default: QADRENUserMessage.text(for: reason)
                }
                let action = reason == "needs_sign_in"
                return ScenarioStage(frame: action ? .needsAction : .waiting, tone: action ? .needsAction : .waiting, headline: "Ожидание", detail: detail, active: true)
            }
            let trigger = active.trigger == "schedule" ? "по расписанию" : "вручную"
            if active.state == "planning" {
                return ScenarioStage(frame: .active, tone: .active, headline: "Планирование", detail: "Снимки и план, запуск \(trigger).", active: true, moving: true)
            }
            if snapshot.batchStopping {
                return ScenarioStage(frame: .waiting, tone: .waiting, headline: "Остановка", detail: "Текущий файл завершается, новые не начинаются.", active: true, moving: true)
            }
            if snapshot.batchPaused {
                return ScenarioStage(frame: .paused, tone: .paused, headline: "Пауза", detail: "Новые файлы не начинаются.", active: true)
            }
            return ScenarioStage(frame: .active, tone: .active, headline: "Копирование", detail: snapshot.currentPath ?? "Запуск \(trigger).", active: true, moving: true)
        }
        if !snapshot.enabled {
            return ScenarioStage(frame: .offline, tone: .offline, headline: "Выключен", detail: "Сценарий выключен в настройках.", active: false)
        }
        guard let last = snapshot.run?.last else {
            return ScenarioStage(frame: .offline, tone: .offline, headline: "Не запускался", detail: "Запусков этого сценария ещё не было.", active: false)
        }
        let finished = Date(timeIntervalSince1970: TimeInterval(last.finished_at ?? last.created_at))
        let when = "Завершён " + Self.shortDate(finished) + (last.trigger == "schedule" ? " · по расписанию" : " · вручную")
        let fileErrors = (snapshot.files?.errors ?? 0) > 0
        switch last.state {
        case "completed":
            return fileErrors
                ? ScenarioStage(frame: .ready, tone: .needsAction, headline: "Готово с ошибками", detail: when, active: false)
                : ScenarioStage(frame: .ready, tone: .ready, headline: "Готово", detail: when, active: false)
        case "cancelled":
            return ScenarioStage(frame: .paused, tone: .paused, headline: "Остановлен", detail: when, active: false)
        default:
            // File failures end a run as `items_failed` or `items_unknown`.
            // They are copy errors, not a failed Scenario.
            if last.error == "items_failed" || last.error == "items_unknown" {
                return ScenarioStage(frame: .ready, tone: .needsAction, headline: "Готово с ошибками", detail: when, active: false)
            }
            let reason = last.error.map(QADRENUserMessage.text(for:)) ?? "Запуск завершился ошибкой."
            return ScenarioStage(frame: .failed, tone: .failed, headline: "Сбой", detail: reason, active: false)
        }
    }

    static func shortDate(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = Calendar.current.isDateInToday(date) ? "HH:mm" : "dd.MM HH:mm"
        return formatter.string(from: date)
    }
}

/// `scenario`: one selected Scenario. Compact shows the stage and the ring;
/// regular adds file counts, the next run, and run controls.
struct ScenarioWidget<Actions: View>: View {
    let widget: MonitoringWidgetLayout
    let isEditing: Bool
    let onSelectScenario: (String) -> Void
    @ViewBuilder let actions: Actions
    @StateObject private var feed = ScenarioWidgetFeed()
    @State private var actionError: String?
    @State private var busy = false
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let snapshot = feed.snapshot
        let stage = ScenarioStage.evaluate(snapshot)
        QDWidgetFrame(
            title: snapshot.name ?? MonitoringWidgetKind.scenario.title,
            symbol: MonitoringWidgetKind.scenario.symbol,
            status: snapshot.found ? stage.frame : nil,
            actions: {
                if isEditing { scenarioMenu(snapshot) }
                actions
            }
        ) {
            if widget.scenarioID == nil || (feed.loaded && !snapshot.found) {
                selectionPrompt(snapshot)
            } else if feed.loaded {
                content(snapshot, stage)
            } else {
                Color.clear.frame(height: 96)
            }
        }
        .task(id: widget.scenarioID) {
            await feed.follow(scenarioID: widget.scenarioID)
        }
    }

    @ViewBuilder
    private func content(_ snapshot: ScenarioWidgetSnapshot, _ stage: ScenarioStage) -> some View {
        let compact = widget.density == .compact
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .center, spacing: 14) {
                // Planning has no file total yet: the ring shows the track only.
                let files = snapshot.run?.active?.state == "planning" ? nil : snapshot.files
                QDCyberRing(
                    total: files?.total, done: files?.copied ?? 0, failed: files?.errors ?? 0,
                    size: compact ? 84 : 104, active: stage.moving, stalled: feed.stalledMinutes != nil
                )
                VStack(alignment: .leading, spacing: 4) {
                    QDCyberHeadline(text: stage.headline, color: stage.tone.cyberColor(for: colorScheme), size: compact ? 24 : 28)
                    Text(stage.detail)
                        .font(QDCyber.heading(13, weight: .medium))
                        .foregroundStyle(QDCyber.color(.secondaryText, for: colorScheme))
                        .lineLimit(compact ? 2 : 3)
                        .truncationMode(.middle)
                    if stage.moving {
                        QDCyberActivityBar(stalled: feed.stalledMinutes != nil)
                            .padding(.top, 4)
                    }
                    if let minutes = feed.stalledMinutes {
                        Text("Нет прогресса \(minutes) мин")
                            .font(QDCyber.heading(13))
                            .textCase(.uppercase)
                            .foregroundStyle(QDStatusKind.needsAction.cyberColor(for: colorScheme))
                    }
                }
            }
            if !compact {
                regularDetails(snapshot, stage)
            }
            if widget.density == .expanded {
                endpoints(snapshot)
            }
            if let message = actionError ?? snapshot.error {
                Text(message)
                    .font(QDCyber.heading(13, weight: .medium))
                    .foregroundStyle(QDStatusKind.failed.cyberColor(for: colorScheme))
                    .padding(.top, 8)
            }
        }
    }

    private func retryText(_ files: ScenarioFileCounts) -> String {
        let count = "Ждут повтора: \(files.retrying)"
        guard let next = files.nextRetryAt, next > .now else { return count }
        return count + " · следующий в " + next.formatted(date: .omitted, time: .shortened)
    }

    @ViewBuilder
    private func regularDetails(_ snapshot: ScenarioWidgetSnapshot, _ stage: ScenarioStage) -> some View {
        let files = snapshot.run?.active?.state == "planning" ? nil : snapshot.files
        HStack(spacing: 0) {
            readout(files.map { "\($0.copied)" } ?? "—", "скопировано", .ready)
            readout(files.map { "\($0.remaining)" } ?? "—", "осталось", nil)
            readout(files.map { "\($0.errors)" } ?? "—", "ошибки", (files?.errors ?? 0) > 0 ? .failed : nil)
        }
        .padding(.top, 14)
        if let files, files.retrying > 0 {
            Text(retryText(files))
                .font(QDCyber.heading(13, weight: .medium))
                .foregroundStyle(QDCyber.color(.mutedText, for: colorScheme))
                .padding(.top, 8)
        }
        if !stage.active {
            VStack(alignment: .leading, spacing: 2) {
                Text("Следующий запуск")
                    .font(QDCyber.heading(12, weight: .semibold))
                    .tracking(1.8)
                    .textCase(.uppercase)
                    .foregroundStyle(QDCyber.color(.mutedText, for: colorScheme))
                nextRun(snapshot)
            }
            .padding(.top, 12)
        }
        HStack(spacing: 10) {
            if stage.active {
                Button {
                    perform { try WorkerIPC.stopRun(scenarioID: $0.id) }
                } label: {
                    Label("Остановить", systemImage: "stop.fill")
                }
                .buttonStyle(QDCyberButtonStyle(tint: QDStatusKind.failed.cyberColor(for: colorScheme)))
                .disabled(busy || snapshot.batchStopping)
            } else {
                Button {
                    perform { entry in
                        let token = try YandexScenarioAccess.token(for: entry)
                        try WorkerIPC.runNow(scenarioID: entry.id, accessToken: token)
                    }
                } label: {
                    Label("Запустить", systemImage: "play.fill")
                }
                .buttonStyle(QDCyberButtonStyle(tint: QDCyber.color(.accent, for: colorScheme), label: QDCyber.color(.onAccent, for: colorScheme)))
                .disabled(busy || !snapshot.enabled)
            }
        }
        .padding(.top, 14)
    }

    @ViewBuilder
    private func nextRun(_ snapshot: ScenarioWidgetSnapshot) -> some View {
        if snapshot.schedulePaused {
            Text("Расписание на паузе")
                .font(QDCyber.heading(15))
                .foregroundStyle(QDStatusKind.paused.cyberColor(for: colorScheme))
        } else if let next = snapshot.nextRunAt {
            Text(ScenarioStage.shortDate(next))
                .font(QDCyber.digits(26))
                .foregroundStyle(QDCyber.color(.accent, for: colorScheme))
        } else {
            Text(snapshot.hasSchedule ? "Время не рассчитано" : "Расписания нет")
                .font(QDCyber.heading(15))
                .foregroundStyle(QDCyber.color(.secondaryText, for: colorScheme))
        }
    }

    /// Expanded: where the Scenario reads and where it writes.
    @ViewBuilder
    private func endpoints(_ snapshot: ScenarioWidgetSnapshot) -> some View {
        if let from = snapshot.from, let to = snapshot.to {
            VStack(alignment: .leading, spacing: 6) {
                endpoint(snapshot.twoWay ? "Сторона 1" : "Источник", from)
                Image(systemName: snapshot.twoWay ? "arrow.up.arrow.down" : "arrow.down")
                    .font(.system(size: 13, weight: .bold))
                    .foregroundStyle(QDCyber.color(.accent, for: colorScheme))
                    .padding(.leading, 8)
                endpoint(snapshot.twoWay ? "Сторона 2" : "Назначение", to)
            }
            .padding(.top, 14)
        }
    }

    private func endpoint(_ title: String, _ side: ScenarioWidgetSnapshot.Endpoint) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(title)
                .font(QDCyber.heading(11, weight: .semibold))
                .tracking(1.4)
                .textCase(.uppercase)
                .foregroundStyle(QDCyber.color(.mutedText, for: colorScheme))
            Text(side.kind.isEmpty ? side.connection : side.connection + " · " + side.kind)
                .font(QDCyber.heading(15))
                .foregroundStyle(QDCyber.color(.primaryText, for: colorScheme))
                .lineLimit(1)
            Text(side.root.isEmpty ? "/" : side.root)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(QDCyber.color(.secondaryText, for: colorScheme))
                .lineLimit(1)
                .truncationMode(.middle)
                .textSelection(.enabled)
                .help(side.root)
            if side.selected > 0 {
                Text("Выбрано объектов: \(side.selected)")
                    .font(QDCyber.heading(12, weight: .medium))
                    .foregroundStyle(QDCyber.color(.mutedText, for: colorScheme))
            }
        }
        .padding(.leading, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .leading) {
            Rectangle().fill(QDCyber.color(.edge, for: colorScheme)).frame(width: 2)
        }
    }

    private func readout(_ value: String, _ label: String, _ tone: QDStatusKind?) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(value)
                .font(QDCyber.digits(28))
                .foregroundStyle(tone?.cyberColor(for: colorScheme) ?? QDCyber.color(.accent, for: colorScheme))
                .lineLimit(1)
                .minimumScaleFactor(0.6)
            Text(label)
                .font(QDCyber.heading(11, weight: .semibold))
                .tracking(1.4)
                .textCase(.uppercase)
                .foregroundStyle(QDCyber.color(.mutedText, for: colorScheme))
        }
        .padding(.leading, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .leading) {
            Rectangle().fill(QDCyber.color(.edge, for: colorScheme)).frame(width: 2)
        }
        .accessibilityElement(children: .combine)
    }

    private func selectionPrompt(_ snapshot: ScenarioWidgetSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(widget.scenarioID == nil ? "Сценарий не выбран" : "Сценарий не найден")
                .font(QDCyber.heading(22))
                .textCase(.uppercase)
                .foregroundStyle(QDCyber.color(.accent, for: colorScheme))
            if snapshot.choices.isEmpty {
                Text("Создайте сценарий в настройках, затем выберите его здесь.")
                    .font(QDCyber.heading(13, weight: .medium))
                    .foregroundStyle(QDCyber.color(.secondaryText, for: colorScheme))
            } else {
                Menu {
                    scenarioChoices(snapshot)
                } label: {
                    Label("Выбрать сценарий", systemImage: "list.bullet")
                }
                .menuStyle(.button)
                .menuIndicator(.hidden)
                .fixedSize()
                .buttonStyle(QDCyberButtonStyle(tint: QDCyber.color(.accent, for: colorScheme), label: QDCyber.color(.onAccent, for: colorScheme)))
            }
        }
        .padding(.vertical, 6)
    }

    private func scenarioMenu(_ snapshot: ScenarioWidgetSnapshot) -> some View {
        Menu {
            scenarioChoices(snapshot)
        } label: {
            Label("Сценарий виджета", systemImage: "list.bullet")
        }
        .labelStyle(.iconOnly)
        .help("Выбрать сценарий виджета")
    }

    @ViewBuilder
    private func scenarioChoices(_ snapshot: ScenarioWidgetSnapshot) -> some View {
        ForEach(snapshot.choices) { choice in
            Button {
                onSelectScenario(choice.id)
            } label: {
                if choice.id == widget.scenarioID {
                    Label(choice.name, systemImage: "checkmark")
                } else {
                    Text(choice.name)
                }
            }
        }
    }

    private func perform(_ work: @escaping @Sendable (ScenarioEntry) throws -> Void) {
        guard let entry = feed.entry else { return }
        busy = true
        actionError = nil
        Task {
            do {
                try await Task.detached { try work(entry) }.value
            } catch {
                actionError = error.localizedDescription
            }
            await feed.refresh(scenarioID: widget.scenarioID)
            busy = false
        }
    }
}
