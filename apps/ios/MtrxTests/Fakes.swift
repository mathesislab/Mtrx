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
    /// The row `seek` and `itemOf` find for each event ID; others are not
    /// found (`itemOf` waits until cancelled).
    var found: [String: String] = [:]
    var seeks: [String] = []
    /// Called with each `seek`'s event ID, e.g. to give the listener the
    /// older rows.
    var onSeek: ((String) -> Void)?
    var markedRead = 0
    var replies: [(itemId: String, text: String)] = []
    /// The names of the files sent with each message or reply, in order.
    var files: [[String]] = []
    var uploadsListener: UploadsListener?
    /// How long after read each message sent or reply disappears, in order
    /// (nil: it doesn't).
    var disappearing: [TimeInterval?] = []
    /// How each message sent or reply sends its photos, in order.
    var qualities: [MediaQuality] = []
    var edits: [(itemId: String, text: String)] = []
    var reactions: [(itemId: String, key: String)] = []
    var deleted: [String] = []
    var retried: [String] = []
    var unsent: [String] = []
    /// What `unsend` gives back.
    var unsentDraft: Draft?
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

    /// A new message's text in `sent`, a reply's in `replies`; the files of
    /// each in `files`, read while it is sent.
    override func send(
        text: String, files: [FileToSend], replyTo: String?, disappearAfterRead: TimeInterval?,
        quality: MediaQuality
    ) async throws {
        if let sendError { throw sendError }
        if let replyTo {
            replies.append((replyTo, text))
        } else {
            sent.append(text)
        }
        self.files.append(files.map(\.filename))
        XCTAssertTrue(files.allSatisfy { FileManager.default.fileExists(atPath: $0.path) })
        disappearing.append(disappearAfterRead)
        qualities.append(quality)
    }

    override func subscribeUploads(listener: UploadsListener) -> TaskHandle {
        uploadsListener = listener
        return FakeTaskHandle(noHandle: .init())
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

    override func unsend(itemId: String) async throws -> Draft? {
        if let actionError { throw actionError }
        unsent.append(itemId)
        return unsentDraft
    }

    override func setTyping(typing: Bool) async throws {
        self.typing.append(typing)
    }

    override func paginateBack(events: UInt16) async throws -> Bool {
        paginations += 1
        onPaginate?(paginations)
        return pages.isEmpty ? true : pages.removeFirst()
    }

    override func seek(eventId: String) async throws -> String {
        seeks.append(eventId)
        onSeek?(eventId)
        guard let row = found[eventId] else { throw MtrxError.EventNotFound }
        return row
    }

    override func itemOf(eventId: String) async throws -> String {
        if let row = found[eventId] { return row }
        while !Task.isCancelled { try? await Task.sleep(nanoseconds: 100_000_000) }
        throw CancellationError()
    }

    override func markRead() async throws {
        markedRead += 1
    }
}

/// A room's draft keeper over the session's drafts: the one left there,
/// once; the draft as last written, kept on `flush()`. The rules of when
/// to save are the library's, tested there.
final class FakeDraftKeeper: DraftKeeper, @unchecked Sendable {
    private let lock = NSLock()
    private var restored = false
    private var written: [Draft] = []
    var load: @Sendable () -> Draft? = { nil }
    var keep: @Sendable (Draft) -> Void = { _ in }

    /// Each draft the model said is written, in order.
    var changes: [Draft] {
        lock.withLock { written }
    }

    override func restore() async -> Draft? {
        let first = lock.withLock {
            defer { restored = true }
            return !restored
        }
        return first ? load() : nil
    }

    override func changed(draft: Draft) {
        lock.withLock { written.append(draft) }
    }

