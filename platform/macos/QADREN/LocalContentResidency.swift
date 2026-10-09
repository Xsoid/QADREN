import Foundation

/// Metadata-only assessment. This does not authorize opening file content.
/// In particular, a non-ubiquitous URL may still be managed by File Provider.
enum LocalContentResidency: Equatable {
    case currentLocalUbiquitousCopy
    case downloadedButStale
    case notDownloaded
    case unknown
}

enum LocalContentResidencyProbe {
    /// Pure classifier; unknown and missing metadata must never be treated as safe.
    static func classify(isUbiquitous: Bool?, status: URLUbiquitousItemDownloadingStatus?) -> LocalContentResidency {
        guard isUbiquitous == true, let status else {
            return .unknown
        }
        switch status {
        case .current:
            return .currentLocalUbiquitousCopy
        case .downloaded:
            return .downloadedButStale
        case .notDownloaded:
            return .notDownloaded
        default:
            return .unknown
        }
    }

    /// Queries URL resource metadata only. Never opens a content stream,
    /// coordinates a read or requests materialization.
    static func inspect(_ url: URL) -> LocalContentResidency {
        guard let values = try? url.resourceValues(forKeys: [
            .isUbiquitousItemKey,
            .ubiquitousItemDownloadingStatusKey,
        ]) else {
            return .unknown
        }
        return classify(
            isUbiquitous: values.isUbiquitousItem,
            status: values.ubiquitousItemDownloadingStatus
        )
    }
}
