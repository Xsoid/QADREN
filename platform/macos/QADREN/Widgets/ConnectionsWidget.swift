import SwiftUI

/// `connections`: the local StorageConnection registry and per-connection state.
/// Compact shows the summary and one marker per connection; regular adds up to
/// four rows; expanded shows all rows and the SMB share address.
struct ConnectionsWidget<Actions: View>: View {
    let density: MonitoringWidgetDensity
    @ObservedObject var connections: StorageConnections
    @ViewBuilder let actions: Actions
    @Environment(\.openWindow) private var openWindow
    @Environment(\.colorScheme) private var colorScheme

    private var regularRows: Int { 4 }

    var body: some View {
        let aggregate = aggregateStatus
        QDWidgetFrame(title: MonitoringWidgetKind.connections.title, symbol: MonitoringWidgetKind.connections.symbol, status: frameStatus(aggregate), actions: {
            actions
        }) {
            VStack(alignment: .leading, spacing: 0) {
                summary(aggregate)
                if connections.error == nil, !connections.entries.isEmpty {
                    if density == .compact {
                        markers.padding(.top, 12)
                    } else {
                        rows.padding(.top, 10)
                    }
                }
                if density == .compact, connections.error == nil, problem(aggregate) {
                    Button("Решить проблему") { openConnections() }
                        .buttonStyle(QDCyberLinkStyle())
                        .padding(.top, 12)
                }
                if density != .compact || connections.entries.isEmpty || connections.error != nil {
                    Button {
                        openWindow(id: "scenario-settings")
                    } label: {
                        Text(connections.entries.isEmpty ? "Добавить подключение" : "Настроить подключения")
                    }
                    .buttonStyle(QDCyberLinkStyle())
                    .padding(.top, 12)
                }
            }
        }
    }

    // MARK: Summary

    private func summary(_ aggregate: ConnectionPresentationStatus) -> some View {
        let kind = statusKind(for: aggregate)
        return VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                QDCyberHeadline(text: headline(aggregate), color: kind.cyberColor(for: colorScheme), size: density == .compact ? 30 : 26)
                Spacer(minLength: 4)
                if connections.error == nil, !connections.entries.isEmpty {
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
        if connections.entries.isEmpty { return "Пусто" }
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
        return nil
    }

    // MARK: Compact markers

    private var markers: some View {
        HStack(spacing: 6) {
            ForEach(connections.entries) { entry in
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
        let visible = density == .expanded ? connections.entries : Array(connections.entries.prefix(regularRows))
        let hidden = connections.entries.count - visible.count
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
        return HStack(alignment: .center, spacing: 10) {
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

    // MARK: Status

    private var enabledEntries: [StorageEntry] { connections.entries.filter(\.enabled) }
    private var enabledCount: Int { enabledEntries.count }
    private var readyCount: Int { enabledEntries.filter { status(for: $0) == .ready }.count }

    private var aggregateStatus: ConnectionPresentationStatus {
        ConnectionPresentationStatus.aggregate(
            connections.entries.map(status(for:)),
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
