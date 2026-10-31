import SwiftUI

/// The health of the background Worker service, from its launchd registration
/// and the IPC `status` request. Only the IPC answer proves a running Worker.
struct WorkerServiceHealth: Equatable {
    enum Action: Equatable {
        case start
        case restart
        case openLoginItems

        var title: String {
            switch self {
            case .start: "Запустить службу"
            case .restart: "Перезапустить службу"
            case .openLoginItems: "Открыть «Объекты входа»"
            }
        }

        var symbol: String {
            switch self {
            case .start: "play.fill"
            case .restart: "arrow.clockwise"
            case .openLoginItems: "gearshape"
            }
        }
    }

    let kind: QDStatusKind
    /// One or two words for the large status line.
    let headline: String
    let title: String
    let detail: String
    var action: Action? = nil
    /// Why the action cannot run now, for example while the Worker writes.
    var actionBlockedReason: String? = nil

    static func evaluate(
        registration: WorkerServiceRegistration,
        reachability: WorkerReachability,
        active: Int?,
        runsPreviousBuild: Bool,
        restarting: Bool,
        draining: Bool = false
    ) -> WorkerServiceHealth {
        if draining {
            return WorkerServiceHealth(kind: .waiting, headline: "Перезапуск", title: "Служба перезапускается", detail: "Worker дописывает текущие файлы и не берёт новые. Затем служба перезапустится.")
        }
        if restarting {
            return WorkerServiceHealth(kind: .waiting, headline: "Перезапуск", title: "Служба перезапускается", detail: "QADREN ждёт ответа от новой службы Worker.")
        }
        switch registration {
        case .requiresApproval:
            return WorkerServiceHealth(
                kind: .needsAction, headline: "Нет разрешения", title: "Нужно разрешение macOS",
                detail: "Разрешите QADREN в Системных настройках → Основные → Объекты входа. Без этого расписания не выполняются.",
                action: .openLoginItems
            )
        case .stoppedByUser:
            return WorkerServiceHealth(
                kind: .needsAction, headline: "Остановлена", title: "Служба остановлена",
                detail: "Вы остановили службу. Расписания не выполняются, пока она не запущена.",
                action: .start
            )
        case .registrationFailed:
            return WorkerServiceHealth(
                kind: .failed, headline: "Сбой запуска", title: "Службу не удалось запустить",
                detail: "macOS не зарегистрировала службу Worker. Расписания не выполняются.",
                action: .start
            )
        case .notRegistered, .notFound:
            return WorkerServiceHealth(
                kind: .failed, headline: "Не запущена", title: "Служба не запущена",
                detail: "Расписания не выполняются, пока служба Worker не запущена.",
                action: .start
            )
        case .unknown:
            return WorkerServiceHealth(kind: .unknown, headline: "Неизвестно", title: "Состояние службы неизвестно", detail: "macOS сообщила неизвестное состояние регистрации.")
        case .enabled:
            break
        }
        switch reachability {
        case .unchecked:
            return WorkerServiceHealth(kind: .unknown, headline: "Проверка", title: "Проверка службы", detail: "QADREN ещё не получил ответ от Worker.")
        case .unavailable:
            return WorkerServiceHealth(
                kind: .failed, headline: "Нет связи", title: "Служба не отвечает",
                detail: "Служба зарегистрирована, но процесс Worker не принимает запросы. Расписания не выполняются.",
                action: .restart
            )
        case .notResponding:
            return WorkerServiceHealth(
                kind: .needsAction, headline: "Не отвечает", title: "Служба занята или зависла",
                detail: "Worker не ответил вовремя. При записи большого файла это бывает; если состояние не меняется, проверьте задания."
            )
        case .responding:
            break
        }
        if runsPreviousBuild {
            return WorkerServiceHealth(
                kind: .needsAction, headline: "Старая сборка", title: "Служба от прошлой сборки",
                detail: "Приложение обновилось, а работает старая служба Worker. Перезапуск дождётся конца текущих файлов.",
                action: .restart
            )
        }
        return WorkerServiceHealth(kind: .ready, headline: "Онлайн", title: "Служба работает", detail: "Worker отвечает и выполняет расписания.")
    }
}

extension MonitoringFeed {
    /// The registration is read when the view renders; the feed publishes a
    /// new Worker check every refresh, so the value follows within one interval.
    var serviceHealth: WorkerServiceHealth {
        WorkerServiceHealth.evaluate(
            registration: LocalWorkerController.registration,
            reachability: worker,
            active: workerActive,
            runsPreviousBuild: LocalWorkerController.runsPreviousBuild,
            restarting: LocalWorkerController.isRestarting,
            draining: LocalWorkerController.isDraining
        )
    }
}

