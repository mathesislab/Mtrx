import Foundation
import MtrxFFI

/// The downloads, as the library pushes them: a message's file finds its
/// download by its source. Downloads go to the cache, to show in QuickLook
/// or share from there. Its own object, so progress redraws only the files
/// shown, not every view of the session.
@MainActor
final class DownloadsModel: ObservableObject {
    /// Newest first.
    @Published private(set) var downloads: [Download] = []

    private let session: Session
    private let subscriptions = Subscriptions()

    init(session: Session) {
        self.session = session
    }

    func start() {
        let feed = Feed<[Download]>()
        subscriptions.add(session.subscribeDownloads(listener: feed), feed) { [weak self] in
            self?.downloads = $0
        }
    }

    /// The download of a message's file, the newest if several.
    func download(of media: Media) -> Download? {
        downloads.first { $0.source == media.source }
    }

    /// Download a message's file in `roomId` (or find it downloading or
    /// done; a failed one goes again); its ID.
    func start(_ media: Media, roomId: String, eventId: String?) throws -> UInt64 {
        if let download = download(of: media), download.stopped {
            session.retryDownload(id: download.id)
            return download.id
        }
        return try session.room(roomId: roomId).download(media: media, eventId: eventId)
    }

    /// The file of download `id` once there; nil if it failed or was
    /// cancelled (its message says so).
    func finished(_ id: UInt64) async -> URL? {
        guard let path = try? await session.downloadFinished(id: id) else { return nil }
        return URL(fileURLWithPath: path)
    }

    func cancel(_ id: UInt64) {
        session.cancelDownload(id: id)
    }

    func retry(_ id: UInt64) {
        session.retryDownload(id: id)
    }
}

extension Download {
    /// Failed or cancelled: it can go again.
    var stopped: Bool {
        switch state {
        case .failed, .cancelled: return true
        case .downloading, .done: return false
        }
    }
}
