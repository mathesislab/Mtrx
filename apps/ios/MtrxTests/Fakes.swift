import AuthenticationServices
import Foundation
import MtrxFFI
import XCTest
@testable import Mtrx

// Fakes of the library's objects, made the way UniFFI provides for: the
// generated classes are open, and `init(noHandle:)` makes one without a
// Rust object behind it. Only overridden methods may be called. They hold
// unsynchronised state, used from the tests' main actor only.

final class FakeTaskHandle: TaskHandle, @unchecked Sendable {
    var cancelled = false
    override func cancel() { cancelled = true }
    override func isFinished() -> Bool { cancelled }
}

final class FakeTimeline: Timeline, @unchecked Sendable {
    var listener: TimelineRowsListener?
    var typingListener: TypingListener?
    var typingSubscription: FakeTaskHandle?
    var sent: [String] = []
    var sendError: Error?
    /// What successive `paginateBack` calls return (then: the start).
    var pages: [Bool] = []
    var paginations = 0
    /// Called with the number of each `paginateBack` call (from 1), e.g. to
    /// give the listener older rows.
    var onPaginate: ((Int) -> Void)?
    var markedRead = 0
    var replies: [(itemId: String, text: String)] = []
    var edits: [(itemId: String, text: String)] = []
    var reactions: [(itemId: String, key: String)] = []
    var deleted: [String] = []
    var retried: [String] = []
    var discarded: [String] = []
    var typing: [Bool] = []
    var actionError: Error?
    /// Given to the listener as it subscribes, if any.
    var initialRows: [TimelineRow] = []

    override func roomId() -> String { "!room:example.org" }

    override func subscribeRows(listener: TimelineRowsListener) -> TaskHandle {
        self.listener = listener
        if !initialRows.isEmpty {
            listener.onUpdate(rows: initialRows)
        }
        return FakeTaskHandle(noHandle: .init())
    }

    override func subscribeTyping(listener: TypingListener) -> TaskHandle {
        typingListener = listener
        let handle = FakeTaskHandle(noHandle: .init())
        typingSubscription = handle
        return handle
    }

    override func sendText(text: String) async throws {
        if let sendError { throw sendError }
        sent.append(text)
    }

    override func reply(itemId: String, text: String) async throws {
        if let sendError { throw sendError }
        replies.append((itemId, text))
    }

    override func edit(itemId: String, text: String) async throws {
        if let sendError { throw sendError }
        edits.append((itemId, text))
    }

    override func react(itemId: String, key: String) async throws {
        if let actionError { throw actionError }
        reactions.append((itemId, key))
    }

    override func delete(itemId: String) async throws {
        if let actionError { throw actionError }
        deleted.append(itemId)
    }

    override func retry(itemId: String) async throws {
        if let actionError { throw actionError }
        retried.append(itemId)
    }

    override func discard(itemId: String) async throws {
        if let actionError { throw actionError }
        discarded.append(itemId)
    }

    override func setTyping(typing: Bool) async throws {
        self.typing.append(typing)
    }

    override func paginateBack(events: UInt16) async throws -> Bool {
        paginations += 1
        onPaginate?(paginations)
        return pages.isEmpty ? true : pages.removeFirst()
    }

    override func markRead() async throws {
        markedRead += 1
    }
}

final class FakeVerification: Verification, @unchecked Sendable {
    var user = "@me:example.org"
    var initialState = VerificationState.waiting
    var listener: VerificationListener?
    var subscription: FakeTaskHandle?
    var accepted = 0
    var confirmed = 0
    var mismatched = 0
    var cancelled = 0
    var answerError: Error?

    override func otherUser() -> String { user }
    override func state() -> VerificationState { initialState }

    override func subscribe(listener: VerificationListener) -> TaskHandle {
        self.listener = listener
        let handle = FakeTaskHandle(noHandle: .init())
        subscription = handle
        return handle
    }

    override func accept() async throws {
        if let answerError { throw answerError }
        accepted += 1
    }

    override func confirm() async throws {
        if let answerError { throw answerError }
        confirmed += 1
    }

    override func mismatch() async throws {
        mismatched += 1
    }

