import Combine
import Foundation

private func check(_ condition: @autoclosure () -> Bool, _ message: String) {
    guard condition() else { fatalError("FAIL: " + message) }
}

@main
struct MonitoringFeedTests {
    @MainActor
    static func main() throws {
        checkFreshness()

        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("qadren-monitoring-feed-tests-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let paths = try JobInventory.paths(in: root)
        let registry = root.appendingPathComponent("registry.sqlite3").path
        // sockaddr_un limits the path length, so keep the socket in a short directory.
        let socketDirectory = URL(fileURLWithPath: "/tmp", isDirectory: true)
            .appendingPathComponent("qmf-\(UUID().uuidString.prefix(8))", isDirectory: true)
        try FileManager.default.createDirectory(
            at: socketDirectory, withIntermediateDirectories: false,
            attributes: [.posixPermissions: NSNumber(value: 0o700)]
        )
        defer { try? FileManager.default.removeItem(at: socketDirectory) }
        let socket = socketDirectory.appendingPathComponent("worker.sock").path

        var snapshot = MonitoringSnapshot.capture(databasePath: paths.database, socketPath: socket)
        check(snapshot.worker == .unavailable, "missing Worker socket is reported as unavailable")
        check(snapshot.jobs?.isEmpty == true && snapshot.jobError == nil, "empty inventory is read without error")

        try JobInventory.queueMockArtifact(jobID: "swift-monitoring-queued", databasePath: paths.database)
        let worker = Process()
        worker.executableURL = URL(fileURLWithPath: CommandLine.arguments[1])
        worker.arguments = ["--socket", socket, "--registry", registry, "--jobs", paths.database]
        try worker.run()
        defer {
            if worker.isRunning { worker.terminate() }
            worker.waitUntilExit()
        }
        for _ in 0..<200 where !FileManager.default.fileExists(atPath: socket) {
            Thread.sleep(forTimeInterval: 0.01)
        }
        check(FileManager.default.fileExists(atPath: socket), "Worker socket is available")

        snapshot = MonitoringSnapshot.capture(databasePath: paths.database, socketPath: socket)
        check(snapshot.worker == .responding, "running Worker answers the status request")
        check(
            snapshot.jobs?.contains { $0.id == "swift-monitoring-queued" && $0.state == "queued" } == true,
            "refresh reads a Job that was added after the previous read"
        )

        let feed = MonitoringFeed()
        feed.apply(snapshot)
        let failed = MonitoringSnapshot(
            jobs: nil, jobError: "Job inventory unavailable", worker: .notResponding,
            capturedAt: snapshot.capturedAt.addingTimeInterval(5)
        )
        feed.apply(failed)
        check(feed.jobs.count == 1, "failed read keeps the last good rows")
        check(feed.jobError == "Job inventory unavailable", "failed read is reported")
        check(feed.lastInventoryRead == snapshot.capturedAt, "failed read does not advance the freshness time")
        check(feed.worker == .notResponding, "Worker state follows the latest probe")

        // D-028 rule 6: a poll with the same data does not redraw the panel.
        feed.apply(snapshot)
        var changes = 0
        let observer = feed.objectWillChange.sink { changes += 1 }
        let unchanged = MonitoringSnapshot(
            jobs: snapshot.jobs, jobCounts: snapshot.jobCounts, jobTotal: snapshot.jobTotal,
            jobError: snapshot.jobError, worker: snapshot.worker,
            workerActive: snapshot.workerActive, workerStarted: snapshot.workerStarted,
            capturedAt: snapshot.capturedAt.addingTimeInterval(10)
        )
        feed.apply(unchanged)
        check(changes == 0, "a poll with unchanged data publishes no change")
        check(feed.lastWorkerCheck == unchanged.capturedAt, "the check time still advances")
        feed.apply(failed)
        check(changes > 0, "a poll with changed data publishes a change")

        // A problem that depends on time is published when it starts, even
        // when the data of the poll does not change.
        check(feed.jobStoreProblem == nil, "a short read problem is not reported yet")
        changes = 0
        let lasting = MonitoringSnapshot(
            jobs: nil, jobError: "Job inventory unavailable", worker: .notResponding,
            capturedAt: failed.capturedAt.addingTimeInterval(MonitoringFeed.refreshInterval * 2)
        )
        feed.apply(lasting)
        check(feed.jobStoreProblem != nil, "a read problem that lasts two intervals is reported")
        check(changes > 0, "the lasting problem is published")

        // The service state that the feed does not read itself is published
        // when it changes, and not on each poll.
        var restarting = false
        feed.serviceStateReader = { AnyHashable(restarting) }
        feed.apply(lasting)
        changes = 0
        feed.apply(lasting)
        check(changes == 0, "an unchanged service state publishes no change")
        restarting = true
        feed.apply(lasting)
        check(changes > 0 && feed.serviceState == AnyHashable(true), "a changed service state is published")
        observer.cancel()

        print("Monitoring feed native tests passed")
    }

    private static func checkFreshness() {
        let start = Date(timeIntervalSince1970: 1_000)
        check(
            MonitoringFreshness.evaluate(lastSuccess: nil, now: start, interval: 5) == .never,
            "no successful read is not reported as fresh"
        )
        check(
            MonitoringFreshness.evaluate(lastSuccess: start, now: start.addingTimeInterval(15), interval: 5) == .fresh(start),
            "data stays fresh for three refresh intervals"
        )
        check(
            MonitoringFreshness.evaluate(lastSuccess: start, now: start.addingTimeInterval(15.1), interval: 5) == .stale(start),
            "data becomes stale after three refresh intervals"
        )
    }
}
