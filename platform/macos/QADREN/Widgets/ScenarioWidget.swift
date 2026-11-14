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
    /// Copied items by kind; `created` is the rest of `copied`.
    let updated: Int
    let deleted: Int
    /// The most frequent failure code, if any.
    let topFailure: String?
    var created: Int { max(0, copied - updated - deleted) }
    var remaining: Int { max(0, total - copied - errors) }
    /// Only retries are left: the Worker copies nothing until the next retry.
    var waitingForRetry: Bool { retrying > 0 && remaining == 0 }

    init(_ summary: ScenarioBatchSummary) {
        total = summary.total
        copied = summary.completed
        updated = summary.completed_updates ?? 0
        deleted = summary.completed_deletes ?? 0
        topFailure = summary.failure_reasons.first.flatMap { pair in
            if case let .text(code)? = pair.first { code } else { nil }
        }
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
    /// A Backup/export run with databases and no files: its "planning" is
    /// the database dump.
    var databasesOnly = false
    var enabled = false
    var found = false
    var run: ScenarioRunState?
    var files: ScenarioFileCounts?
    var batchPaused = false
    var batchStopping = false
    var currentPath: String?
    /// A plan that is being made now: by "Проверить план" or by a run in
    /// planning. Shows how far the snapshots are.
    var planCheck: PlanCheck?
    /// The database step of the active run (D-027 section 7b).
    var databaseProgress: WorkerDatabaseProgress?
    /// The selected Backup/export objects outside `files/`, in run order:
    /// category and count (D-027 sections 7b and 7c).
    var subsystems: [Subsystem] = []

    struct Subsystem: Equatable {
        let category: String
        /// Selected objects; nil for "all sites", whose count is known only
        /// in a run.
        let count: Int?

        var title: String {
            switch category {
            case "databases": "Базы"
            case "cron": "Cron"
            case "dns": "DNS"
            case "mail": "Почта"
            case "sites": "Сайты"
            case "ssl": "SSL"
            case "ftp": "FTP"
            case "accounts": "Лимиты"
            case "php": "PHP"
            default: category
            }
        }
    }

    /// The object results of the active run, or of the last finished run,
    /// without the summary results of "all sites" and of the certificate
    /// list: their objects count one by one. Cron, FTP, limits, and PHP
    /// also use `all`; they stay (found on 2026-11-14).
    var objectResults: [ScenarioRunState.ObjectResult] {
        ((run?.active != nil ? run?.active?.objects : run?.last?.objects) ?? [])
            .filter { !(["sites", "ssl"].contains($0.category) && $0.provider_id == "all") }
    }

    /// A failed or stopped summary of "all sites" or of the certificate
    /// list, for example when the list itself could not be read.
    func summaryFailed(_ category: String) -> Bool {
        ((run?.active != nil ? run?.active?.objects : run?.last?.objects) ?? []).contains {
            $0.category == category && $0.provider_id == "all" && ["failed", "cancelled"].contains($0.result)
        }
    }

    /// Selected objects of one category; for "all sites" the sites that the
    /// run has reached so far.
    func total(of subsystem: Subsystem) -> Int {
        subsystem.count ?? objectResults.filter { $0.category == subsystem.category }.count
    }

    /// Finished, failed, and total objects of the current or last run.
    var objectCounts: (done: Int, failed: Int, total: Int) {
        let results = objectResults
        return (
            results.filter { ["created", "updated", "unchanged"].contains($0.result) }.count,
            results.filter { ["failed", "cancelled"].contains($0.result) }.count,
            subsystems.reduce(0) { $0 + total(of: $1) }
        )
    }
    var nextRunAt: Date?
    var hasSchedule = false
    var schedulePaused = false
    var error: String?

    /// The progress of a plan that is not complete yet.
    struct PlanCheck: Equatable {
        /// Only what is read so far: the total is unknown until the walk
        /// ends, and an estimate misleads (owner decision 2026-11-04).
        struct Side: Equatable {
            let files: Int
            let foldersDone: Int
            let complete: Bool
        }
        let source: Side?
        let destination: Side?

        var detail: String {
            // One short line for each side: a long line was cut in the
            // middle and read as "0 files" (found on 2026-11-04).
            func line(_ title: String, _ side: Side?) -> String {
                guard let side else { return "\(title): ждёт" }
                let files = side.files.formatted()
                if side.complete { return "\(title): готово, файлов \(files)" }
                return "\(title): файлов \(files), папок \(side.foldersDone.formatted())"
            }
            if source?.complete == true, destination?.complete == true { return "Снимки готовы, строю план." }
            return line("Источник", source) + "\n" + line("Назначение", destination)
        }
    }

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
        snapshot.databasesOnly = entry.databasesOnly
        if entry.isBackup {
            let included = entry.resources.filter { $0.rule == "include" }
            snapshot.subsystems = ["databases", "cron", "dns", "mail", "sites", "ssl", "ftp", "accounts", "php"].compactMap { category in
                // All sites and the certificate list count their objects in a run.
                if ["sites", "ssl"].contains(category), included.contains(where: { $0.category == category && $0.provider_id == "all" }) {
                    return ScenarioWidgetSnapshot.Subsystem(category: category, count: nil)
                }
                let count = included.filter { $0.category == category }.count
                return count > 0 ? ScenarioWidgetSnapshot.Subsystem(category: category, count: count) : nil
            }
        }
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
            // A plan in progress, with or without a run (the run waits for the
            // same plan, so both show the same progress).
            if run.active == nil || run.active?.state == "planning",
               // A run follows its own revision, also after a newer save.
               let status = try? WorkerIPC.planStatus(
                   scenarioID: scenarioID, revision: run.active?.revision ?? entry.revision
               ),
               let plan = status.plan, plan.state == "waiting" || plan.state == "building" {
                func side(_ info: WorkerSnapshotInfo?) -> PlanCheck.Side? {
                    info.map {
                        PlanCheck.Side(files: $0.files, foldersDone: $0.folders_done, complete: $0.state == "complete")
                    }
                }
                let left = side(status.left)
                let right = side(status.right)
                snapshot.planCheck = reversed
                    ? PlanCheck(source: right, destination: left)
                    : PlanCheck(source: left, destination: right)
            }
            if run.active != nil, entry.isBackup {
                snapshot.databaseProgress = try? WorkerIPC.databaseProgress(scenarioID: scenarioID)
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
        // A run that only waits for a retry does not move; it is not a hang.
        let executing = snapshot.run?.active?.state == "executing" && !snapshot.batchPaused
            && snapshot.files?.waitingForRetry != true
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
            if let progress = snapshot.databaseProgress {
                let base = progress.total > 1 ? "база \(progress.index) из \(progress.total)" : "база"
                let detail = switch progress.stage {
                case "cleanup": "Очистка папки files: файлы не выбраны."
                case "publish": "Дамп готов: \(base) записывается в копию."
                default: progress.bytes > 0
                    ? "Дамп: \(base), получено \(ByteCountFormatter.string(fromByteCount: Int64(clamping: progress.bytes), countStyle: .file))."
                    : "Дамп: \(base), подключение к серверу."
                }
                let headline = switch progress.stage {
                case "cleanup": "Очистка"
                case "settings": "Настройки"
                default: "Копия базы"
                }
                let position = progress.total > 1 ? " \(progress.index) из \(progress.total)" : ""
                let settingsText = switch progress.category {
                case "cron": "Cron: задания читаются с панели."
                case "dns": "DNS: зона\(position) читается с панели."
                case "mail": "Почта: домен\(position) читается с панели."
                case "sites": "Сайты: сайт\(position) читается с панели."
                case "ssl": "SSL: список сертификатов читается с панели."
                case "ftp": "FTP: пользователи читаются с панели."
                case "accounts": "Лимиты: тариф аккаунта читается с панели."
                case "php": "PHP: версии и расширения читаются с панели."
                default: "Настройки читаются с панели."
                }
                let text = progress.stage == "settings" ? settingsText : detail
                return ScenarioStage(frame: .active, tone: .active, headline: headline, detail: text, active: true, moving: true)
            }
            if active.state == "planning" && snapshot.databasesOnly {
                return ScenarioStage(frame: .active, tone: .active, headline: "Копия", detail: "Базы и настройки хостинга, запуск \(trigger).", active: true, moving: true)
            }
            if active.state == "planning" {
                let detail = snapshot.planCheck?.detail ?? "Снимки и план, запуск \(trigger)."
                return ScenarioStage(frame: .active, tone: .active, headline: "Планирование", detail: detail, active: true, moving: true)
            }
            if snapshot.batchStopping {
                return ScenarioStage(frame: .waiting, tone: .waiting, headline: "Остановка", detail: "Текущий файл завершается, новые не начинаются.", active: true, moving: true)
            }
            if snapshot.batchPaused {
                return ScenarioStage(frame: .paused, tone: .paused, headline: "Пауза", detail: "Новые файлы не начинаются.", active: true)
            }
            if let files = snapshot.files, files.waitingForRetry {
                let next = files.nextRetryAt.map { " Следующий повтор в " + $0.formatted(date: .omitted, time: .shortened) + "." } ?? ""
                return ScenarioStage(frame: .waiting, tone: .waiting, headline: "Ждёт повтора", detail: "Остальные файлы скопированы." + next, active: true)
            }
            return ScenarioStage(frame: .active, tone: .active, headline: "Копирование", detail: snapshot.currentPath ?? "Запуск \(trigger).", active: true, moving: true)
        }
        // "Проверить план" without a run. "Запустить" stays available: a run
        // takes this same plan.
        if let check = snapshot.planCheck {
            return ScenarioStage(frame: .active, tone: .active, headline: "Проверка плана", detail: check.detail, active: false, moving: true)
        }
        if !snapshot.enabled {
            return ScenarioStage(frame: .offline, tone: .offline, headline: "Выключен", detail: "Сценарий выключен в настройках.", active: false)
        }
        guard let last = snapshot.run?.last else {
            return ScenarioStage(frame: .offline, tone: .offline, headline: "Не запускался", detail: "Запусков этого сценария ещё не было.", active: false)
        }
        let finished = Date(timeIntervalSince1970: TimeInterval(last.finished_at ?? last.created_at))
        var when = "Завершён " + Self.shortDate(finished) + (last.trigger == "schedule" ? " · по расписанию" : " · вручную")
        if let end = last.finished_at, let duration = Self.duration(seconds: end - last.created_at) {
            when += " · " + duration
        }
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

    /// "3 мин 12 с"; nil for an unknown or negative time.
    static func duration(seconds: Int64) -> String? {
        guard seconds >= 0 else { return nil }
        let formatter = DateComponentsFormatter()
        formatter.unitsStyle = .abbreviated
        formatter.allowedUnits = [.day, .hour, .minute, .second]
        formatter.maximumUnitCount = 2
        formatter.calendar?.locale = Locale(identifier: "ru_RU")
        return seconds == 0 ? "меньше секунды" : formatter.string(from: TimeInterval(seconds))
    }

    static func shortDate(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = Calendar.current.isDateInToday(date) ? "HH:mm" : "dd.MM HH:mm"
        return formatter.string(from: date)
    }
}

