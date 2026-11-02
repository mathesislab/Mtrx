#if DEBUG
import Foundation
import MtrxFFI

/// For the UI tests (launch argument `-ui-testing`, debug builds only): an
/// account signed in with made-up rooms and messages, and no server: the
/// Lab room in no space, Models and Papers in the space Team, and a chat
/// with Ann, who is in all of them. Each room has messages of its own, the
/// Lab's last one ours, to delete. A message sent shows at once, then as
/// the server's copy; one deleted shows as deleted. A message sent or
/// arrived moves its room up the list, as the library orders it. With
/// `-ui-testing-arrivals`, a message of Ann's arrives every two seconds in
/// the open room; with `-ui-testing-arrivals-papers`, in Papers, open or
/// not. With `-ui-testing-history`, a room opens with its newest
/// messages, long ones, and its older, short ones load a moment later.
/// The library's classes are faked as UniFFI provides for
/// (`init(noHandle:)`, overriding what the app calls).
enum UITesting {
    static var on: Bool {
        ProcessInfo.processInfo.arguments.contains("-ui-testing")
    }

    static var arrivals: Bool {
        ProcessInfo.processInfo.arguments.contains("-ui-testing-arrivals")
    }

    static var arrivalsInPapers: Bool {
        ProcessInfo.processInfo.arguments.contains("-ui-testing-arrivals-papers")
    }

    static var history: Bool {
        ProcessInfo.processInfo.arguments.contains("-ui-testing-history")
    }

    @MainActor
    static func app() -> AppModel {
        AppModel(core: UITestCore(noHandle: .init()), browser: WebAuthenticator())
    }

    static let lab = "!lab:example.org"
    static let models = "!models:example.org"
    static let papers = "!papers:example.org"
    static let team = "!team:example.org"
    static let ann = "!ann:example.org"
}

private final class UITestCore: Mtrx, @unchecked Sendable {
    override func restore() async throws -> Session? {
        UITestSession(noHandle: .init())
    }
}

private final class UITestHandle: TaskHandle, @unchecked Sendable {
    override func cancel() {}
    override func isFinished() -> Bool { false }
}

private final class UITestSession: Session, @unchecked Sendable {
    private let lock = NSLock()
    private var drafts: [String: Draft] = [:]
    private var timelines: [String: UITestTimeline] = [:]
    private var listener: RoomListsListener?
    /// The rooms, in any order.
    private var rooms = [
        UITestSession.room(UITesting.lab, "Lab"),
        UITestSession.room(UITesting.ann, "Ann", kind: .direct, parents: [UITesting.team]),
        UITestSession.room(UITesting.models, "Models", unread: 2, parents: [UITesting.team], ago: 600),
        UITestSession.room(UITesting.papers, "Papers", parents: [UITesting.team], ago: 1_200),
        UITestSession.room(UITesting.team, "Team", kind: .space, ago: 3_600),
    ]

    override func userId() -> String { "@me:example.org" }
    override func deviceId() -> String { "UITEST" }
    override func homeserver() -> String { "https://example.org" }

    override func subscribeConnection(listener: ConnectionListener) -> TaskHandle {
        listener.onUpdate(state: .online)
        return UITestHandle(noHandle: .init())
    }

    override func subscribeEncryptionState(listener: EncryptionStateListener) -> TaskHandle {
        listener.onUpdate(state: EncryptionState(deviceVerified: true, recovery: .enabled))
        return UITestHandle(noHandle: .init())
    }

    override func encryptionState() async throws -> EncryptionState {
        EncryptionState(deviceVerified: true, recovery: .enabled)
    }

    override func subscribeVerificationRequests(listener: VerificationRequestListener) -> TaskHandle {
        UITestHandle(noHandle: .init())
    }

    override func subscribeRoomLists(listener: RoomListsListener) async throws -> TaskHandle {
        let lists = lock.withLock {
            self.listener = listener
            return roomLists()
        }
        listener.onUpdate(lists: lists)
        if UITesting.arrivalsInPapers {
            let papers = timeline(UITesting.papers)
            Task {
                while true {
                    try? await Task.sleep(nanoseconds: 2_000_000_000)
                    papers.arrive()
                }
            }
        }
        return UITestHandle(noHandle: .init())
    }

