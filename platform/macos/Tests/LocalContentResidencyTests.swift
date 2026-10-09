import Foundation

@main
struct LocalContentResidencyTests {
    static func main() {
        assert(LocalContentResidencyProbe.classify(isUbiquitous: true, status: .current) == .currentLocalUbiquitousCopy)
        assert(LocalContentResidencyProbe.classify(isUbiquitous: true, status: .downloaded) == .downloadedButStale)
        assert(LocalContentResidencyProbe.classify(isUbiquitous: true, status: .notDownloaded) == .notDownloaded)
        assert(LocalContentResidencyProbe.classify(isUbiquitous: true, status: nil) == .unknown)
        assert(LocalContentResidencyProbe.classify(isUbiquitous: false, status: .current) == .unknown)
        assert(LocalContentResidencyProbe.classify(isUbiquitous: nil, status: .current) == .unknown)
        assert(LocalContentResidencyProbe.inspect(URL(fileURLWithPath: "/nonexistent-qadren-placeholder-probe")) == .unknown)
        print("Local content residency metadata tests passed")
    }
}