/// The result of the last finished run: the changed counts and at most one
/// line about what needs attention. Replaces the progress counts when no run
/// or plan check is active.
struct ScenarioRunResult: Equatable {
    /// The batch of the run; nil when the run had no file to change.
    let files: ScenarioFileCounts?
    /// One line for compact density.
    let short: String?
    /// One line under the counts: errors first, then a stopped run.
    let note: String?
    let noteTone: QDStatusKind?

    static func evaluate(_ snapshot: ScenarioWidgetSnapshot) -> ScenarioRunResult? {
        guard snapshot.run?.active == nil, snapshot.planCheck == nil, let last = snapshot.run?.last else { return nil }
        guard let files = snapshot.files else {
            guard last.state == "completed" else { return ScenarioRunResult(files: nil, short: nil, note: nil, noteTone: nil) }
            if !snapshot.subsystems.isEmpty {
                let results = last.objects ?? []
                if let failed = results.first(where: { $0.result == "failed" }) {
                    let reason = failed.error.map(QADRENUserMessage.text(for:)) ?? "Объект не сохранён."
                    return ScenarioRunResult(files: nil, short: "Есть ошибки", note: reason, noteTone: .failed)
                }
                let changed = results.filter { ["created", "updated"].contains($0.result) }.count
                let text = changed == 0
                    ? "Объекты сохранены, изменений не было."
                    : "Объекты сохранены, изменилось: \(changed)."
                return ScenarioRunResult(files: nil, short: text, note: snapshot.databasesOnly ? text : nil, noteTone: nil)
            }
            let text = "Изменений не было."
            return ScenarioRunResult(files: nil, short: text, note: text, noteTone: nil)
        }
        let parts = [
            (files.created, "новых"), (files.updated, "заменено"), (files.deleted, "удалено"), (files.errors, "ошибок"),
        ].filter { $0.0 > 0 }.map { "\($0.1) \($0.0.formatted())" }
        let short = parts.isEmpty ? "Изменений не было." : parts.joined(separator: " · ").capitalizedFirst
        if files.errors > 0 {
            let reason = files.topFailure.map(QADRENUserMessage.text(for:)) ?? "Следующий запуск повторит эти файлы."
            return ScenarioRunResult(files: files, short: short, note: reason, noteTone: .failed)
        }
        if files.remaining > 0 {
            return ScenarioRunResult(files: files, short: short, note: "Не выполнено: \(files.remaining.formatted())", noteTone: .paused)
        }
        return ScenarioRunResult(files: files, short: short, note: nil, noteTone: nil)
    }
}