    override func cancel() async throws {
        cancelled += 1
    }
}

final class FakeSession: Session, @unchecked Sendable {
    let fakeTimeline = FakeTimeline(noHandle: .init())
    var roomListListener: RoomListListener?
    var connectionListener: ConnectionListener?
    var verificationRequestListener: VerificationRequestListener?
    var syncStarts = 0
    var syncStops = 0
    /// What `encryptionState()` returns, and the subscription's first state.
    var encryption = EncryptionState(deviceVerified: false, recovery: .unknown)
    var encryptionRequests = 0
    var encryptionListener: EncryptionStateListener?
    var encryptionSubscription: FakeTaskHandle?
    /// What `userProfile` returns, and for whom it was asked.
    var profile: Result<UserProfile, Error> = .failure(MtrxError.Other(reason: "No connection"))
    var profileRequests: [String] = []
    var invites: [(roomId: String, userId: String)] = []
    var inviteError: Error?
    var deviceList: [DeviceInfo] = []
    /// What `recover` reports, then its outcome.
    var recoverySteps: [RecoveryStep] = []
    var recoverError: Error?
    var recoveredWith: String?
    var selfVerification = FakeVerification(noHandle: .init())
    /// What `presence` returns, and for whom it was asked.
    var presenceResult: Result<Presence, Error> = .failure(MtrxError.Other(reason: "No connection"))
    var presenceRequests: [String] = []
    var setPresenceError: Error?
    /// Each `subscribePresence`, in order.
    var presenceSubscriptions: [(users: [String], listener: PresenceListener, handle: FakeTaskHandle)] = []
    /// The calls that change the session's state on the server, in order:
    /// "startSync", "stopSync", "presence <state>", "logout". Our presence
    /// may be set while sync starts, on another thread.
    var calls: [String] {
        lock.lock()
        defer { lock.unlock() }
        return log
    }
    private var log: [String] = []
    private let lock = NSLock()

    private func record(_ call: String) {
        lock.lock()
        log.append(call)
        lock.unlock()
    }

    /// Our presence as set, in order.
    var presencesSet: [String] {
        calls.filter { $0.hasPrefix("presence ") }.map { String($0.dropFirst("presence ".count)) }
    }

    override func userId() -> String { "@me:example.org" }
    override func deviceId() -> String { "DEVICE" }
    override func homeserver() -> String { "https://example.org" }

    override func timeline(roomId: String) async throws -> Timeline {
        fakeTimeline
    }

    override func subscribeRoomList(listener: RoomListListener) async throws -> TaskHandle {
        roomListListener = listener
        return FakeTaskHandle(noHandle: .init())
    }

    override func subscribeConnection(listener: ConnectionListener) -> TaskHandle {
        connectionListener = listener
        return FakeTaskHandle(noHandle: .init())
    }

    override func subscribeVerificationRequests(listener: VerificationRequestListener) -> TaskHandle {
        verificationRequestListener = listener
        return FakeTaskHandle(noHandle: .init())
    }

    override func subscribeEncryptionState(listener: EncryptionStateListener) -> TaskHandle {
        encryptionListener = listener
        listener.onUpdate(state: encryption)
        let handle = FakeTaskHandle(noHandle: .init())
        encryptionSubscription = handle
        return handle
    }

    override func userProfile(userId: String) async throws -> UserProfile {
        profileRequests.append(userId)
        return try profile.get()
    }

    override func invite(roomId: String, userId: String) async throws {
        if let inviteError { throw inviteError }
        invites.append((roomId, userId))
    }

    override func startSync() async {
        syncStarts += 1
        record("startSync")
    }

    override func stopSync() async {
        syncStops += 1
        record("stopSync")
    }

    override func presence(userId: String) async throws -> Presence {
        presenceRequests.append(userId)
        return try presenceResult.get()
    }

    override func setPresence(state: PresenceState, statusMessage: String?) async throws {
        if let setPresenceError { throw setPresenceError }
        record("presence \(state)")
    }

