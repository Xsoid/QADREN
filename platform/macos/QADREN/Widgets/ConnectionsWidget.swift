import SwiftUI

/// `connections`: the local StorageConnection registry and per-connection state.
/// Compact shows the summary and one marker per connection; regular adds up to
/// four rows with used and free space; expanded shows all rows and the SMB
/// share address. Layout mode selects the connections that the widget shows.
struct ConnectionsWidget<Actions: View>: View {
    let widget: MonitoringWidgetLayout
    @ObservedObject var connections: StorageConnections
    let isEditing: Bool
    let onShowConnection: (String, Bool) -> Void
    @ViewBuilder let actions: Actions
    @StateObject private var space = ConnectionSpaceFeed()
    @Environment(\.openWindow) private var openWindow
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.qdWindowVisible) private var windowVisible

    private var density: MonitoringWidgetDensity { widget.density }
    private var regularRows: Int { 4 }

    /// The connections that the user selected for this widget.
    private var shownEntries: [StorageEntry] {
        connections.entries.filter { !widget.hiddenConnectionIDs.contains($0.id) }
    }

    /// Hidden connections that need the user. A required sign-in stays
    /// visible outside the widget selection (widget system section 6).
    private var hiddenProblems: Int {
        connections.entries
            .filter { widget.hiddenConnectionIDs.contains($0.id) && problem(status(for: $0)) }
            .count
    }

    /// Space reads only for the rows on screen, and only while the window is
    /// visible (D-028 rule 6).
    private var spaceKey: ConnectionSpaceFeed.Key {
        let rows = density == .compact ? [] : density == .expanded ? shownEntries : Array(shownEntries.prefix(regularRows))
        return ConnectionSpaceFeed.Key(
            targets: rows.compactMap { entry in
                status(for: entry) == .ready ? ConnectionSpaceFeed.Target(entry) : nil
            },
            visible: windowVisible
        )
    }

    var body: some View {
        let aggregate = aggregateStatus
        QDWidgetFrame(title: MonitoringWidgetKind.connections.title, symbol: MonitoringWidgetKind.connections.symbol, status: frameStatus(aggregate), actions: {
            if isEditing { connectionMenu }
            actions
        }) {
            VStack(alignment: .leading, spacing: 0) {
                summary(aggregate)
                if connections.error == nil, !shownEntries.isEmpty {
                    if density == .compact {
                        markers.padding(.top, 12)
                    } else {
                        rows.padding(.top, 10)
                    }
                }
                if connections.error == nil, hiddenProblems > 0 {
                    HStack(spacing: 10) {
                        marker(.needsAction)
                        Text(hiddenProblems == 1 ? "Скрытое подключение требует внимания" : "Скрытые подключения требуют внимания: \(hiddenProblems)")
                            .font(QDCyber.heading(13, weight: .medium))
                            .foregroundStyle(QDStatusKind.needsAction.cyberColor(for: colorScheme))
                            .fixedSize(horizontal: false, vertical: true)
                        Spacer(minLength: 6)
                        Button("Решить") { openConnections() }
                            .buttonStyle(QDCyberLinkStyle())
                    }
                    .padding(.top, 10)
                }
                if density == .compact, connections.error == nil, problem(aggregate) {
                    Button("Решить проблему") { openConnections() }
                        .buttonStyle(QDCyberLinkStyle())
                        .padding(.top, 12)
                }
                // An empty registry has a way to its first connection. A
                // filled one has no settings button (owner request 2026-11-15):
                // a problem row has "Решить".
                if connections.error == nil, connections.entries.isEmpty {
                    Button("Добавить подключение") {
                        openWindow(id: "scenario-settings")
                    }
                    .buttonStyle(QDCyberLinkStyle())
                    .padding(.top, 12)
                }
            }
        }
        .task(id: spaceKey) {
            guard spaceKey.visible, !spaceKey.targets.isEmpty else { return }
            await space.follow(spaceKey.targets)
        }
    }

    // MARK: Layout menu

    /// Layout mode: one checkbox for each connection.
    private var connectionMenu: some View {
        Menu {
            ForEach(connections.entries) { entry in
                Toggle(isOn: Binding(
                    get: { !widget.hiddenConnectionIDs.contains(entry.id) },
                    set: { onShowConnection(entry.id, $0) }
                )) {
                    Text("\(entry.name) · \(adapterTitle(entry.adapter))")
                }
            }
        } label: {
            Label("Подключения виджета", systemImage: "checklist")
        }
        .labelStyle(.iconOnly)
        .disabled(connections.entries.isEmpty)
        .help("Выбрать подключения, которые показывает виджет")
    }

    // MARK: Summary

    private func summary(_ aggregate: ConnectionPresentationStatus) -> some View {
        let kind = statusKind(for: aggregate)
        return VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                QDCyberHeadline(text: headline(aggregate), color: kind.cyberColor(for: colorScheme), size: density == .compact ? 30 : 26)
                Spacer(minLength: 4)
                if connections.error == nil, !shownEntries.isEmpty {
                    VStack(alignment: .trailing, spacing: 0) {
                        Text("\(readyCount)/\(enabledCount)")
                            .font(QDCyber.digits(30))
                            .foregroundStyle(QDCyber.color(.accent, for: colorScheme))
                        caption("онлайн")
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("Онлайн \(readyCount) из \(enabledCount)")
                }
            }
            if let detail = summaryDetail(aggregate) {
                Text(detail)
                    .font(QDCyber.heading(14, weight: .medium))
                    .foregroundStyle(QDCyber.color(.secondaryText, for: colorScheme))
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private func headline(_ aggregate: ConnectionPresentationStatus) -> String {
        if connections.error != nil { return "Сбой" }
        if shownEntries.isEmpty { return "Пусто" }
        switch aggregate {
        case .ready: return "В сети"
        case .needsAction: return "Нужен вход"
        case .failed: return "Ошибка"
        case .offline, .disconnected: return "Нет связи"
        case .paused: return "Выключены"
        case .waiting: return "Ожидание"
        case .unknown: return "Проверка"
        }
    }

    private func summaryDetail(_ aggregate: ConnectionPresentationStatus) -> String? {
        if let error = connections.error { return "Реестр подключений недоступен: \(error)" }
        if connections.entries.isEmpty { return "Подключения ещё не настроены." }
        if shownEntries.isEmpty { return "Все подключения скрыты. Выберите их в режиме «Компоновка»." }
        return nil
    }

    // MARK: Compact markers

    private var markers: some View {
        HStack(spacing: 6) {
            ForEach(shownEntries) { entry in
                marker(statusKind(for: status(for: entry)))
                    .help("\(entry.name) · \(status(for: entry).title)")
            }
        }
        .accessibilityHidden(true)
    }

    private func marker(_ kind: QDStatusKind, size: CGFloat = 10) -> some View {
        let color = kind.cyberColor(for: colorScheme)
        return QDCyberCutShape(cut: size * 0.4)
            .fill(color)
            .frame(width: size, height: size)
            .shadow(color: color.opacity(kind == .ready ? 0.7 : 0.4), radius: 4)
    }

    // MARK: Rows

    private var rows: some View {
        let visible = density == .expanded ? shownEntries : Array(shownEntries.prefix(regularRows))
        let hidden = shownEntries.count - visible.count
        return VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(visible.enumerated()), id: \.element.id) { index, entry in
                if index > 0 { divider }
                row(entry)
            }
            if hidden > 0 {
                divider
                caption("Ещё \(hidden) · подробный размер покажет все")
                    .padding(.vertical, 6)
            }
        }
    }

    private var divider: some View {
        Line()
            .stroke(QDCyber.color(.edge, for: colorScheme).opacity(0.25), style: StrokeStyle(lineWidth: 1, dash: [3, 3]))
            .frame(height: 1)
    }

    private func row(_ entry: StorageEntry) -> some View {
        let presentation = status(for: entry)
        let kind = statusKind(for: presentation)
        let attention = kind == .needsAction || kind == .failed || kind == .unknown
        return VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .center, spacing: 10) {
                marker(kind)
                VStack(alignment: .leading, spacing: 1) {
                    Text(entry.name)
                        .font(QDCyber.heading(16, weight: .semibold))
                        .foregroundStyle(QDCyber.color(entry.enabled ? .primaryText : .mutedText, for: colorScheme))
                        .lineLimit(1)
                        .truncationMode(.middle)
                    caption(adapterTitle(entry.adapter))
                    if density == .expanded, let address = entry.endpoint_url, !address.isEmpty {
                        Text(address)
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(QDCyber.color(.secondaryText, for: colorScheme))
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .textSelection(.enabled)
                            .help(address)
                            .padding(.top, 1)
                    }
                }
                Spacer(minLength: 6)
                // The marker already says "connected"; a word shows only a deviation.
                if presentation != .ready {
                    Text(presentation.title)
                        .font(QDCyber.heading(12, weight: .bold))
                        .tracking(1.4)
                        .textCase(.uppercase)
                        .foregroundStyle(attention ? kind.cyberColor(for: colorScheme) : QDCyber.color(.mutedText, for: colorScheme))
                        .lineLimit(1)
                }
                // A problem that needs the user has a way to its fix. "Нет связи"
                // can pass by itself and has no button.
                if problem(presentation) {
                    Button("Решить") { openConnections() }
                        .buttonStyle(QDCyberLinkStyle())
                        .help("Открыть подключение «\(entry.name)» в настройках")
                }
            }
            if presentation == .ready, let value = space.values[entry.id] {
                spaceLine(value)
                    .padding(.leading, 20)
            }
        }
        .padding(.vertical, 7)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("\(entry.name), \(adapterTitle(entry.adapter)), \(presentation.title)")
    }

    private func problem(_ presentation: ConnectionPresentationStatus) -> Bool {
        presentation == .needsAction || presentation == .failed
    }

    private func openConnections() {
        openWindow(id: "scenario-settings")
        NotificationCenter.default.post(name: .qadrenShowConnectionSettings, object: nil)
    }

    private func caption(_ text: String) -> some View {
        Text(text)
            .font(QDCyber.heading(12, weight: .semibold))
            .tracking(1.6)
            .textCase(.uppercase)
            .foregroundStyle(QDCyber.color(.mutedText, for: colorScheme))
            .lineLimit(1)
    }

    // MARK: Space

    /// Used and free space of one connection: a bar and one line. A read that
    /// failed says so; it never shows an old value as current.
    @ViewBuilder
    private func spaceLine(_ value: ConnectionSpace) -> some View {
        switch value {
        case let .known(total, free):
            let used = total > free ? total - free : 0
            let fraction = total > 0 ? Double(used) / Double(total) : 0
            // Less than 10 % free is a warning, less than 3 % is a failure.
            let tone: QDStatusKind? = fraction >= 0.97 ? .failed : fraction >= 0.9 ? .needsAction : nil
            let fill = tone?.cyberColor(for: colorScheme) ?? QDCyber.color(.accent, for: colorScheme)
            VStack(alignment: .leading, spacing: 3) {
                GeometryReader { proxy in
                    ZStack(alignment: .leading) {
                        Rectangle().fill(QDCyber.color(.edge, for: colorScheme).opacity(0.3))
                        Rectangle().fill(fill)
                            .frame(width: max(2, proxy.size.width * fraction))
                    }
                }
                .frame(height: 4)
                HStack(spacing: 6) {
                    Text("Занято \(Self.bytes(used)) из \(Self.bytes(total))")
                        .foregroundStyle(QDCyber.color(.secondaryText, for: colorScheme))
                    Spacer(minLength: 4)
                    Text("Свободно \(Self.bytes(free))")
                        .foregroundStyle(tone?.cyberColor(for: colorScheme) ?? QDCyber.color(.primaryText, for: colorScheme))
                }
                .font(QDCyber.heading(12, weight: .semibold))
                .lineLimit(1)
                .minimumScaleFactor(0.85)
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Занято \(Self.bytes(used)) из \(Self.bytes(total)), свободно \(Self.bytes(free))")
        case .unavailable:
            caption("Место: нет данных")
        }
    }

    private static func bytes(_ value: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: value), countStyle: .file)
    }

    // MARK: Status

    private var enabledEntries: [StorageEntry] { shownEntries.filter(\.enabled) }
    private var enabledCount: Int { enabledEntries.count }
    private var readyCount: Int { enabledEntries.filter { status(for: $0) == .ready }.count }

    private var aggregateStatus: ConnectionPresentationStatus {
        ConnectionPresentationStatus.aggregate(
            shownEntries.map(status(for:)),
            registryFailed: connections.error != nil
        )
    }

    /// A healthy, empty, or switched-off registry keeps the frame quiet; only
    /// a connection problem colors it.
    private func frameStatus(_ aggregate: ConnectionPresentationStatus) -> QDStatusKind {
        switch aggregate {
        case .paused, .waiting: .ready
        default: statusKind(for: aggregate)
        }
    }

    private func status(for entry: StorageEntry) -> ConnectionPresentationStatus {
        ConnectionPresentationStatus(adapter: entry.adapter, enabled: entry.enabled, registryStatus: entry.status)
    }

    private func statusKind(for status: ConnectionPresentationStatus) -> QDStatusKind {
        switch status {
        case .ready: .ready
        case .needsAction: .needsAction
        case .failed: .failed
        case .offline: .offline
        case .disconnected: .offline
        case .paused: .paused
        case .waiting: .waiting
        case .unknown: .unknown
        }
    }

    private func adapterTitle(_ adapter: String) -> String { StorageConnector(adapter: adapter)?.title ?? adapter }
}

