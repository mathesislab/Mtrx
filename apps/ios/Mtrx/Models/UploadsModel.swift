import Foundation
import MtrxFFI

/// The files uploading for messages sent in a room, as the library pushes
/// them: a message's bubble finds its upload by its row. Its own object,
/// so progress redraws only those bubbles, not the whole room.
@MainActor
final class UploadsModel: ObservableObject {
    @Published private(set) var uploads: [Upload] = []

    /// The subscription.
    private let work = Work()

    func follow(_ timeline: Timeline) {
        let feed = Feed<[Upload]>()
        work.follow(timeline.subscribeUploads(listener: feed), feed) { [weak self] in
            self?.uploads = $0
        }
    }

    /// How much of row `itemId`'s file is sent, from 0 to 1, while it
    /// uploads.
    func fraction(of itemId: String) -> Float? {
        uploads.first { $0.itemId == itemId }?.fraction
    }
}

extension Feed: UploadsListener where Value == [Upload] {
    func onUpdate(uploads: [Upload]) { send(uploads) }
}
