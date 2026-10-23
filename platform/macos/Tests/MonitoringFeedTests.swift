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
