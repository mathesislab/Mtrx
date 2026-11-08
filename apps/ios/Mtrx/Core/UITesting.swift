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
/// a room from when it first opens; with `-ui-testing-arrivals-papers`, in
/// Papers only, also once another room opens over it. With `-ui-testing-history`, a room opens with its newest
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
    /// The LaTeX of the Lab's displayed formula.
    static let displayedTex = "\\frac{a}{b} = c"
}

/// The other person of the direct chats and messages.
private var annUser: User { User(userId: "@ann:example.org", name: "Ann", avatarUrl: nil, displayName: "Ann") }

private final class UITestCore: Mtrx, @unchecked Sendable {
    override func restore() async throws -> Session? {
        UITestSession(noHandle: .init())
    }
}

private final class UITestHandle: TaskHandle, @unchecked Sendable {
    override func cancel() {}
}

private final class UITestSession: Session, @unchecked Sendable {
    private let lock = NSLock()
    private var drafts: [String: Draft] = [:]
    private var timelines: [String: UITestTimeline] = [:]
    private var listener: RoomListsListener?
    /// The rooms, in any order.
    private var rooms = [
        UITestSession.summary(UITesting.lab, "Lab"),
        UITestSession.summary(UITesting.ann, "Ann", kind: .direct),
        UITestSession.summary(UITesting.models, "Models", unread: 2, ago: 600),
        UITestSession.summary(UITesting.papers, "Papers", ago: 1_200),
        UITestSession.summary(UITesting.team, "Team", kind: .space, ago: 3_600),
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

    override func subscribeVerificationRequests(listener: VerificationRequestListener) -> TaskHandle {
        UITestHandle(noHandle: .init())
    }

    override func subscribeDownloads(listener: DownloadsListener) -> TaskHandle {
        UITestHandle(noHandle: .init())
    }

    override func subscribeRoomLists(listener: RoomListsListener) async throws -> TaskHandle {
        let lists = lock.withLock {
            self.listener = listener
            return roomLists()
        }
        listener.onUpdate(lists: lists)
        return UITestHandle(noHandle: .init())
    }

    /// The lists as the library makes them from the rooms, most recently
    /// active first.
    private func roomLists() -> RoomLists {
        let ordered = rooms.sorted { ($0.lastActivity ?? .distantPast) > ($1.lastActivity ?? .distantPast) }
        // The rooms are made up valid.
        return try! RoomLists(
            rooms: ordered, loaded: true, ownUserId: userId(), notesAtTop: true,
            spaceChildren: [UITesting.team: [UITesting.ann, UITesting.models, UITesting.papers]], spaceMembers: [:],
            drafts: [], favourites: [], lowPriority: [], categories: [], joining: [])
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

    private static func summary(
        _ id: String, _ name: String, kind: RoomKind = .room, unread: UInt64 = 0, ago: TimeInterval = 0
    ) -> RoomSummary {
        RoomSummary(
            id: id, name: name, avatarUrl: nil, topic: .none, kind: kind, membership: .joined, encrypted: false,
            unreadMessages: unread, unreadMentions: 0, unreadNotifications: 0,
            lastActivity: Date().addingTimeInterval(-ago), directUser: kind == .direct ? annUser : nil,
            alias: nil, joinedMembers: 3, joinRule: .invite,
            joinRuleFact: "Private: by invitation", pinned: [], rights: noRights)
    }

    /// Nothing to change: as before the room's power levels are known.
    private static let noRights = Rights(
        send: false, redactOthers: false, pin: false, mentionRoom: false, rename: false, changeTopic: false,
        changePicture: false, invite: false, kick: false, ban: false, changeJoinRule: false,
        changeHistory: false, changeAddress: false, changeLevels: false, encrypt: false, addRooms: false,
        setCallService: false, level: 0, defaultLevel: 0)

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

    override func listedSpaceChildren(spaceId: String) -> [DirectoryRoom]? { nil }

    override func sharedRooms(userId: String) async throws -> [String] {
        [UITesting.lab, UITesting.ann, UITesting.models, UITesting.papers, UITesting.team]
    }

    override func startSync() async {}
    override func stopSync() async {}
    override func reconnect() async {}
    override func presenceActivity(activity: PresenceActivity) async throws {}
    override func setPresenceSharing(on: Bool) async throws {}

    override func presence(userId: String) async throws -> Presence {
        throw MtrxError.Other(reason: "No presence here")
    }

    override func subscribePresence(userIds: [String], listener: PresenceListener) throws -> TaskHandle {
        UITestHandle(noHandle: .init())
    }

    override func devices() async throws -> [DeviceInfo] { [] }

    override func timeline(roomId: String) async throws -> Timeline { timeline(roomId) }

    override func room(roomId: String) throws -> Room { UITestRoom(noHandle: .init()) }

    override func draftKeeper(roomId: String) -> DraftKeeper {
        let keeper = UITestDraftKeeper(noHandle: .init())
        keeper.keep(roomId, in: self)
        return keeper
    }

    /// The draft kept for room `roomId`.
    fileprivate func keptDraft(_ roomId: String) -> Draft? {
        lock.withLock { drafts[roomId] }
    }

    fileprivate func keepDraft(_ draft: Draft, for roomId: String) {
        lock.withLock { drafts[roomId] = draft }
    }
}

/// A room's settings and members: none here.
private final class UITestRoom: Room, @unchecked Sendable {
    override func subscribeSettings(listener: RoomSettingsListener) -> TaskHandle {
        UITestHandle(noHandle: .init())
    }

    override func members() async throws -> [Member] { [] }
}

/// A room's draft, kept in the session as it changes while the app runs.
private final class UITestDraftKeeper: DraftKeeper, @unchecked Sendable {
    private let lock = NSLock()
    private weak var session: UITestSession?
    private var room = ""
    private var restored = false

    /// Room `room`'s keeper; called once, before use.
    func keep(_ room: String, in session: UITestSession) {
        self.room = room
        self.session = session
    }

    override func restore() async -> Draft? {
        let first = lock.withLock {
            defer { restored = true }
            return !restored
        }
        return first ? session?.keptDraft(room) : nil
    }

    override func changed(draft: Draft) {
        session?.keepDraft(draft, for: room)
    }

    override func flush() async {}
}

/// A room's messages: Ann's and ours, of one to seven lines, the last two
/// not read yet; in the Lab "Message 0, a line of it." and on, then Ann's
/// formatted message and her message with math, the last one ours, to
/// delete; elsewhere the room's name instead of "Message".
private final class UITestTimeline: Timeline, @unchecked Sendable {
    private let lock = NSLock()
    private var room = UITesting.lab
    private var items: [TimelineRow] = []
    private weak var session: UITestSession?
    private var listener: TimelineRowsListener?
    private var sent = 0
    private var arrived = 0
    /// Messages arrive (`-ui-testing-arrivals`, `-ui-testing-arrivals-papers`).
    private var arriving = false
    /// Not loaded yet (`-ui-testing-history`), oldest first.
    private var older: [TimelineRow] = []

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
    private static func history(_ word: String, lines: (Int) -> Int = { 1 + $0 * 5 % 7 }) -> [TimelineRow] {
        let start = Date().addingTimeInterval(-3_600)
        var items = [TimelineRow(id: "day", content: .day(date: start))]
        for i in 0..<30 {
            if i == 28 {
                items.append(TimelineRow(id: "read-marker", content: .readMarker))
            }
            let text = Array(repeating: "\(word) \(i), a line of it.", count: lines(i)).joined(separator: "\n")
            items.append(message("m\(i)", text, own: i % 3 == 0, at: start.addingTimeInterval(Double(i) * 60)))
        }
        if word == "Message" {
            // Not among the tall newest of `-ui-testing-history`.
            if !UITesting.history {
                items.append(
                    row("formatted", event(formatted, own: false, at: start.addingTimeInterval(1_860), id: "$formatted")))
                items.append(row("math", event(math, own: false, at: start.addingTimeInterval(1_880), id: "$math")))
            }
            items.append(message("mine", "Mine, to delete", own: true, at: start.addingTimeInterval(1_900)))
        }
        return items
    }

    private static func message(_ id: String, _ text: String, own: Bool, at: Date, sent: Bool = true) -> TimelineRow {
        let message = Message(
            kind: .text, body: text, edited: false, media: nil, reactions: [], replyTo: nil, rawBody: text,
            text: [paragraph([.text(text: text, style: .plain, link: nil)])])
        return row(id, event(message, own: own, at: at, id: sent ? "$\(id)" : nil))
    }

    /// Ours or Ann's `message`, sent unless it has no event ID.
    private static func event(_ message: Message, own: Bool, at: Date, id: String?) -> TimelineEvent {
        let sender = own ? User(userId: "@me:example.org", name: "Me", avatarUrl: nil, displayName: "Me") : annUser
        return TimelineEvent(
            eventId: id, sender: sender, timestamp: at, own: own,
            content: .message(message: message), sendState: id != nil ? .sent : .sending, editable: own)
    }

    private static func paragraph(_ runs: [TextRun]) -> TextBlock {
        TextBlock(kind: .paragraph(runs: runs), quotes: 0, lists: 0, marker: nil)
    }

    /// Bold, a link and inline code, then a list item.
    private static var formatted: Message {
        let bold = TextStyle(
            bold: true, italic: false, strikethrough: false, underline: false, superscript: false,
            subscript: false, spoiler: false)
        return Message(
            kind: .text, body: "Formatted: bold, a link and code\n- an item", edited: false, media: nil,
            text: [
                paragraph([
                    .text(text: "Formatted: ", style: .plain, link: nil),
                    .text(text: "bold", style: bold, link: nil),
                    .text(text: ", a ", style: .plain, link: nil),
                    .text(text: "link", style: .plain, link: "https://example.org"),
                    .text(text: " and ", style: .plain, link: nil),
                    .code(code: "code", style: .plain, link: nil),
                ]),
                TextBlock(
                    kind: .paragraph(runs: [.text(text: "an item", style: .plain, link: nil)]), quotes: 0,
                    lists: 1, marker: "•"),
            ])
    }

    /// Inline math, then display math (`UITesting.displayedTex`).
    private static var math: Message {
        Message(
            kind: .text, body: "Energy $E=mc^2$ and $$\(UITesting.displayedTex)$$", edited: false, media: nil,
            text: [
                paragraph([
                    .text(text: "Energy ", style: .plain, link: nil),
                    .math(formula: Formula(tex: "E=mc^2", display: false), link: nil),
                ]),
                TextBlock(
                    kind: .math(formula: Formula(tex: UITesting.displayedTex, display: true)), quotes: 0, lists: 0,
                    marker: nil),
            ])
    }

    /// The row of `event`, a group of its own, with what the library offers
    /// for it.
    private static func row(_ id: String, _ event: TimelineEvent) -> TimelineRow {
        let sent = event.eventId != nil
        var actions: [MessageAction] = []
        if case .message = event.content {
            actions += (sent ? [.react, .reply] : []) + [.copy, .selectText]
        }
        if event.editable {
            actions.append(.edit)
        }
        if event.own && sent && event.content != .redacted {
            actions.append(.delete)
        }
        return TimelineRow(
            id: id,
            content: .message(
                event: event, firstInGroup: true, showSender: !event.own, avatarColumn: !event.own,
                decrypting: false, actions: actions))
    }

    /// Change the rows and give them to the listener.
    private func change(_ change: (inout [TimelineRow]) -> Void) {
        let (items, listener) = lock.withLock {
            change(&self.items)
            return (self.items, self.listener)
        }
        listener?.onUpdate(rows: items)
    }

    override func roomId() -> String { lock.withLock { room } }

    override func subscribeRows(listener: TimelineRowsListener) -> TaskHandle {
        let (items, starts) = lock.withLock {
            self.listener = listener
            let arrives = UITesting.arrivals || (UITesting.arrivalsInPapers && room == UITesting.papers)
            let starts = arrives && !arriving
            arriving = arriving || arrives
            return (self.items, starts)
        }
        listener.onUpdate(rows: items)
        if starts {
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
                var event = items[at].messageRow?.event
            else { return }
            event.content = .redacted
            event.editable = false
            items[at] = Self.row(itemId, event)
        }
    }

    override func react(itemId: String, key: String) async throws {}
    override func setTyping(typing: Bool) async throws {}
    override func markRead() async throws {}
    /// A message among the rows shown; nothing older is looked for.
    override func seek(eventId: String) async throws -> String {
        let row = lock.withLock { items.first { $0.messageRow?.event.eventId == eventId } }
        guard let row else { throw MtrxError.EventNotFound }
        return row.id
    }

    /// A message among the rows shown; for any other, waits until cancelled.
    override func itemOf(eventId: String) async throws -> String {
        let row = lock.withLock { items.first { $0.messageRow?.event.eventId == eventId } }
        if let row { return row.id }
        while !Task.isCancelled { try? await Task.sleep(nanoseconds: 100_000_000) }
        throw CancellationError()
    }
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