    /// The lists as the library makes them from the rooms, most recently
    /// active first.
    private func roomLists() -> RoomLists {
        let ordered = rooms.sorted { ($0.lastActivity ?? .distantPast) > ($1.lastActivity ?? .distantPast) }
        // The rooms are made up valid.
        return try! RoomLists(
            rooms: ordered, loaded: true, ownUserId: userId(), notesAtTop: true, spaceMembers: [:], drafts: [],
            favourites: [], lowPriority: [], categories: [])
    }

    /// A message was sent or arrived in room `id`: it moves up the list.
    fileprivate func active(_ id: String) {
        let (lists, listener) = lock.withLock {
            if let at = rooms.firstIndex(where: { $0.id == id }) {
                rooms[at].lastActivity = Date()
            }
            return (roomLists(), self.listener)
        }
        listener?.onUpdate(lists: lists)
    }

    private static func room(
        _ id: String, _ name: String, kind: RoomKind = .room, unread: UInt64 = 0, parents: [String] = [],
        ago: TimeInterval = 0
    ) -> RoomSummary {
        RoomSummary(
            id: id, name: name, avatarUrl: nil, topic: nil, kind: kind, invited: false, encrypted: false,
            unreadMessages: unread, unreadMentions: 0, unreadNotifications: 0,
            lastActivity: Date().addingTimeInterval(-ago), directUserId: kind == .direct ? "@ann:example.org" : nil,
            alias: nil, joinedMembers: 3, parents: parents, public: false, pinned: [])
    }

    /// Room `id`'s timeline, the same each time.
    private func timeline(_ id: String) -> UITestTimeline {
        lock.withLock {
            if let timeline = timelines[id] { return timeline }
            let name = rooms.first { $0.id == id }?.name ?? id
            let timeline = UITestTimeline(noHandle: .init())
            timeline.fill(room: id, name: name, session: self)
            timelines[id] = timeline
            return timeline
        }
    }

    override func sharedRooms(userId: String) async throws -> [String] {
        [UITesting.lab, UITesting.ann, UITesting.models, UITesting.papers, UITesting.team]
    }

    override func startSync() async {}
    override func stopSync() async {}
    override func setPresence(state: PresenceState, statusMessage: String?) async throws {}

    override func presence(userId: String) async throws -> Presence {
        throw MtrxError.Other(reason: "No presence here")
    }

    override func subscribePresence(userIds: [String], listener: PresenceListener) throws -> TaskHandle {
        UITestHandle(noHandle: .init())
    }

    override func devices() async throws -> [DeviceInfo] { [] }

    override func timeline(roomId: String) async throws -> Timeline { timeline(roomId) }

    override func canPin(roomId: String) async throws -> Bool { false }

    override func roomSettings(roomId: String) async throws -> RoomSettings {
        throw MtrxError.Other(reason: "No settings here")
    }

    override func members(roomId: String) async throws -> [Member] { [] }

    override func draft(roomId: String) async throws -> Draft? {
        lock.withLock { drafts[roomId] }
    }

    override func saveDraft(roomId: String, draft: Draft) async throws {
        lock.withLock { drafts[roomId] = draft }
    }
}

/// A room's messages: Ann's and ours, of one to seven lines, the last two
/// not read yet; in the Lab "Message 0, a line of it." and on, the last one
/// ours, to delete; elsewhere the room's name instead of "Message".
private final class UITestTimeline: Timeline, @unchecked Sendable {
    private let lock = NSLock()
    private var room = UITesting.lab
    private var items: [TimelineItem] = []
    private weak var session: UITestSession?
    private var listener: TimelineListener?
    private var sent = 0
    private var arrived = 0
    /// Not loaded yet (`-ui-testing-history`), oldest first.
    private var older: [TimelineItem] = []

    /// Room `room`'s messages; called once, before use.
    func fill(room: String, name: String, session: UITestSession) {
        lock.withLock {
            self.room = room
            self.session = session
            let word = room == UITesting.lab ? "Message" : name
            if UITesting.history {
                // Far taller than the older ones loaded after them: a lazy
                // stack's guess at those, from these, is far off.
                items = Self.history(word) { $0 >= 26 ? 20 : 1 }
                older = Array(items.dropLast(4))
                items = Array(items.suffix(4))
            } else {
                items = Self.history(word)
            }
        }
    }