private struct Line: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: CGPoint(x: rect.minX, y: rect.midY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.midY))
        return path
    }
}

/// Used and free space of one connection. `unavailable` means that the read
/// failed or the storage does not report its space.
enum ConnectionSpace: Equatable {
    case known(total: UInt64, free: UInt64)
    case unavailable
}

/// Reads the space of the shown connections. It reads only metadata: the
/// volume of this Mac, the QADREN mount of an SMB share, and the Yandex Disk
/// quota. It never reads file content and never mounts a share.
@MainActor
final class ConnectionSpaceFeed: ObservableObject {
    struct Target: Hashable, Sendable {
        let id: String
        let adapter: String

        init(id: String, adapter: String) {
            self.id = id
            self.adapter = adapter
        }

        /// Nil for a connection without space, such as an ISPmanager panel.
        init?(_ entry: StorageEntry) {
            guard [StorageConnector.thisMac, .networkDrive, .yandexDisk]
                .contains(where: { $0.rawValue == entry.adapter }) else { return nil }
            self.init(id: entry.id, adapter: entry.adapter)
        }
    }

    struct Key: Hashable {
        let targets: [Target]
        let visible: Bool
    }

    /// D-028: free space changes slowly; a Yandex Disk quota is an HTTP call.
    static let refreshInterval: Duration = .seconds(600)

