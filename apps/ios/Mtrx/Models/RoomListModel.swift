import Combine
import MtrxFFI

/// The account's rooms as the library lists them: the chats, the rooms by
/// space, drafts, rooms coming and unread messages. Only the views that
/// show rooms observe it, so only they are drawn again when the lists
/// change.
@MainActor
final class RoomListModel: ObservableObject {
    /// Most recently active first.
    @Published private(set) var rooms: [RoomSummary] = []
    /// Synced with the server since start; before, `rooms` may be a stale
    /// cache, or empty on the first sign-in.
    @Published private(set) var loaded = false
    /// The chats, and the rooms by space, as the library lists them.
    @Published private(set) var lists = RoomLists.empty()
    /// The rooms with a draft on this device.
    @Published private(set) var drafts: Set<String> = []
    /// Rooms and spaces just made or joined here, not listed as joined
    /// yet (the library's): shown as coming rather than gone.
    @Published private(set) var joining: Set<String> = []
    /// Shows why a change failed (the session's alert).
    var failed: @MainActor (String) -> Void = { _ in }

    private let session: Session

    init(session: Session) {
        self.session = session
    }

    /// The library's lists, as its subscription pushes them.
    func apply(_ lists: RoomLists) {
        rooms = lists.rooms()
        loaded = lists.loaded()
        drafts = Set(lists.drafts())
        joining = Set(lists.joining())
        self.lists = lists
    }

    func room(_ id: String) -> RoomSummary? {
        rooms.first { $0.id == id }
    }

    /// What a listed room's row offers, with our membership as the room
    /// lists have it: joined once joining here, asked once asked.
    func button(for room: DirectoryRoom) -> JoinButton {
        lists.joinButton(room: room)
    }

    /// The Chats tab's badge: unread messages in the direct chats.
    var chatsBadge: Int { Int(clamping: lists.badges().chats) }

    /// The Rooms tab's badge: unread messages in the joined rooms.
    var roomsBadge: Int { Int(clamping: lists.badges().rooms) }

    /// The Spaces tab's badge: unread messages in the joined rooms in
    /// spaces.
    var spacesBadge: Int { Int(clamping: lists.badges().spaces) }

    /// The number on a room's or chat's back button: unread messages in
    /// the other joined rooms and direct chats.
    func backBadge(besides id: String) -> Int {
        Int(clamping: lists.unreadBesides(roomId: id))
    }

    /// The Notes to self stays at the top of Chats and of every space (for
    /// all the user's devices).
    var notesAtTop: Bool {
        lists.notesAtTop()
    }

    /// Keep the Notes to self at the top, or not: the library shows it so
    /// at once (if it fails, with why).
    @discardableResult
    func setNotesAtTop(_ on: Bool) -> Task<Void, Never> {
        let session = self.session
        return Task {
            do {
                try await session.setNotesAtTop(on: on)
            } catch {
                if let message = errorMessage(error) {
                    failed(message)
                }
            }
        }
    }
}