/// `system`: the background Worker service. One fixed presentation.
struct SystemWidget<Actions: View>: View {
    @ObservedObject var feed: MonitoringFeed
    @ViewBuilder let actions: Actions
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let health = feed.serviceHealth
        let accent = health.kind.cyberColor(for: colorScheme)
        QDWidgetFrame(title: MonitoringWidgetKind.system.title, symbol: MonitoringWidgetKind.system.symbol, status: health.kind, actions: {
            actions
        }) {
            VStack(alignment: .leading, spacing: 0) {
                QDCyberHeadline(text: health.headline, color: accent)
                Text(health.detail)
                    .font(QDCyber.heading(14, weight: .medium))
                    .foregroundStyle(QDCyber.color(.secondaryText, for: colorScheme))
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 4)
                HStack(spacing: 0) {
                    readout(value: activeValue, label: "операции", tint: activeTint)
                    TimelineView(.periodic(from: .now, by: MonitoringFeed.refreshInterval)) { context in
                        checkedReadout(now: context.date)
                    }
                }
                .padding(.top, 14)
                if let action = health.action {
                    Button {
                        perform(action)
                    } label: {
                        Label(action.title, systemImage: action.symbol)
                    }
                    .buttonStyle(QDCyberButtonStyle(
                        tint: accent,
                        label: health.kind == .failed ? .white : QDCyber.color(.onAccent, for: colorScheme)
                    ))
                    .disabled(health.actionBlockedReason != nil)
                    .padding(.top, 14)
                }
                if let reason = health.actionBlockedReason {
                    Text(reason)
                        .font(QDCyber.heading(13, weight: .medium))
                        .foregroundStyle(QDCyber.color(.mutedText, for: colorScheme))
                        .padding(.top, 8)
                }
                Text(versionText)
                    .font(QDCyber.digits(15))
                    .foregroundStyle(QDCyber.color(.mutedText, for: colorScheme))
                    .padding(.top, 12)
            }
        }
    }

    private func checkedReadout(now: Date) -> some View {
        switch MonitoringFreshness.evaluate(lastSuccess: feed.lastWorkerCheck, now: now, interval: MonitoringFeed.refreshInterval) {
        case .never:
            readout(value: "—", label: "проверено", tint: nil)
        case let .fresh(date):
            readout(value: date.formatted(date: .omitted, time: .standard), label: "проверено", tint: nil)
        case let .stale(date):
            readout(value: date.formatted(date: .omitted, time: .standard), label: "данные устарели", tint: QDStatusKind.needsAction.cyberColor(for: colorScheme))
        }
    }

    private func readout(value: String, label: String, tint: Color?) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(value)
                .font(QDCyber.digits(32))
                .foregroundStyle(tint ?? QDCyber.color(.accent, for: colorScheme))
                .lineLimit(1)
                .minimumScaleFactor(0.7)
            Text(label)
                .font(QDCyber.heading(12, weight: .semibold))
                .tracking(1.8)
                .textCase(.uppercase)
                .foregroundStyle(QDCyber.color(.mutedText, for: colorScheme))
        }
        .padding(.leading, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .leading) {
            Rectangle().fill(QDCyber.color(.edge, for: colorScheme)).frame(width: 2)
        }
        .accessibilityElement(children: .combine)
    }

    /// Unknown, not zero, when the Worker did not answer.
    private var activeValue: String {
        feed.workerActive.map(String.init) ?? "—"
    }

    private var activeTint: Color? {
        guard let active = feed.workerActive, active > 0 else { return nil }
        return QDStatusKind.active.cyberColor(for: colorScheme)
    }

    /// VT323 has no Cyrillic, so this line uses Latin words only.
    private var versionText: String {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "—"
        guard let built = Self.buildDate(LocalWorkerController.buildVersion) else { return "QADREN \(version)" }
        let local = DateFormatter()
        local.dateFormat = "dd.MM HH:mm"
        return "QADREN \(version) // BUILD \(local.string(from: built))"
    }

    private static func buildDate(_ version: String) -> Date? {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(identifier: "UTC")
        formatter.dateFormat = "yyyyMMddHHmmss"
        return formatter.date(from: version)
    }

    private func perform(_ action: WorkerServiceHealth.Action) {
        switch action {
        case .start, .restart: LocalWorkerController.restart()
        case .openLoginItems: LocalWorkerController.openLoginItemsSettings()
        }
        Task { await feed.refresh() }
    }
}