    override func subscribePresence(userIds: [String], listener: PresenceListener) throws -> TaskHandle {
        let handle = FakeTaskHandle(noHandle: .init())
        presenceSubscriptions.append((userIds, listener, handle))
        return handle
    }

    override func encryptionState() async throws -> EncryptionState {
        encryptionRequests += 1
        return encryption
    }

    override func devices() async throws -> [DeviceInfo] { deviceList }

    override func recover(recoveryKey: String, listener: RecoveryListener) async throws {
        recoveredWith = recoveryKey
        for step in recoverySteps {
            listener.onUpdate(step: step)
        }
        if let recoverError { throw recoverError }
    }

    override func requestSelfVerification() async throws -> Verification {
        selfVerification
    }

    override func logout() async throws {
        record("logout")
    }

    override func forget() async throws {}

    // MARK: Drafts

    // Saved off the main actor (the model's tasks call the library from
    // there) while the tests read them: behind the lock.
    private var draftStore: [String: Draft] = [:]
    private var draftSaves: [(roomId: String, draft: Draft)] = []
    private var draftFailure: Error?

    private func locked<T>(_ body: () throws -> T) rethrows -> T {
        lock.lock()
        defer { lock.unlock() }
        return try body()
    }

    /// The drafts kept, by room ID.
    var drafts: [String: Draft] {
        get { locked { draftStore } }
        set { locked { draftStore = newValue } }
    }

    /// Each `saveDraft`, in order.
    var savedDrafts: [(roomId: String, draft: Draft)] {
        locked { draftSaves }
    }

    var draftError: Error? {
        get { locked { draftFailure } }
        set { locked { draftFailure = newValue } }
    }

    override func draft(roomId: String) async throws -> Draft? {
        drafts[roomId]
    }

    override func saveDraft(roomId: String, draft: Draft) async throws {
        try locked { () throws -> Void in
            if let draftFailure { throw draftFailure }
            draftSaves.append((roomId, draft))
            let empty = draft.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && draft.replyTo == nil
            draftStore[roomId] = empty ? nil : draft
        }
    }

    // MARK: Rooms: settings, members, pins, joining, making

    let fakePinned = FakeTimeline(noHandle: .init())
    /// Each `roomSettings` call returns this (after the changes made).
    var settings = Make.settings()
    var memberList: [Member] = []
    /// Changes asked for, in order, e.g. "notify mentions", "pin $e true".
    var changes: [String] = []
    /// Thrown by the calls that change something, if set.
    var changeError: Error?
    var children: Result<[DirectoryRoom], Error> = .success([])
    var childrenRequests = 0
    /// What `sharedRooms` returns.
    var shared: [String] = []
    var directory: Result<DirectoryPage, Error> = .success(DirectoryPage(rooms: [], next: nil))
    /// Each `publicRooms` call's server, query and kind.
    var directoryRequests: [(server: String?, query: String, kind: DirectoryKind)] = []
    var browsedServers: [String] = []
    var previewResult: Result<DirectoryRoom, Error> = .failure(MtrxError.Other(reason: "Not found"))
    var offersOwnersResult = false
    var created: [NewRoom] = []

    private func change(_ change: String) throws {
        if let changeError { throw changeError }
        changes.append(change)
    }

    override func setPinned(roomId: String, eventId: String, pin: Bool) async throws {
        try change("pin \(eventId) \(pin)")
    }

    override func pinnedTimeline(roomId: String) async throws -> Timeline { fakePinned }

    override func roomSettings(roomId: String) async throws -> RoomSettings { settings }

    override func members(roomId: String) async throws -> [Member] { memberList }

    override func setNotifyMode(roomId: String, mode: NotifyMode) async throws {
        try change("notify \(mode)")
        settings.notify = mode
    }

    override func setRoomAccess(roomId: String, access: Access) async throws {
        try change("access \(access)")
        settings.access = access
    }

    override func setRoomName(roomId: String, name: String) async throws { try change("name \(name)") }

    override func setRoomTopic(roomId: String, topic: String) async throws { try change("topic \(topic)") }

    override func setAddress(roomId: String, name: String?) async throws { try change("address \(name ?? "-")") }

    override func setListed(roomId: String, listed: Bool) async throws { try change("listed \(listed)") }