private extension String {
    var capitalizedFirst: String { prefix(1).uppercased() + dropFirst() }
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
        let result = ScenarioRunResult.evaluate(snapshot)
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .center, spacing: 14) {
                // Planning has no file total yet: the ring shows the track only.
                // A completed run with nothing to change is a full ring.
                let files = snapshot.run?.active?.state == "planning" ? nil : snapshot.files
                let nothingToDo = result != nil && result?.files == nil && result?.short != nil
                // A run without files counts its objects: databases, Cron,
                // DNS zones, and mail domains.
                let objects = snapshot.objectCounts
                let objectRing = snapshot.databasesOnly && objects.total > 0
                    && (snapshot.run?.active != nil || result != nil)
                QDCyberRing(
                    total: objectRing ? objects.total : nothingToDo ? 0 : files?.total,
                    done: objectRing ? objects.done : files?.copied ?? 0,
                    failed: objectRing ? objects.failed : files?.errors ?? 0,
                    size: compact ? 84 : 104, active: stage.moving, stalled: feed.stalledMinutes != nil
                )
                VStack(alignment: .leading, spacing: 4) {
                    QDCyberHeadline(text: stage.headline, color: stage.tone.cyberColor(for: colorScheme), size: compact ? 24 : 28)
                    Text(stage.detail)
                        .font(QDCyber.heading(13, weight: .medium))
                        .foregroundStyle(QDCyber.color(.secondaryText, for: colorScheme))
                        // Plan progress has two lines, and each can wrap.
                        .lineLimit(compact ? 3 : 4)
                        .truncationMode(.middle)
                        .fixedSize(horizontal: false, vertical: true)
                    if compact, let short = result?.short {
                        Text(short)
                            .font(QDCyber.heading(13, weight: .semibold))
                            .foregroundStyle(QDCyber.color(.primaryText, for: colorScheme))
                            .lineLimit(2)
                    }
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
                regularDetails(snapshot, stage, result)
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
    private func regularDetails(_ snapshot: ScenarioWidgetSnapshot, _ stage: ScenarioStage, _ result: ScenarioRunResult?) -> some View {
        let files = snapshot.run?.active?.state == "planning" ? nil : snapshot.files
        if let result {
            runResult(result)
        } else if let files {
            // The file counts appear only while a run copies files.
            HStack(spacing: 0) {
                readout("\(files.copied)", "скопировано", .ready)
                readout("\(files.remaining)", "осталось", nil)
                readout("\(files.errors)", "ошибки", files.errors > 0 ? .failed : nil)
            }
            .padding(.top, 14)
        }
        if !snapshot.subsystems.isEmpty, snapshot.run?.active != nil || snapshot.run?.last != nil {
            subsystemRow(snapshot)
        }
        if result == nil, let files, files.retrying > 0 {
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

    /// One column for each selected object category outside `files/`:
    /// finished objects of the current or last run against the selection.
    /// The category in work is active; a failed object makes it red.
    /// The columns wrap: one row squeezed seven labels and broke them
    /// (found on 2026-11-14).
    private func subsystemRow(_ snapshot: ScenarioWidgetSnapshot) -> some View {
        let results = snapshot.objectResults
        let current = snapshot.databaseProgress?.category
        let columns = [GridItem(.adaptive(minimum: 64), spacing: 0, alignment: .leading)]
        return LazyVGrid(columns: columns, alignment: .leading, spacing: 10) {
            ForEach(snapshot.subsystems, id: \.category) { subsystem in
                let mine = results.filter { $0.category == subsystem.category }
                let done = mine.filter { ["created", "updated", "unchanged"].contains($0.result) }.count
                let failed = mine.contains { ["failed", "cancelled"].contains($0.result) }
                    || snapshot.summaryFailed(subsystem.category)
                let total = snapshot.total(of: subsystem)
                let tone: QDStatusKind? = failed ? .failed
                    : current == subsystem.category ? .active
                    : done == total && total > 0 ? .ready : nil
                readout(subsystem.count == nil && total == 0 ? "все" : "\(done)/\(total)", subsystem.title, tone)
            }
        }
        .padding(.top, 14)
    }

    /// The last finished run: what it changed, then one note.
    @ViewBuilder
    private func runResult(_ result: ScenarioRunResult) -> some View {
        if let files = result.files {
            HStack(spacing: 0) {
                readout(files.created.formatted(), "новых", files.created > 0 ? .ready : nil)
                readout(files.updated.formatted(), "заменено", nil)
                readout(files.deleted.formatted(), "удалено", nil)
                if files.errors > 0 {
                    readout(files.errors.formatted(), "ошибки", .failed)
                }
            }
            .padding(.top, 14)
        }
        if let note = result.note {
            Text(note)
                .font(QDCyber.heading(13, weight: .medium))
                .foregroundStyle(result.noteTone?.cyberColor(for: colorScheme) ?? QDCyber.color(.secondaryText, for: colorScheme))
                .lineLimit(2)
                .padding(.top, result.files == nil ? 14 : 8)
        }
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
