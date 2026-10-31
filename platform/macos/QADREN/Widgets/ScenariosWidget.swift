import SwiftUI

/// The row counts that Layout mode offers for a `scenarios` widget.
enum ScenariosWidgetLimit {
    static let choices: [Int?] = [3, 5, 8, 12, nil]

    static func title(_ limit: Int?) -> String {
        limit.map { "Показывать \($0)" } ?? "Показывать все"
    }
}

/// One Scenario row: its direction, schedule, and the nearest run.
struct ScenarioQueueRow: Equatable, Identifiable {
    enum State: Equatable {
        /// A run plans or copies now.
        case running
        /// The run waits for another Scenario that writes the same root.
        case waiting
        /// A schedule gives the next run.
        case scheduled(Date)
        /// No enabled schedule: runs only on request.
        case manual
        /// The schedules are paused on this Mac.
        case paused
        case disabled
    }

    let id: String
    let name: String
    /// Source and destination of a one-way Scenario; the two sides otherwise.
    let from: String
    let to: String
    let twoWay: Bool
    let state: State
    /// The first enabled schedule rule; "+N" counts the others.
    let schedule: String?
    let extraSchedules: Int
    /// The newest finished run failed (not single files: the run itself).
    let lastRunFailed: Bool

    /// Running first, then the nearest scheduled run, then manual, paused,
    /// and disabled Scenarios.
    fileprivate var order: (Int, Date, String) {
        switch state {
        case .running: (0, .distantPast, name)
        case .waiting: (1, .distantPast, name)
        case let .scheduled(date): (2, date, name)
        case .manual: (3, .distantFuture, name)
        case .paused: (4, .distantFuture, name)
        case .disabled: (5, .distantFuture, name)
        }
    }

    static func capture() throws -> [ScenarioQueueRow] {
        let scenarios = try StorageRegistryBridge.listScenarios()
        let connections = (try? StorageRegistryBridge.list()) ?? []
        let parser = ISO8601DateFormatter()
        func name(_ id: String) -> String { connections.first { $0.id == id }?.name ?? "Подключение не найдено" }
        let rows = scenarios.map { entry -> ScenarioQueueRow in
            let left = name(entry.left_connection_id)
            let right = name(entry.right_connection_id)
            let reversed = entry.direction == "right_to_left"
            let schedules = ((try? StorageRegistryBridge.listScenarioSchedules(scenarioID: entry.id)) ?? []).filter(\.enabled)
            let run = try? StorageRegistryBridge.scenarioRunState(scenarioID: entry.id)
            let paused = (try? StorageRegistryBridge.scenarioAutomation(scenarioID: entry.id))?.locally_paused ?? false
            let next = schedules.compactMap { $0.next_at.flatMap(parser.date(from:)) }.min()
            let state: State
            if let active = run?.active {
                state = active.waiting_for_scenario != nil ? .waiting : .running
            } else if !entry.enabled {
                state = .disabled
            } else if paused {
                state = .paused
            } else if let next {
                state = .scheduled(next)
            } else {
                state = .manual
            }
            return ScenarioQueueRow(
                id: entry.id,
                name: entry.name,
                from: reversed ? right : left,
                to: reversed ? left : right,
                twoWay: entry.direction == "bidirectional",
                state: state,
                schedule: schedules.first.map(ScheduleWording.rule),
                extraSchedules: max(0, schedules.count - 1),
                lastRunFailed: run?.active == nil && run?.last?.state == "failed"
            )
        }
        return rows.sorted { $0.order < $1.order }
    }
}

/// Periodic read-only feed of all Scenarios for the widget.
@MainActor
final class ScenariosWidgetFeed: ObservableObject {
    @Published private(set) var rows: [ScenarioQueueRow] = []
    @Published private(set) var error: String?
    @Published private(set) var loaded = false

    func follow() async {
        while !Task.isCancelled {
            let result = await Task.detached(priority: .utility) {
                Result { try ScenarioQueueRow.capture() }
            }.value
            switch result {
            case let .success(rows):
                self.rows = rows
                error = nil
            case let .failure(failure):
                error = failure.localizedDescription
            }
            loaded = true
            try? await Task.sleep(for: .seconds(5))
        }
    }
}