    /// `lines`: message `i`'s.
    private static func history(_ word: String, lines: (Int) -> Int = { 1 + $0 * 5 % 7 }) -> [TimelineItem] {
        let start = Date().addingTimeInterval(-3_600)
        var items = [TimelineItem(id: "day", content: .dayDivider(date: start))]
        for i in 0..<30 {
            if i == 28 {
                items.append(TimelineItem(id: "read-marker", content: .readMarker))
            }
            let text = Array(repeating: "\(word) \(i), a line of it.", count: lines(i)).joined(separator: "\n")
            items.append(message("m\(i)", text, own: i % 3 == 0, at: start.addingTimeInterval(Double(i) * 60)))
        }
        if word == "Message" {
            items.append(message("mine", "Mine, to delete", own: true, at: start.addingTimeInterval(1_900)))
        }
        return items
    }

    private static func message(_ id: String, _ text: String, own: Bool, at: Date, sent: Bool = true) -> TimelineItem {
        let message = Message(
            kind: .text, body: text, edited: false, media: nil, reactions: [], replyTo: nil, rawBody: text)
        let event = TimelineEvent(
            eventId: sent ? "$\(id)" : nil, senderId: own ? "@me:example.org" : "@ann:example.org",
            senderName: own ? "Me" : "Ann", senderAvatarUrl: nil, timestamp: at, own: own,
            content: .message(message: message), sendState: sent ? .sent : .sending, editable: own)
        return TimelineItem(id: id, content: .event(event: event))
    }

    /// Change the items and give them to the listener.
    private func change(_ change: (inout [TimelineItem]) -> Void) {
        let (items, listener) = lock.withLock {
            change(&self.items)
            return (self.items, self.listener)
        }
        listener?.onUpdate(items: items)
    }

    override func roomId() -> String { lock.withLock { room } }

    override func subscribe(listener: TimelineListener) -> TaskHandle {
        let items = lock.withLock {
            self.listener = listener
            return self.items
        }
        listener.onUpdate(items: items)
        if UITesting.arrivals {
            Task { [weak self] in
                while let self {
                    try? await Task.sleep(nanoseconds: 2_000_000_000)
                    arrive()
                }
            }
        }
        return UITestHandle(noHandle: .init())
    }

    /// One of Ann's, of one to four lines.
    func arrive() {
        let n = lock.withLock {
            arrived += 1
            return arrived
        }
        let text = Array(repeating: "Arrived \(n), a line of it.", count: 1 + n % 4).joined(separator: "\n")
        change { $0.append(Self.message("arrived\(n)", text, own: false, at: Date())) }
        session?.active(roomId())
    }

    override func subscribeTyping(listener: TypingListener) -> TaskHandle {
        UITestHandle(noHandle: .init())
    }

    /// Shown at once; read (the marker gone) and the server's copy a moment
    /// later.
    override func sendText(text: String) async throws {
        let id = lock.withLock {
            sent += 1
            return "sent\(sent)"
        }
        change { $0.append(Self.message(id, text, own: true, at: Date(), sent: false)) }
        session?.active(roomId())
        try? await Task.sleep(nanoseconds: 300_000_000)
        change { items in
            items.removeAll { $0.id == "read-marker" }
            if let at = items.firstIndex(where: { $0.id == id }) {
                items[at] = Self.message(id, text, own: true, at: Date())
            }
        }
    }

    override func delete(itemId: String) async throws {
        change { items in
            guard let at = items.firstIndex(where: { $0.id == itemId }),
                case .event(var event) = items[at].content
            else { return }
            event.content = .redacted
            event.editable = false
            items[at] = TimelineItem(id: itemId, content: .event(event: event))
        }
    }

    override func react(itemId: String, key: String) async throws {}
    override func setTyping(typing: Bool) async throws {}
    override func markRead() async throws {}
    /// With `-ui-testing-history`, `events` older messages a moment later.
    override func paginateBack(events: UInt16) async throws -> Bool {
        guard UITesting.history else { return true }
        try? await Task.sleep(nanoseconds: 500_000_000)
        var atStart = false
        change { items in
            let page = older.suffix(Int(events))
            older.removeLast(page.count)
            items.insert(contentsOf: page, at: 0)
            atStart = older.isEmpty
        }
        return atStart
    }
}
#endif