    override func setHistory(roomId: String, history: History) async throws { try change("history \(history)") }

    override func setPermissions(roomId: String, permissions: Permissions) async throws {
        try change("permissions \(permissions)")
    }

    override func encryptRoom(roomId: String) async throws { try change("encrypt") }

    override func setSuggested(spaceId: String, roomId: String, suggested: Bool) async throws {
        try change("suggested \(spaceId) \(suggested)")
    }

    override func addToSpace(spaceId: String, roomId: String) async throws { try change("add to \(spaceId)") }

    override func removeFromSpace(spaceId: String, roomId: String) async throws {
        try change("remove from \(spaceId)")
    }

    override func setLevel(roomId: String, userId: String, level: Int64) async throws {
        try change("level \(userId) \(level)")
    }

    override func removeMember(roomId: String, userId: String, reason: String?, ban: Bool) async throws {
        try change("\(ban ? "ban" : "remove") \(userId) \(reason ?? "-")")
    }

    override func unban(roomId: String, userId: String) async throws { try change("unban \(userId)") }

    override func setNotesAtTop(on: Bool) async throws { try change("notes at top \(on)") }

    override func leave(roomId: String) async throws { try change("leave \(roomId)") }

    override func join(room: String, via: [String]) async throws -> String {
        try change("join \(room)")
        return room.hasPrefix("!") ? room : "!joined:example.org"
    }

    override func knock(room: String, via: [String]) async throws -> String {
        try change("knock \(room)")
        return room
    }

    override func sharedRooms(userId: String) async throws -> [String] {
        shared
    }

    override func spaceChildren(spaceId: String) async throws -> [DirectoryRoom] {
        childrenRequests += 1
        return try children.get()
    }

    override func publicRooms(server: String?, query: String, kind: DirectoryKind, page: String?) async throws
        -> DirectoryPage
    {
        directoryRequests.append((server, query, kind))
        return try directory.get()
    }

    override func preview(address: Address) async throws -> DirectoryRoom { try previewResult.get() }

    override func directoryServers() -> [String] { ["other.org"] }

    override func rememberDirectoryServer(server: String) async throws { browsedServers.append(server) }

    override func offersOwners() async -> Bool { offersOwnersResult }

    override func createRoom(new: NewRoom) async throws -> String {
        if let changeError { throw changeError }
        created.append(new)
        return "!new:example.org"
    }
}

final class FakeOAuthLogin: OAuthLogin, @unchecked Sendable {
    var session: Session?
    var finishedWith: String?

    override func url() -> String { "https://auth.example.org/login" }

    override func finish(redirectUrl: String) async throws -> Session {
        finishedWith = redirectUrl
        guard let session else { throw MtrxError.LoginCancelled }
        return session
    }
}

final class FakeMtrx: Mtrx, @unchecked Sendable {
    var oauth = false
    var loginResult: Result<Session, Error> = .failure(MtrxError.Other(reason: "Invalid password"))
    let oauthLogin = FakeOAuthLogin(noHandle: .init())

    override func supportsOauth(homeserver: String) async throws -> Bool { oauth }

    override func login(homeserver: String, user: String, password: String) async throws -> Session {
        try loginResult.get()
    }

    override func startOauthLogin(homeserver: String) async throws -> OAuthLogin {
        oauthLogin
    }
}

@MainActor
final class FakeBrowser: BrowserSignIn {
    var result: Result<URL, Error> = .failure(ASWebAuthenticationSessionError(.canceledLogin))
    var opened: URL?

    func signIn(at url: URL, callbackScheme: String) async throws -> URL {
        opened = url
        return try result.get()
    }
}

/// Wait for `condition` (e.g. a listener's update reaching the main actor),
/// at most `seconds`.
@MainActor
func eventually(
    seconds: Double = 1, _ condition: @MainActor () -> Bool, file: StaticString = #filePath, line: UInt = #line
) async {
    for _ in 0..<Int(seconds * 200) {
        if condition() { return }
        try? await Task.sleep(nanoseconds: 5_000_000)
    }
    XCTFail("Condition not met in time", file: file, line: line)
}