/// `scenarios`: all Scenarios in the order of their nearest run, with the
/// direction and the schedule of each. Layout mode sets the row count.
struct ScenariosWidget<Actions: View>: View {
    let limit: Int?
    @ViewBuilder let actions: Actions
    @StateObject private var feed = ScenariosWidgetFeed()
    @Environment(\.openWindow) private var openWindow
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        QDWidgetFrame(title: MonitoringWidgetKind.scenarios.title, symbol: MonitoringWidgetKind.scenarios.symbol, status: .ready, actions: {
            actions
        }) {
            VStack(alignment: .leading, spacing: 0) {
                if let error = feed.error {
                    QDCyberHeadline(text: "Сбой", color: QDStatusKind.failed.cyberColor(for: colorScheme), size: 26)
                    detail("Список сценариев недоступен: \(error)")
                } else if feed.loaded, feed.rows.isEmpty {
                    QDCyberHeadline(text: "Пусто", color: QDCyber.color(.accent, for: colorScheme), size: 26)
                    detail("Сценарии ещё не созданы.")
                    Button("Создать сценарий") { openWindow(id: "scenario-settings") }
                        .buttonStyle(QDCyberLinkStyle())
                        .padding(.top, 12)
                } else if feed.loaded {
                    summary.padding(.bottom, 6)
                    rows
                }
            }
        }
        .task { await feed.follow() }
    }

    private var summary: some View {
        let running = feed.rows.filter { $0.state == .running }.count
        return HStack(alignment: .firstTextBaseline, spacing: 14) {
            counter("\(feed.rows.count)", "всего", nil)
            counter("\(running)", "в работе", running > 0 ? .active : nil)
            Spacer()
        }
    }

    private func counter(_ value: String, _ label: String, _ tint: QDStatusKind?) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(value)
                .font(QDCyber.digits(26))
                .foregroundStyle(tint?.cyberColor(for: colorScheme) ?? QDCyber.color(.accent, for: colorScheme))
            caption(label)
        }
    }

    private var rows: some View {
        let visible = limit.map { Array(feed.rows.prefix($0)) } ?? feed.rows
        let hidden = feed.rows.count - visible.count
        return VStack(alignment: .leading, spacing: 0) {
            ForEach(visible) { row in
                divider
                rowView(row)
            }
            if hidden > 0 {
                divider
                caption("Ещё \(hidden) · число строк меняется в режиме компоновки")
                    .padding(.vertical, 6)
            }
        }
    }

    private var divider: some View {
        Rectangle()
            .fill(.clear)
            .frame(height: 1)
            .overlay(
                GeometryReader { proxy in
                    Path { path in
                        path.move(to: .zero)
                        path.addLine(to: CGPoint(x: proxy.size.width, y: 0))
                    }
                    .stroke(QDCyber.color(.edge, for: colorScheme).opacity(0.25), style: StrokeStyle(lineWidth: 1, dash: [3, 3]))
                }
            )
    }

    private func rowView(_ row: ScenarioQueueRow) -> some View {
        HStack(alignment: .top, spacing: 10) {
            marker(row).padding(.top, 5)
            VStack(alignment: .leading, spacing: 2) {
                Text(row.name)
                    .font(QDCyber.heading(16, weight: .semibold))
                    .foregroundStyle(QDCyber.color(row.state == .disabled ? .mutedText : .primaryText, for: colorScheme))
                    .lineLimit(1)
                    .truncationMode(.tail)
                direction(row)
                if let schedule = row.schedule {
                    Text(row.extraSchedules > 0 ? "\(schedule) · +\(row.extraSchedules)" : schedule)
                        .font(QDCyber.heading(13, weight: .medium))
                        .foregroundStyle(QDCyber.color(.mutedText, for: colorScheme))
                        .lineLimit(1)
                }
                if row.state == .running {
                    QDCyberActivityBar(height: 3).padding(.top, 3)
                }
            }
            Spacer(minLength: 8)
            when(row)
        }
        .padding(.vertical, 8)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibility(row))
    }

    /// "Источник → Назначение", or "Сторона 1 ⇄ Сторона 2" for a two-way Scenario.
    private func direction(_ row: ScenarioQueueRow) -> some View {
        HStack(spacing: 6) {
            Text(row.from).lineLimit(1).truncationMode(.middle)
            Image(systemName: row.twoWay ? "arrow.left.arrow.right" : "arrow.right")
                .font(.system(size: 11, weight: .bold))
                .foregroundStyle(QDCyber.color(.mint, for: colorScheme))
            Text(row.to).lineLimit(1).truncationMode(.middle)
        }
        .font(QDCyber.heading(13, weight: .semibold))
        .foregroundStyle(QDCyber.color(.secondaryText, for: colorScheme))
        .help(row.twoWay ? "Двусторонняя синхронизация: \(row.from) ⇄ \(row.to)" : "Из \(row.from) в \(row.to)")
    }

    @ViewBuilder
    private func when(_ row: ScenarioQueueRow) -> some View {
        switch row.state {
        case .running:
            status("идёт", .active)
        case .waiting:
            status("ждёт", .waiting)
        case let .scheduled(date):
            VStack(alignment: .trailing, spacing: 0) {
                Text(date.formatted(date: .omitted, time: .shortened))
                    .font(QDCyber.digits(24))
                    .foregroundStyle(QDCyber.color(.accent, for: colorScheme))
                caption(dayTitle(date))
            }
        case .manual:
            status("вручную", nil)
        case .paused:
            status("пауза", .needsAction)
        case .disabled:
            status("выкл", nil)
        }
    }

    private func status(_ text: String, _ kind: QDStatusKind?) -> some View {
        Text(text)
            .font(QDCyber.heading(12, weight: .bold))
            .tracking(1.4)
            .textCase(.uppercase)
            .foregroundStyle(kind?.cyberColor(for: colorScheme) ?? QDCyber.color(.mutedText, for: colorScheme))
            .padding(.top, 4)
    }

    private func marker(_ row: ScenarioQueueRow) -> some View {
        let kind: QDStatusKind = switch row.state {
        case .running, .waiting: .active
        case .disabled: .offline
        case .paused: .needsAction
        case .scheduled, .manual: row.lastRunFailed ? .failed : .ready
        }
        let color = kind.cyberColor(for: colorScheme)
        return QDCyberCutShape(cut: 4)
            .fill(color)
            .frame(width: 10, height: 10)
            .shadow(color: color.opacity(0.6), radius: 4)
            .help(row.lastRunFailed ? "Последний запуск завершился сбоем" : "")
    }

    private func dayTitle(_ date: Date) -> String {
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return "сегодня" }
        if calendar.isDateInTomorrow(date) { return "завтра" }
        return date.formatted(.dateTime.day().month(.abbreviated))
    }

    private func detail(_ text: String) -> some View {
        Text(text)
            .font(QDCyber.heading(14, weight: .medium))
            .foregroundStyle(QDCyber.color(.secondaryText, for: colorScheme))
            .fixedSize(horizontal: false, vertical: true)
            .padding(.top, 4)
    }

    private func caption(_ text: String) -> some View {
        Text(text)
            .font(QDCyber.heading(12, weight: .semibold))
            .tracking(1.6)
            .textCase(.uppercase)
            .foregroundStyle(QDCyber.color(.mutedText, for: colorScheme))
            .lineLimit(1)
    }

    private func accessibility(_ row: ScenarioQueueRow) -> String {
        let direction = row.twoWay ? "между \(row.from) и \(row.to)" : "из \(row.from) в \(row.to)"
        let when: String = switch row.state {
        case .running: "выполняется"
        case .waiting: "ждёт другой сценарий"
        case let .scheduled(date): "следующий запуск \(date.formatted(date: .abbreviated, time: .shortened))"
        case .manual: "запуск вручную"
        case .paused: "расписание на паузе"
        case .disabled: "выключен"
        }
        return "\(row.name), \(direction), \(when)"
    }
}
