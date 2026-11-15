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
    /// When the Worker process started; `nil` when it did not answer.
    var workerStarted: Date? = nil
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
        var workerStarted: Date?
        do {
            let status = try WorkerIPC.probeStatus(socketPath: socketPath)
            workerActive = status.active
            workerStarted = status.started.map { Date(timeIntervalSince1970: TimeInterval($0)) }
            worker = .responding
        } catch WorkerIPCError.unavailable {
            worker = .unavailable
        } catch {
            worker = .notResponding
        }
        return MonitoringSnapshot(
            jobs: inventory?.recent, jobCounts: inventory?.counts, jobTotal: inventory?.total,
            jobError: jobError, worker: worker, workerActive: workerActive,
            workerStarted: workerStarted, capturedAt: Date()
        )
    }
}

/// A JobStore inventory read that fails for three refresh intervals or more.
/// One failed read is not shown: the Worker can hold the store for a short
/// write. The Worker continues; the panel keeps the last rows it read.
struct JobStoreReadProblem: Equatable {
    let since: Date
    /// The Core error code, for diagnostics only.
    let code: String

    var message: String {
        "Не удаётся прочитать журнал заданий с \(since.formatted(date: .omitted, time: .standard)). "
            + "Worker продолжает работу, панель показывает последние прочитанные данные."
    }

    static func evaluate(error: String?, since: Date?, now: Date, interval: TimeInterval) -> JobStoreReadProblem? {
        guard let error, let since, now.timeIntervalSince(since) >= interval * 2 else { return nil }
        return JobStoreReadProblem(since: since, code: error)
    }
}

/// Periodic read-only feed for the monitoring window. It does not own Jobs,
/// does not dispatch work, and does not retry effects.
@MainActor
final class MonitoringFeed: ObservableObject {
    static let refreshInterval: TimeInterval = 5

    // D-028 rule 6: only the values that a widget shows are published, and
    // only when they change. A view that observes this feed redraws only for
    // its own data.

    /// The newest Jobs of the last good read. No widget shows them, so they
    /// are not published.
    private(set) var jobs: [JobRow] = []
    private(set) var jobError: String?
    /// The first failed inventory read of the current run of failures; `nil`
    /// after a successful read.
    private(set) var jobErrorSince: Date?
    /// A JobStore read problem that lasts two intervals. The feed evaluates it
    /// at each poll, because it depends on the time.
    @Published private(set) var jobStoreProblem: JobStoreReadProblem?
    @Published private(set) var worker: WorkerReachability = .unchecked
    @Published private(set) var workerActive: Int?
    @Published private(set) var workerStarted: Date?
    /// The service state that the feed does not read itself, such as the
    /// registration and a restart in progress. The app sets the reader; a
    /// change is published at the next poll.
    var serviceStateReader: @MainActor () -> AnyHashable? = { nil }
    @Published private(set) var serviceState: AnyHashable?
    /// Not published: a new time on each poll would redraw the observers.
    /// The "проверено" readout has its own timeline.
    private(set) var lastInventoryRead: Date?
    private(set) var lastWorkerCheck: Date?

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
        // D-028 rule 6: the panel redraws only when its data changes.
        if let jobs = snapshot.jobs {
            self.jobs = jobs
            lastInventoryRead = snapshot.capturedAt
        }
        // Keep the last good rows after a failed read; freshness marks them stale.
        jobError = snapshot.jobError
        jobErrorSince = snapshot.jobError == nil ? nil : (jobErrorSince ?? snapshot.capturedAt)
        update(\.jobStoreProblem, JobStoreReadProblem.evaluate(
            error: jobError, since: jobErrorSince, now: snapshot.capturedAt, interval: Self.refreshInterval
        ))
        update(\.serviceState, serviceStateReader())
        update(\.worker, snapshot.worker)
        update(\.workerActive, snapshot.workerActive)
        update(\.workerStarted, snapshot.workerStarted)
        lastWorkerCheck = snapshot.capturedAt
    }

    private func update<Value: Equatable>(_ property: ReferenceWritableKeyPath<MonitoringFeed, Value>, _ value: Value) {
        if self[keyPath: property] != value { self[keyPath: property] = value }
    }
}
