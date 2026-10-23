import Foundation

/// Result of one IPC `status` request to the local Worker. The UI shows only
/// what this request proves. A registered service is not a responding Worker.
enum WorkerReachability: Equatable, Sendable {
    case unchecked
    case responding
    /// The socket accepted the connection but no valid reply arrived in time.
    /// The Worker can be busy with a long effect or it can be stalled.
    case notResponding
    /// No Worker accepts connections on the socket.
    case unavailable

    var title: String {
        switch self {
        case .unchecked: "Worker не проверен"
        case .responding: "Worker отвечает"
        case .notResponding: "Worker занят или не отвечает"
        case .unavailable: "Worker не запущен"
        }
    }
}

/// Freshness of the monitoring data. Stale data stays visible but is marked,
/// so the panel never presents an old read as a live value.
enum MonitoringFreshness: Equatable {
    case never
    case fresh(Date)
    case stale(Date)

    /// Data is stale when no successful inventory read occurred within three
    /// refresh intervals.
    static func evaluate(lastSuccess: Date?, now: Date, interval: TimeInterval) -> MonitoringFreshness {
        guard let lastSuccess else { return .never }
        return now.timeIntervalSince(lastSuccess) > interval * 3 ? .stale(lastSuccess) : .fresh(lastSuccess)
    }
}

struct MonitoringSnapshot: Sendable {
    let jobs: [JobRow]?
    let jobError: String?
    let worker: WorkerReachability
    let capturedAt: Date

    /// Reads the JobStore inventory and sends one IPC `status` request. Both
    /// reads are non-mutating. Call this function off the main thread.
    static func capture(databasePath: String? = nil, socketPath: String? = nil) -> MonitoringSnapshot {
        var jobs: [JobRow]?
        var jobError: String?
        do {
            jobs = try databasePath.map { try JobInventory.list(databasePath: $0) } ?? JobInventory.list()
        } catch {
            jobError = error.localizedDescription
        }
        let worker: WorkerReachability
        do {
            try WorkerIPC.probeStatus(socketPath: socketPath)
            worker = .responding
        } catch WorkerIPCError.unavailable {
            worker = .unavailable
        } catch {
            worker = .notResponding
        }
        return MonitoringSnapshot(jobs: jobs, jobError: jobError, worker: worker, capturedAt: Date())
    }
}

/// Periodic read-only feed for the monitoring window. It does not own Jobs,
/// does not dispatch work, and does not retry effects.
@MainActor
final class MonitoringFeed: ObservableObject {
    static let refreshInterval: TimeInterval = 5

    @Published private(set) var jobs: [JobRow] = []
    @Published private(set) var jobError: String?
    @Published private(set) var worker: WorkerReachability = .unchecked
    @Published private(set) var lastInventoryRead: Date?
    @Published private(set) var lastWorkerCheck: Date?

    private var loop: Task<Void, Never>?
    private var refreshing = false

    func start() {
        guard loop == nil else { return }
        loop = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refresh()
                try? await Task.sleep(for: .seconds(MonitoringFeed.refreshInterval))
            }
        }
    }

    func stop() {
        loop?.cancel()
        loop = nil
    }

    func refresh() async {
        guard !refreshing else { return }
        refreshing = true
        defer { refreshing = false }
        let snapshot = await Task.detached(priority: .utility) { MonitoringSnapshot.capture() }.value
        apply(snapshot)
    }

    func apply(_ snapshot: MonitoringSnapshot) {
        if let jobs = snapshot.jobs {
            self.jobs = jobs
            lastInventoryRead = snapshot.capturedAt
        }
        // Keep the last good rows after a failed read; freshness marks them stale.
        jobError = snapshot.jobError
        worker = snapshot.worker
        lastWorkerCheck = snapshot.capturedAt
    }
}