    @Published private(set) var values: [String: ConnectionSpace] = [:]

    func follow(_ targets: [Target]) async {
        while !Task.isCancelled {
            let read = await Task.detached(priority: .utility) {
                var result: [String: ConnectionSpace] = [:]
                for target in targets {
                    result[target.id] = Self.read(target)
                }
                return result
            }.value
            guard !Task.isCancelled else { return }
            // D-028 rule 6: publish only a change.
            if values != read { values = read }
            try? await Task.sleep(for: Self.refreshInterval)
        }
    }

    nonisolated private static func read(_ target: Target) -> ConnectionSpace {
        switch StorageConnector(adapter: target.adapter) {
        case .thisMac:
            return volume(URL(fileURLWithPath: NSHomeDirectory(), isDirectory: true), important: true)
        case .networkDrive:
            // Only a share that QADREN has mounted; a read never mounts it.
            guard SMBMounts.isMounted(target.id), let path = SMBMounts.path(for: target.id) else {
                return .unavailable
            }
            return volume(URL(fileURLWithPath: path, isDirectory: true), important: false)
        case .yandexDisk:
            guard let token = try? YandexKeychain.read(target.id),
                  let quota = try? DiskMetadataBridge.quota(token: token),
                  quota.total_space > 0 else { return .unavailable }
            let free = quota.total_space > quota.used_space ? quota.total_space - quota.used_space : 0
            return .known(total: quota.total_space, free: free)
        case .ispmanager, nil:
            return .unavailable
        }
    }

    nonisolated private static func volume(_ url: URL, important: Bool) -> ConnectionSpace {
        let keys: Set<URLResourceKey> = [
            .volumeTotalCapacityKey, .volumeAvailableCapacityKey, .volumeAvailableCapacityForImportantUsageKey,
        ]
        guard let values = try? url.resourceValues(forKeys: keys),
              let total = values.volumeTotalCapacity, total > 0 else { return .unavailable }
        // On this Mac, Finder counts purgeable space as free.
        let free = important
            ? values.volumeAvailableCapacityForImportantUsage.map { max(0, $0) } ?? Int64(values.volumeAvailableCapacity ?? 0)
            : Int64(values.volumeAvailableCapacity ?? 0)
        return .known(total: UInt64(total), free: UInt64(max(0, free)))
    }
}