enum Make {
    static let start = Date(timeIntervalSince1970: 1_700_000_000)

    static func event(
        _ sender: String, _ content: EventContent, at minutes: Double = 0, own: Bool = false,
        state: SendState = .sent, eventId: String? = nil, editable: Bool = false, readBy: [Reader] = []
    ) -> TimelineEvent {
        TimelineEvent(
            eventId: eventId, senderId: "@\(sender):example.org", senderName: sender.capitalized,
            senderAvatarUrl: nil, timestamp: start.addingTimeInterval(minutes * 60), own: own, content: content,
            sendState: state, editable: editable, readBy: readBy)
    }

    static func text(
        _ body: String, edited: Bool = false, reactions: [Reaction] = [], replyTo: ReplyQuote? = nil,
        rawBody: String = ""
    ) -> EventContent {
        .message(
            message: Message(
                kind: .text, body: body, edited: edited, media: nil, reactions: reactions, replyTo: replyTo,
                rawBody: rawBody))
    }

    static func presence(
        _ state: PresenceState, lastActive: Date? = nil, status: String? = nil
    ) -> Presence {
        Presence(state: state, lastActive: lastActive, currentlyActive: state == .online, statusMessage: status)
    }

    static func reader(_ name: String, named: Bool = true) -> Reader {
        Reader(userId: "@\(name):example.org", name: named ? name.capitalized : nil, avatarUrl: nil)
    }

    /// The row of `event`, as the library gives it: the first of its group
    /// unless said otherwise, with `actions` (else those of a message in a
    /// room where we may not pin).
    static func row(
        _ id: String, _ event: TimelineEvent, firstInGroup: Bool = true, direct: Bool = false,
        decrypting: Bool = false, actions: [MessageAction]? = nil
    ) -> TimelineRow {
        let others = !event.own && !direct
        return TimelineRow(
            id: id,
            content: .message(
                event: event, firstInGroup: firstInGroup, showSender: firstInGroup && others, avatarColumn: others,
                decrypting: decrypting, actions: actions ?? Self.actions(event)))
    }

    /// What holding a message offers, as the library says.
    static func actions(_ event: TimelineEvent) -> [MessageAction] {
        let sent = event.eventId != nil
        var actions: [MessageAction] = []
        if case .message(let message) = event.content {
            actions += (sent ? [.react, .reply] : []) + (message.body.isEmpty ? [] : [.copy, .selectText])
        }
        if event.editable {
            actions.append(.edit)
        }
        if event.own && sent && event.content != .redacted {
            actions.append(.delete)
        }
        if case .failed = event.sendState {
            actions += [.retry, .discard]
        }
        return actions
    }

    static func room(
        _ id: String, kind: RoomKind = .room, invited: Bool = false, name: String? = nil, unread: UInt64 = 0,
        directUser: String? = nil, lastActivity: Date? = nil, avatarUrl: String? = nil, topic: String? = nil,
        parents: [String] = [], pinned: [String] = [], alias: String? = nil, members: UInt64 = 2,
        mentions: UInt64 = 0
    ) -> RoomSummary {
        RoomSummary(
            id: id, name: name ?? id, avatarUrl: avatarUrl, topic: topic, kind: kind, invited: invited,
            encrypted: false, unreadMessages: unread, unreadMentions: mentions, unreadNotifications: 0,
            lastActivity: lastActivity, directUserId: directUser, alias: alias, joinedMembers: members,
            parents: parents, public: false, pinned: pinned)
    }

    /// A room's settings, all ours to change unless said otherwise.
    static func settings(
        canChange: Bool = true, access: Access = .invite, encrypted: Bool = true, listed: Bool? = false,
        spaces: [SpaceLink] = [], canAddTo: [String] = [], banned: [Banned] = [], address: String? = nil
    ) -> RoomSettings {
        RoomSettings(
            canRename: canChange, canChangeTopic: canChange, canChangePicture: canChange, canInvite: canChange,
            notify: .all, access: access, canChangeAccess: canChange, canAddRooms: canChange, address: address,
            canChangeAddress: canChange, listed: listed, history: .joined, canChangeHistory: canChange,
            permissions: .everyone, canChangePermissions: canChange, encrypted: encrypted, canEncrypt: canChange,
            version: "12", spaces: spaces, canAddTo: canAddTo, banned: banned, canUnban: canChange)
    }

