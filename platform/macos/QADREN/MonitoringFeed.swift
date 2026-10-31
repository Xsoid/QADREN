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
    /// D-024: counts of all Jobs by state; `jobs` holds only the newest ones.
    var jobCounts: [String: Int]? = nil
    var jobTotal: Int? = nil
    let jobError: String?
    let worker: WorkerReachability
    /// Batch files that the Worker runs now; `nil` when it did not answer.
    var workerActive: Int? = nil
    let capturedAt: Date

    /// Reads the JobStore inventory and sends one IPC `status` request. Both
    /// reads are non-mutating. Call this function off the main thread.
    static func capture(databasePath: String? = nil, socketPath: String? = nil) -> MonitoringSnapshot {
        var inventory: JobInventorySnapshot?
        var jobError: String?
        do {
            inventory = try databasePath.map { try JobInventory.inventory(databasePath: $0) } ?? JobInventory.inventory()
        } catch {
            jobError = error.localizedDescription
        }
        let worker: WorkerReachability
        var workerActive: Int?
        do {
            workerActive = try WorkerIPC.probeStatus(socketPath: socketPath)
            worker = .responding
        } catch WorkerIPCError.unavailable {
            worker = .unavailable
        } catch {
            worker = .notResponding
        }
        return MonitoringSnapshot(
            jobs: inventory?.recent, jobCounts: inventory?.counts, jobTotal: inventory?.total,
            jobError: jobError, worker: worker, workerActive: workerActive, capturedAt: Date()
        )
    }
}

/// Periodic read-only feed for the monitoring window. It does not own Jobs,
/// does not dispatch work, and does not retry effects.
@MainActor
final class MonitoringFeed: ObservableObject {
    static let refreshInterval: TimeInterval = 5

    @Published private(set) var jobs: [JobRow] = []
    /// Counts of all Jobs by state (D-024).
    @Published private(set) var jobCounts: [String: Int] = [:]
    @Published private(set) var jobTotal = 0
    @Published private(set) var jobError: String?
    @Published private(set) var worker: WorkerReachability = .unchecked
    @Published private(set) var workerActive: Int?
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
            jobCounts = snapshot.jobCounts ?? Dictionary(jobs.map { ($0.state, 1) }, uniquingKeysWith: +)
            jobTotal = snapshot.jobTotal ?? jobs.count
            lastInventoryRead = snapshot.capturedAt
        }
        // Keep the last good rows after a failed read; freshness marks them stale.
        jobError = snapshot.jobError
        worker = snapshot.worker
        workerActive = snapshot.workerActive
        lastWorkerCheck = snapshot.capturedAt
    }
}