    override func flush() async {
        if let last = changes.last {
            keep(last)
        }
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
    var roomListListener: RoomListsListener?
    var connectionListener: ConnectionListener?
    var verificationRequestListener: VerificationRequestListener?
    var syncStarts = 0
    var syncStops = 0
    var reconnects = 0
    /// The subscription's first state.
    var encryption = EncryptionState(deviceVerified: false, recovery: .unknown)
    var encryptionListener: EncryptionStateListener?
    var encryptionSubscription: FakeTaskHandle?
    /// What `userProfile` returns, and for whom it was asked.
    var profile: Result<User, Error> = .failure(MtrxError.Other(reason: "No connection"))
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
    /// Fails `presenceActivity` and `setPresenceSharing`.
    var ownPresenceError: Error?
    /// `presenceActivity` and `setPresenceSharing` wait while this is set.
    var ownPresenceHangs = false
    /// Each `subscribePresence`, in order.
    var presenceSubscriptions: [(users: [String], listener: PresenceListener, handle: FakeTaskHandle)] = []
    /// The calls that change the session's state on the server, in order:
    /// "startSync", "stopSync", "presence <activity>", "sharing <on>",
    /// "logout".
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

    /// What the library was told for our presence, in order: activities
    /// and "sharing <on>".
    var presenceCalls: [String] {
        calls.filter { $0.hasPrefix("presence ") || $0.hasPrefix("sharing ") }
    }

    override func userId() -> String { "@me:example.org" }
    override func deviceId() -> String { "DEVICE" }
    override func homeserver() -> String { "https://example.org" }

    override func timeline(roomId: String) async throws -> Timeline {
        fakeTimeline
    }

    /// What `videoPlan` returns, and the videos it was asked for.
    var plan: VideoPlan?
    var planned: [OutgoingVideo.Source] = []

    override func videoPlan(
        width: UInt32, height: UInt32, fps: Float, durationMs: UInt64, size: UInt64
    ) async throws -> VideoPlan? {
        planned.append(
            OutgoingVideo.Source(width: width, height: height, fps: fps, durationMs: durationMs, size: size))
        return plan
    }

    override func subscribeRoomLists(listener: RoomListsListener) async throws -> TaskHandle {
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

    var downloadsListener: DownloadsListener?

    override func subscribeDownloads(listener: DownloadsListener) -> TaskHandle {
        downloadsListener = listener
        return FakeTaskHandle(noHandle: .init())
    }

    /// What `subscribeQuiet` gives, now and on every change; the changes
    /// asked for, as `changes` has them.
    var quietState = QuietState(
        quiet: false, paused: false, until: nil, line: "Notifications on",
        hours: QuietHours(on: false, fromMinute: 22 * 60, toMinute: 7 * 60, days: [.mon, .tue, .wed, .thu, .fri, .sat, .sun]),
        hoursLine: "Off", mentionsThrough: false
    ) {
        didSet { quietListener?.onUpdate(state: quietState) }
    }
    var quietListener: QuietListener?

    override func subscribeQuiet(listener: QuietListener) -> TaskHandle {
        quietListener = listener
        listener.onUpdate(state: quietState)
        return FakeTaskHandle(noHandle: .init())
    }

    override func pauseMenu() -> [PauseMenuItem] {
        [
            PauseMenuItem(entry: .pause(pause: .hour), title: "For 1 hour"),
            PauseMenuItem(entry: .pause(pause: .tomorrowMorning), title: "Until tomorrow, 08:00"),
            PauseMenuItem(entry: .pickTime, title: "Until…"),
            PauseMenuItem(entry: .pause(pause: .untilResumed), title: "Until I turn them back on"),
        ]
    }

    override func pauseNotifications(pause: PauseFor) async throws {
        try change("pause \(pause)")
        quietState.quiet = true
        quietState.paused = true
    }

    override func resumeNotifications() async throws {
        try change("resume")
        quietState.quiet = false
        quietState.paused = false
    }

    override func setQuietHours(hours: QuietHours) async throws {
        try change("hours \(hours.fromMinute)-\(hours.toMinute)")
        quietState.hours = hours
    }

    override func setMentionsThrough(on: Bool) async throws {
        try change("mentions through \(on)")
        quietState.mentionsThrough = on
    }

    override func subscribeEncryptionState(listener: EncryptionStateListener) -> TaskHandle {
        encryptionListener = listener
        listener.onUpdate(state: encryption)
        let handle = FakeTaskHandle(noHandle: .init())
        encryptionSubscription = handle
        return handle
    }

    override func userProfile(userId: String) async throws -> User {
        profileRequests.append(userId)
        return try profile.get()
    }

    override func startSync() async {
        syncStarts += 1
        record("startSync")
    }

    override func stopSync() async {
        syncStops += 1
        record("stopSync")
    }

    override func reconnect() async {
        reconnects += 1
    }

    override func presence(userId: String) async throws -> Presence {
        presenceRequests.append(userId)
        return try presenceResult.get()
    }

    override func presenceActivity(activity: PresenceActivity) async throws {
        record("presence \(activity)")
        while ownPresenceHangs { try await Task.sleep(nanoseconds: 10_000_000) }
        if let ownPresenceError { throw ownPresenceError }
    }

    override func setPresenceSharing(on: Bool) async throws {
        record("sharing \(on)")
        while ownPresenceHangs { try await Task.sleep(nanoseconds: 10_000_000) }
        if let ownPresenceError { throw ownPresenceError }
    }

    override func subscribePresence(userIds: [String], listener: PresenceListener) throws -> TaskHandle {
        let handle = FakeTaskHandle(noHandle: .init())
        presenceSubscriptions.append((userIds, listener, handle))
        return handle
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

    /// The last keeper made for each room.
    private(set) var keepers: [String: FakeDraftKeeper] = [:]

    override func draftKeeper(roomId: String) -> DraftKeeper {
        let keeper = FakeDraftKeeper(noHandle: .init())
        keeper.load = { [weak self] in self?.drafts[roomId] }
        keeper.keep = { [weak self] draft in
            let empty = draft.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && draft.replyTo == nil
            self?.drafts[roomId] = empty ? nil : draft
        }
        keepers[roomId] = keeper
        return keeper
    }

    // MARK: Rooms: settings, members, pins, joining, making

    let fakePinned = FakeTimeline(noHandle: .init())
    /// The rooms' settings, as `Room.subscribeSettings` gives them: to the
    /// listener as it subscribes, and again on every change (the changes
    /// made show at once, as in the library).
    var settings = Make.settings() {
        didSet { settingsListener?.onUpdate(settings: settings) }
    }
    var settingsListener: RoomSettingsListener?
    /// The rooms' disappearing mode, as `Room.subscribeDisappearingMode`
    /// gives it: to each listener as it subscribes, and again on every
    /// change (one made shows at once, as in the library).
    var disappearingMode: TimeInterval? {
        didSet { disappearingModeListeners.forEach { $0.onUpdate(afterRead: disappearingMode) } }
    }
    var disappearingModeListeners: [DisappearingModeListener] = []
    /// The limit of disappearing messages, as the library keeps it and
    /// `subscribeDisappearingLimit` gives it: to each listener as it
    /// subscribes, and again on every change.
    var limit: TimeInterval = 86_400 {
        didSet { limitListeners.forEach { $0.onUpdate(atMost: limit) } }
    }
    var limitListeners: [DisappearingLimitListener] = []
    var memberList: [Member] = []
    /// Changes asked for, in order, e.g. "notify mentions", "pin $e true".
    var changes: [String] = []
    /// Thrown by the calls that change something, if set.
    var changeError: Error?
    var children: Result<[DirectoryRoom], Error> = .success([])
    var childrenRequests = 0
    /// Each space's rooms as `spaceChildren` last gave them, as the
    /// library keeps them.
    var listed: [String: [DirectoryRoom]] = [:]
    /// What `sharedRooms` returns.
    var shared: [String] = []
    var directory: Result<DirectoryPage, Error> = .success(DirectoryPage(rooms: [], next: nil))
    /// Each `publicRooms` call's server, query and kind.
    var directoryRequests: [(server: String?, query: String, kind: DirectoryKind)] = []
    var browsedServers: [String] = []
    var previewResult: Result<DirectoryRoom, Error> = .failure(MtrxError.Other(reason: "Not found"))
    var offersOwnersResult = false
    var created: [NewRoom] = []

    fileprivate func change(_ change: String) throws {
        if let changeError { throw changeError }
        changes.append(change)
    }

    /// Room `roomId`: its calls change this session's state.
    override func room(roomId: String) throws -> Room {
        let room = FakeRoom(noHandle: .init())
        room.session = self
        room.roomId = roomId
        return room
    }

    override func pinnedTimeline(roomId: String) async throws -> Timeline { fakePinned }

    override func setNotesAtTop(on: Bool) async throws { try change("notes at top \(on)") }

    override func disappearingLimit() -> TimeInterval { limit }

    override func subscribeDisappearingLimit(listener: DisappearingLimitListener) -> TaskHandle {
        limitListeners.append(listener)
        listener.onUpdate(atMost: limit)
        return FakeTaskHandle(noHandle: .init())
    }

    /// Shown at once, as in the library; the saved one again if it fails.
    override func setDisappearingLimit(atMost: TimeInterval) async throws {
        let saved = limit
        limit = atMost
        do {
            try change("limit \(Int(atMost))")
        } catch {
            limit = saved
            throw error
        }
    }

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
        let rooms = try children.get()
        listed[spaceId] = rooms
        return rooms
    }

    override func listedSpaceChildren(spaceId: String) -> [DirectoryRoom]? {
        listed[spaceId]
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

    // MARK: Audio

    /// The file `audioFile` gives for each source, after reporting
    /// `audioProgress`; for others it waits until cancelled.
    var audioFiles: [String: Result<String, Error>] = [:]
    var audioProgress: [Float?] = []
    private var fetches: [String] = []
    private var cancelledFetches: [String] = []

    /// The sources fetched, in order (from the model's tasks, off the main
    /// actor: behind the lock).
    var audioFetches: [String] { locked { fetches } }
    /// The sources whose fetch was cancelled, in order.
    var audioCancelled: [String] { locked { cancelledFetches } }

    override func audioFile(media: Media, progress: AudioProgress) async throws -> String {
        let (file, steps) = locked { () -> (Result<String, Error>?, [Float?]) in
            fetches.append(media.source)
            return (audioFiles[media.source], audioProgress)
        }
        for step in steps {
            progress.onProgress(fraction: step)
        }
        if let file { return try file.get() }
        while !Task.isCancelled { try? await Task.sleep(nanoseconds: 10_000_000) }
        locked { cancelledFetches.append(media.source) }
        throw CancellationError()
    }
}

/// A player of a message's file (`AudioModel`): it plays from where it is
/// set, which the tests move; `end()` plays it to its end.
@MainActor
final class FakeAudioPlayer: AudioPlaying {
    let file: URL
    let duration: TimeInterval
    var currentTime: TimeInterval = 0
    var stopped: (@MainActor (AudioStop) -> Void)?
    private(set) var playing = false

    init(file: URL, duration: TimeInterval) {
        self.file = file
        self.duration = duration
    }

    func play() -> Bool {
        playing = true
        return true
    }

    func pause() {
        playing = false
    }

    func end() {
        playing = false
        currentTime = 0
        stopped?(.ended)
    }
}

/// A room of a fake session (`FakeSession.room`): gives its settings and
/// members, records changes in its `changes` and shows them in its
/// settings.
final class FakeRoom: Room, @unchecked Sendable {
    fileprivate var session: FakeSession!
    fileprivate var roomId = ""

    override func subscribeSettings(listener: RoomSettingsListener) -> TaskHandle {
        session.settingsListener = listener
        listener.onUpdate(settings: session.settings)
        return FakeTaskHandle(noHandle: .init())
    }

    override func members() async throws -> [Member] { session.memberList }

    override func setPinned(eventId: String, pin: Bool) async throws {
        try session.change("pin \(eventId) \(pin)")
    }

    override func setNotify(mode: NotifyMode) async throws {
        try session.change("notify \(mode)")
        session.settings.notify = mode
    }

    override func mute(duration: MuteFor) async throws {
        try session.change("mute \(duration)")
        session.settings.notify = .mute
    }

    override func setJoinRule(rule: JoinRule) async throws {
        try session.change("join rule \(rule)")
        session.settings.joinRule = rule
    }

    override func setName(name: String) async throws { try session.change("name \(name)") }

    override func setTopic(topic: String) async throws { try session.change("topic \(topic)") }

    override func setAddress(name: String?) async throws {
        try session.change("address \(name ?? "-")")
        session.settings.address = name.map { "#\($0):example.org" }
    }

    override func setListed(listed: Bool) async throws {
        try session.change("listed \(listed)")
        session.settings.listed = listed
    }

    override func setHistory(history: History) async throws {
        try session.change("history \(history)")
        session.settings.history = history
    }

    override func setPermissions(permissions: Permissions) async throws {
        try session.change("permissions \(permissions)")
        session.settings.permissions = permissions
    }

    override func subscribeDisappearingMode(listener: DisappearingModeListener) -> TaskHandle {
        session.disappearingModeListeners.append(listener)
        listener.onUpdate(afterRead: session.disappearingMode)
        return FakeTaskHandle(noHandle: .init())
    }

    override func setDisappearingMode(afterRead: TimeInterval?) async throws {
        let mode = afterRead.map { String(Int($0)) } ?? "off"
        try session.change("disappearing \(mode)")
        session.disappearingMode = afterRead
    }

    override func encrypt() async throws {
        try session.change("encrypt")
        session.settings.encrypted = true
    }

    /// On a space's room.
    override func setSuggested(roomId: String, suggested: Bool) async throws {
        try session.change("suggested \(self.roomId) \(suggested)")
        if let at = session.settings.spaces.firstIndex(where: { $0.spaceId == self.roomId }) {
            session.settings.spaces[at].suggested = suggested
        }
    }

    /// On a space's room.
    override func addChild(roomId: String) async throws {
        try session.change("add to \(self.roomId)")
        session.settings.canAddTo.removeAll { $0 == self.roomId }
        session.settings.spaces.append(SpaceLink(spaceId: self.roomId, suggested: false, canChange: true))
    }

    /// On a space's room.
    override func removeChild(roomId: String) async throws {
        try session.change("remove from \(self.roomId)")
        session.settings.spaces.removeAll { $0.spaceId == self.roomId }
        session.settings.canAddTo.append(self.roomId)
    }

    override func setLevel(userId: String, level: Int64) async throws {
        try session.change("level \(userId) \(level)")
    }

    override func remove(userId: String, ban: Bool, reason: String?) async throws {
        try session.change("\(ban ? "ban" : "remove") \(userId) \(reason ?? "-")")
    }

    override func unban(userId: String) async throws {
        try session.change("unban \(userId)")
        session.settings.banned.removeAll { $0.user.userId == userId }
    }

    override func invite(userId: String) async throws {
        if let inviteError = session.inviteError { throw inviteError }
        session.invites.append((roomId, userId))
    }

    override func leave() async throws { try session.change("leave \(roomId)") }
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
        state: SendState = .sent, eventId: String? = nil, editable: Bool = false, readBy: [User] = [],
        disappear: Disappearance? = nil
    ) -> TimelineEvent {
        TimelineEvent(
            eventId: eventId, sender: user(sender), timestamp: start.addingTimeInterval(minutes * 60), own: own,
            content: content, sendState: state, editable: editable, readBy: readBy, disappear: disappear)
    }

    /// How a message disappears, as the library words it for ours (with
    /// the time after read and the limit) and others'.
    static func disappearance(
        own: Bool, afterRead: TimeInterval = 300, atMost: TimeInterval = 86_400
    ) -> Disappearance {
        let after = Disappearing.choices.first { $0.afterRead == afterRead }?.label ?? ""
        let most = Disappearing.limits.first { $0.atMost == atMost }?.label ?? ""
        return Disappearance(
            afterRead: afterRead, atMost: atMost,
            words: own
                ? "Disappears \(after) after everyone has read it, at the latest \(most) after sending"
                : "Disappears after it was read")
    }

    /// A text message; its `text` one paragraph of `body` unless given.
    static func text(
        _ body: String, edited: Bool = false, reactions: [Reaction] = [], replyTo: ReplyQuote? = nil,
        rawBody: String = "", text: [TextBlock]? = nil, kind: MessageKind = .text
    ) -> EventContent {
        .message(
            message: Message(
                kind: kind, body: body, edited: edited, media: nil, reactions: reactions, replyTo: replyTo,
                rawBody: rawBody, text: text ?? [paragraph([plain(body)])]))
    }

    /// An audio message's file, `seconds` long as the library labels it: a
    /// voice message with its waveform, unless `name` is given.
    static func audio(_ source: String, name: String? = nil, seconds: TimeInterval? = 42) -> Media {
        Media(
            kind: .audio, filename: name ?? "Voice message.ogg", size: 24_000,
            mimetype: name == nil ? "audio/ogg" : "audio/mp4", width: nil, height: nil, source: source,
            duration: seconds, durationLabel: seconds.map { playbackTime(time: $0) }, voice: name == nil,
            waveform: name == nil ? waveform : [])
    }

    /// A voice message's 40 bars.
    static let waveform = (0..<40).map { Float(0.2 + 0.8 * abs(sin(Double($0) * 0.45))) }

    /// A message with an audio file.
    static func audioMessage(_ media: Media) -> EventContent {
        .message(
            message: Message(
                kind: .audio, body: media.filename, edited: false, media: media, reactions: [], replyTo: nil,
                rawBody: media.filename, text: []))
    }

    /// A paragraph of `runs`, outside quotes and lists.
    static func paragraph(_ runs: [TextRun]) -> TextBlock {
        TextBlock(kind: .paragraph(runs: runs), quotes: 0, lists: 0, marker: nil)
    }

    /// Text without style, optionally a link.
    static func plain(_ text: String, link: String? = nil) -> TextRun {
        .text(text: text, style: .plain, link: link)
    }

    /// With its dot and line, as the library sends them.
    static func presence(
        _ state: PresenceState, lastActive: Date? = nil, status: String? = nil
    ) -> Presence {
        let dot: PresenceDot? = switch state {
        case .online: .online
        case .unavailable: .idle
        case .offline: nil
        }
        var presence = Presence(
            state: state, lastActive: lastActive, statusMessage: status, dot: dot)
        presence.line = presenceLine(presence: presence, now: Date())
        return presence
    }

    /// `@name:example.org`, named `Name` unless `named` is false; shown as
    /// the library names them.
    static func user(_ name: String, named: Bool = true) -> User {
        User(
            userId: "@\(name):example.org", name: named ? name.capitalized : nil, avatarUrl: nil,
            displayName: named ? name.capitalized : name)
    }

    /// Someone known by their ID only, shown by its local part.
    static func user(id userId: String) -> User {
        User(userId: userId, name: nil, avatarUrl: nil, displayName: String(userId.dropFirst().prefix { $0 != ":" }))
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
        switch event.sendState {
        case .failed: actions += [.retry, .unsend]
        case .sending, .waiting: actions.append(.unsend)
        case .sent: break
        }
        return actions
    }

    /// A topic as the library parses it, with its first line and preview as
    /// the library makes them.
    static func topic(_ text: String = "", calls: [RoomLink] = [], links: [RoomLink] = []) -> Topic {
        let firstLine = text.split(separator: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty }
        let names = (calls + links).map(\.name)
        return Topic(
            text: text.trimmingCharacters(in: .whitespacesAndNewlines), calls: calls, links: links,
            firstLine: firstLine, preview: firstLine ?? (names.isEmpty ? nil : names.joined(separator: " · ")))
    }

    /// A topic's call or link, named after its title, else `name`.
    static func link(_ url: String, title: String? = nil, name: String? = nil, passcode: String? = nil) -> RoomLink {
        RoomLink(title: title, url: url, passcode: passcode, name: title ?? name ?? url)
    }

    static func room(
        _ id: String, kind: RoomKind = .room, membership: Membership = .joined, name: String? = nil, unread: UInt64 = 0,
        directUser: String? = nil, lastActivity: Date? = nil, avatarUrl: String? = nil, topic: Topic = .none,
        pinned: [String] = [], alias: String? = nil, members: UInt64 = 2,
        mentions: UInt64 = 0, rights: Rights = Make.rights()
    ) -> RoomSummary {
        RoomSummary(
            id: id, name: name ?? id, avatarUrl: avatarUrl, topic: topic, kind: kind, membership: membership,
            encrypted: false, unreadMessages: unread, unreadMentions: mentions, unreadNotifications: 0,
            lastActivity: lastActivity, directUser: directUser.map { user(id: $0) }, alias: alias,
            joinedMembers: members, joinRule: .invite, joinRuleFact: fact(.invite), pinned: pinned,
            rights: rights)
    }

    /// Our rights in a room: everything, unless not `may`, at a plain
    /// member's level 0.
    static func rights(_ may: Bool = true) -> Rights {
        Rights(
            send: may, redactOthers: may, pin: may, mentionRoom: may, rename: may, changeTopic: may,
            changePicture: may, invite: may, kick: may, ban: may, changeJoinRule: may, changeHistory: may,
            changeAddress: may, changeLevels: may, encrypt: may, addRooms: may, setCallService: may,
            level: 0, defaultLevel: 0)
    }

    /// A join rule's title in the settings' menu, as the library words it.
    static func title(_ rule: JoinRule) -> String {
        switch rule {
        case .invite: return "Invited people"
        case .knock: return "Ask to join"
        case .spaceMembers: return "Space members"
        case .public: return "Anyone"
        }
    }

    /// A join rule as a room's facts say it, as the library words it.
    static func fact(_ rule: JoinRule) -> String {
        switch rule {
        case .invite: return "Private: by invitation"
        case .knock: return "Private: people ask to join"
        case .spaceMembers: return "Private: space members can join"
        case .public: return "Public: anyone can join"
        }
    }

    /// A room's settings, all ours to change unless said otherwise; Space
    /// members offered as the library offers it.
    static func settings(
        canChange: Bool = true, joinRule: JoinRule = .invite, inSpace: Bool = false, encrypted: Bool = true,
        listed: Bool? = false, spaces: [SpaceLink] = [], canAddTo: [String] = [], banned: [Banned] = [], address: String? = nil
    ) -> RoomSettings {
        let choices = [JoinRule.invite, .knock, .spaceMembers, .public]
            .filter { $0 != .spaceMembers || inSpace || joinRule == .spaceMembers }
            .map { JoinRuleChoice(rule: $0, title: title($0), fact: fact($0)) }
        return RoomSettings(
            rights: rights(canChange), notify: .all, mutedUntil: nil, notifyTitle: "All messages",
            notifyExplanation: "Every message notifies you.", notifyChoices: notifyChoices, joinRule: joinRule,
            joinRuleFact: fact(joinRule), joinRuleChoices: choices, address: address, listed: listed,
            history: .joined, historyChoices: historyChoices, permissions: .everyone,
            permissionsChoices: permissionsChoices, encrypted: encrypted, version: "12", spaces: spaces,
            canAddTo: canAddTo, banned: banned)
    }

    /// The notification modes a room's settings offer, as the library words
    /// them.
    static let notifyChoices = [
        NotifyChoice(title: "All messages", action: .mode(mode: .all), current: true),
        NotifyChoice(title: "Mentions", action: .mode(mode: .mentions), current: false),
        NotifyChoice(title: "Mute for 1 hour", action: .mute(duration: .hour), current: false),
        NotifyChoice(title: "Mute for 8 hours", action: .mute(duration: .eightHours), current: false),
        NotifyChoice(title: "Mute until tomorrow, 08:00", action: .mute(duration: .tomorrowMorning), current: false),
        NotifyChoice(title: "Mute until I unmute", action: .mute(duration: .untilUnmuted), current: false),
    ]

    /// Who may read the history, as the library words it.
    static let historyChoices = [
        HistoryChoice(history: .joined, title: "Since joining", explanation: "Members, from when they joined."),
        HistoryChoice(history: .invited, title: "Since invited", explanation: "Members, from when they were invited."),
        HistoryChoice(
            history: .members, title: "Members", explanation: "Members, all of it, also from before they joined."),
        HistoryChoice(history: .anyone, title: "Anyone", explanation: "Anyone, without joining."),
    ]

    /// Who may edit, as the library words it.
    static let permissionsChoices = [
        PermissionsChoice(
            permissions: .everyone, title: "Everyone", explanation: "Everyone can invite, rename and change the topic."),
        PermissionsChoice(
            permissions: .moderators, title: "Moderators",
            explanation: "Invite, rename, change the topic and picture, and pin messages."),
    ]

    /// A member at `level` (nil: an owner); `upTo`: the highest level we
    /// may give them (nil: theirs is not ours to change). Their label and
    /// level choices as the library gives them, without what it asks.
    static func member(
        _ name: String, level: Int64?, upTo: Int64? = nil, remove: Bool = false, ban: Bool = false,
        membership: Membership = .joined, named: Bool = true
    ) -> Member {
        let labels: [Int64: String] = [100: "Admin", 50: "Mod", 0: ""]
        let label = level.map { labels[$0] ?? "Level \($0)" } ?? "Owner"
        let levels: [(String, Int64)] = [("Admin", 100), ("Moderator", 50), ("Member", 0)]
        let choices = upTo.map { upTo in
            levels.filter { $0.1 <= upTo || $0.1 == level }
                .map { LevelChoice(title: $0.0, level: $0.1, current: $0.1 == level, confirmation: nil) }
        }
        return Member(
            user: user(name, named: named),
            level: level,
            membership: membership, own: name == "me",
            manage: Manage(remove: remove, ban: ban, upTo: upTo, defaultLevel: 0, alone: false),
            label: label.isEmpty ? nil : label, choices: choices ?? [],
            manageable: upTo != nil || remove || ban)
    }

    static func directoryRoom(
        _ id: String, name: String? = nil, joinRule: JoinRule = .public, membership: Membership? = nil,
        space: Bool = false, members: UInt64 = 5, topic: Topic = .none, role: Role? = nil, children: UInt64 = 0
    ) -> DirectoryRoom {
        DirectoryRoom(
            id: id, alias: nil, name: name ?? id, topic: topic, avatarUrl: nil, members: members, isSpace: space,
            children: children, joinRule: joinRule, joinRuleFact: fact(joinRule), membership: membership, role: role,
            via: ["example.org"], suggested: false)
    }

    /// The library's lists of `rooms` (most recently active first), with
    /// what each space lists (`children`) and its members, by space ID; the
    /// rooms tagged by ID; the rooms joined or made here, not listed yet.
    static func lists(
        _ rooms: [RoomSummary], loaded: Bool = true, notesAtTop: Bool = true, children: [String: [String]] = [:],
        members: [String: [String]] = [:], drafts: [String] = [], favourites: [String] = [],
        lowPriority: [String] = [], categories: [TestCategory] = [], joining: [String] = []
    ) -> RoomLists {
        // The rooms are made up valid.
        try! RoomLists(
            rooms: rooms, loaded: loaded, ownUserId: "@me:example.org", notesAtTop: notesAtTop,
            spaceChildren: children, spaceMembers: members, drafts: drafts, favourites: favourites,
            lowPriority: lowPriority, categories: categories, joining: joining)
    }

    /// The rooms of the library's integration test, most recently active
    /// first: spaces Team and Other; Lab in Team, Deep in Team's nested
    /// space Sub; Loose in no space; a direct chat with Bob, a member of Team
    /// (unread); invites to Offer, in Team, and from Carol, in no space, to a
    /// direct chat and Party. Loose is unread.
    static let roomsBySpaceRooms = [
        room("!carol", kind: .direct, membership: .invited, name: "Carol", directUser: "@carol:example.org"),
        room("!party", membership: .invited, name: "Party"),
        room("!offer", membership: .invited, name: "Offer"),
        room("!bob", kind: .direct, name: "Bob", unread: 2, directUser: "@bob:example.org"),
        room("!loose", name: "Loose", unread: 1),
        room("!deep", name: "Deep"),
        room("!sub", kind: .space, name: "Sub"),
        room("!lab", name: "Lab"),
        room("!team", kind: .space, name: "Team"),
        room("!other", kind: .space, name: "Other"),
    ]

    /// What the spaces of `roomsBySpaceRooms` list.
    static let roomsBySpaceChildren = ["!team": ["!offer", "!sub", "!lab"], "!sub": ["!deep"]]

    /// Those rooms' lists, Bob's chat in Team; `extra` rooms last; rooms
    /// `joining`, not listed yet.
    static func roomsBySpace(loaded: Bool = true, extra: [RoomSummary] = [], joining: [String] = []) -> RoomLists {
        lists(
            roomsBySpaceRooms + extra, loaded: loaded, children: roomsBySpaceChildren,
            members: ["!team": ["@bob:example.org", "@me:example.org"]], joining: joining)
    }
}