    /// A member at `level` (nil: an owner); `upTo`: the highest level we
    /// may give them (nil: theirs is not ours to change). Their label and
    /// level choices as the library gives them, without what it asks.
    static func member(
        _ name: String, level: Int64?, upTo: Int64? = nil, remove: Bool = false, ban: Bool = false,
        invited: Bool = false, named: Bool = true
    ) -> Member {
        let labels: [Int64: String] = [100: "Admin", 50: "Mod", 0: ""]
        let label = level.map { labels[$0] ?? "Level \($0)" } ?? "Owner"
        let levels: [(String, Int64)] = [("Admin", 100), ("Moderator", 50), ("Member", 0)]
        let choices = upTo.map { upTo in
            levels.filter { $0.1 <= upTo || $0.1 == level }
                .map { LevelChoice(title: $0.0, level: $0.1, current: $0.1 == level, confirmation: nil) }
        }
        return Member(
            userId: "@\(name):example.org", name: named ? name.capitalized : nil, avatarUrl: nil,
            role: level == 100 || level == nil ? .admin : level == 50 ? .moderator : .member, level: level,
            invited: invited, own: name == "me",
            manage: Manage(remove: remove, ban: ban, upTo: upTo, defaultLevel: 0, alone: false),
            label: label.isEmpty ? nil : label, choices: choices ?? [],
            manageable: upTo != nil || remove || ban)
    }

    /// A section of a space's list; `key` as the library makes them.
    static func section(_ kind: SectionKind, _ roomIds: [String], name: String = "", key: String? = nil) -> RoomSection {
        let keys: [SectionKind: String] = [
            .invites: "invites", .favourites: "favourites", .directMessages: "direct_messages", .rooms: "rooms",
            .lowPriority: "low_priority", .category: "category:\(name.lowercased())",
        ]
        return RoomSection(key: key ?? keys[kind]!, kind: kind, name: name, roomIds: roomIds)
    }

    static func directoryRoom(
        _ id: String, name: String? = nil, access: JoinAccess = .public, membership: Membership? = nil,
        space: Bool = false, members: UInt64 = 5, topic: String? = nil, role: Role? = nil, children: UInt64 = 0
    ) -> DirectoryRoom {
        DirectoryRoom(
            id: id, alias: nil, name: name ?? id, topic: topic, avatarUrl: nil, members: members, isSpace: space,
            children: children, access: access, membership: membership, role: role, via: ["example.org"],
            suggested: false)
    }

    /// The rooms by space as the library's integration test makes them, most
    /// recently active first: spaces Team and Other; Lab in Team, Deep in
    /// Team's nested space Sub; Loose in no space; a direct chat with Bob, a
    /// member of Team (unread); invites to Offer, in Team, and from Carol,
    /// in no space, to a direct chat and Party. Loose is unread.
    static func roomsBySpace(loaded: Bool = true) -> RoomListState {
        RoomListState(
            rooms: [
                room("!carol", kind: .direct, invited: true, name: "Carol", directUser: "@carol:example.org"),
                room("!party", invited: true, name: "Party"),
                room("!offer", invited: true, name: "Offer"),
                room("!bob", kind: .direct, name: "Bob", unread: 2, directUser: "@bob:example.org"),
                room("!loose", name: "Loose", unread: 1),
                room("!deep", name: "Deep"),
                room("!sub", kind: .space, name: "Sub"),
                room("!lab", name: "Lab"),
                room("!team", kind: .space, name: "Team"),
                room("!other", kind: .space, name: "Other"),
            ],
            loaded: loaded,
            spaces: [
                SpaceRooms(spaceId: "!team", roomIds: ["!offer", "!bob", "!deep", "!lab"]),
                SpaceRooms(spaceId: "!other", roomIds: []),
            ],
            roomsOutsideSpaces: ["!party", "!loose"])
    }
}
