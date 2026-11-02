import Combine
import MtrxFFI
import UIKit

/// A signed-in account: its room lists, connection, pictures and presence,
/// sync, which runs while the app is in the foreground, and whether this
/// device can read encrypted messages.
@MainActor
final class SessionModel: ObservableObject {
    let session: Session
    let images: ImageCache
    let presence: PresenceModel
    let userId: String
    let deviceId: String
    let homeserver: String

    /// Most recently active first.
    @Published private(set) var rooms: [RoomSummary] = []
    /// Synced with the server since start; before, `rooms` may be a stale
    /// cache, or empty on the first sign-in.
    @Published private(set) var loaded = false
    /// The chats, and the rooms by space.
    @Published private(set) var lists = RoomLists()
    /// The Spaces tab's selected space, whose rooms it shows, remembered
    /// across launches (until sign-out).
    @Published private(set) var selectedSpace: SpaceEntry?
    /// The tab shown.
    @Published var tab = HomeTab.chats
    /// The chats opened over the Chats tab's list.
    @Published var chatsPath: [String] = []
    /// The rooms opened over the Rooms tab's list.
    @Published var roomsPath: [String] = []
    /// An invite to answer, while its sheet shows.
    @Published var invite: RoomRef?
    /// Rooms and spaces just made or joined here, not in the room list
    /// yet: shown as coming rather than gone.
    @Published private(set) var expecting: Set<String> = []
    /// The order of every space's rooms (not of the chats), remembered
    /// across launches.
    @Published private(set) var roomSort: RoomSort
    @Published private(set) var connection: ConnectionState = .online
    @Published var error: String?

    /// Whether this device is verified, the account's recovery, and
    /// whether message keys are arriving, as the library pushes it; nil
    /// until its first update.
    @Published private(set) var encryption: EncryptionState?
    /// Whether another of our devices is signed in (to verify with); nil
    /// until known.
    @Published private(set) var hasOtherDevice: Bool?
    /// The verification on screen, asked for here or by another device.
    @Published private(set) var verification: VerificationModel?
    @Published private(set) var requestingVerification = false
    /// Why asking another device to verify failed.
    @Published var verificationError: String?
    private(set) lazy var recovery = RecoveryModel(session: session)
    /// The spaces' rooms as their servers last listed them, for the
    /// overview to show at once.
    let spaceChildren = SpaceChildrenCache()
    /// How this phone shows each space's sections.
    let sectionLayouts: SectionLayouts
    /// The Rooms tab's groups folded on this phone, by `ListSection.key`
    /// (a space's ID, or Home's), remembered across launches (until
    /// sign-out).
    @Published private(set) var foldedGroups: Set<String>
    private var layoutChanges: AnyCancellable?

    private let subscriptions = Subscriptions()
    /// The room list as last given.
    private var listState: RoomListState?
    /// Notes at the top as just chosen here, until the room list has it.
    @Published private var notesAtTopSaving: Bool?
    /// The app is in the foreground: sync runs.
    private var foreground = false
    /// Sync has started once.
    private var started = false
    /// The last start or stop of sync; the next waits for it.
    private var syncChange: Task<Void, Never>?
    private var verificationRequest: Task<Void, Never>?
    private let defaults: UserDefaults
    private static let selectedSpaceKey = "selectedSpace"
    private static let roomSortKey = "roomSort"
    private static let foldedGroupsKey = "foldedRoomGroups"
    /// The rooms opened lately, kept by earlier versions.
    private static let recentKey = "recentRooms"

    init(session: Session, defaults: UserDefaults = .standard, idleTime: TimeInterval = PresenceModel.idleTime) {
        self.session = session
        self.defaults = defaults
        images = ImageCache(session: session)
        presence = PresenceModel(session: session, defaults: defaults, idleTime: idleTime)
        userId = session.userId()
        deviceId = session.deviceId()
        homeserver = session.homeserver()
        roomSort = defaults.string(forKey: Self.roomSortKey).flatMap(RoomSort.init(rawValue:)) ?? .listOrder
        // Opens on the space's rooms, until the room list says it is gone.
        if let saved = defaults.string(forKey: Self.selectedSpaceKey) {
            selectedSpace = SpaceEntry(saved: saved)
        }
        sectionLayouts = SectionLayouts(defaults: defaults)
        foldedGroups = Set(defaults.stringArray(forKey: Self.foldedGroupsKey) ?? [])
        defaults.removeObject(forKey: Self.recentKey)
        layoutChanges = sectionLayouts.objectWillChange.sink { [weak self] _ in
            MainActor.assumeIsolated { self?.objectWillChange.send() }
        }
    }

    /// Follow the connection, the room list, the encryption state and
    /// verification requests, for the session (sync starts right after).
    /// The encryption state is pushed now and on every change: its recovery
    /// is `unknown` until sync has run.
    func start() {
        let connection = Feed<ConnectionState>()
        subscriptions.add(session.subscribeConnection(listener: connection), connection) { [weak self] in
            self?.connection = $0
        }
        let encryption = Feed<EncryptionState>()
        subscriptions.add(session.subscribeEncryptionState(listener: encryption), encryption) { [weak self] in
            self?.encryption = $0
        }
        let roomList = Feed<RoomListState>()
        let session = self.session
        subscriptions.add(Task { [weak self] in
            do {
                let handle = try await session.subscribeRoomList(listener: roomList)
                self?.subscriptions.add(handle, roomList) { [weak self] state in
                    self?.apply(state)
                }
            } catch {
                self?.error = errorMessage(error)
            }
        })
        // Requests arrive while sync runs.
        let requests = Feed<Verification>(keepingAll: true)
        subscriptions.add(session.subscribeVerificationRequests(listener: requests), requests) { [weak self] in
            self?.received($0)
        }
    }

    private func apply(_ state: RoomListState) {
        listState = state
        rooms = state.rooms
        loaded = state.loaded
        // A choice made here shows until the list has it.
        if notesAtTopSaving == state.notesAtTop {
            notesAtTopSaving = nil
        }
        if let notesAtTop = notesAtTopSaving {
            var state = state
            state.notesAtTop = notesAtTop
            lists = RoomLists(state)
        } else {
            lists = RoomLists(state)
        }
        expecting = expecting.filter { lists.byId[$0]?.invited ?? true }
        guard state.loaded else { return }
        // The first states may be a cache: rooms may come with sync.
        presence.follow(lists.chats)
        if let selectedSpace, lists.space(selectedSpace) == nil, !isExpected(selectedSpace) {
            selectSpace(nil)
        }
    }

    /// The Notes to self stays at the top of Chats and of every space (for
    /// all the user's devices).
    var notesAtTop: Bool {
        notesAtTopSaving ?? listState?.notesAtTop ?? true
    }

    /// Keep the Notes to self at the top, or not: shown so at once, until
    /// the room list has it (or it failed, with why).
    @discardableResult
    func setNotesAtTop(_ on: Bool) -> Task<Void, Never> {
        notesAtTopSaving = on
        if let listState { apply(listState) }
        let session = self.session
        return Task {
            do {
                try await session.setNotesAtTop(on: on)
            } catch {
                self.error = errorMessage(error)
                notesAtTopSaving = nil
                if let listState { apply(listState) }
            }
        }
    }

    private func isExpected(_ entry: SpaceEntry) -> Bool {
        if case .space(let id) = entry { return expecting.contains(id) }
        return false
    }

    func room(_ id: String) -> RoomSummary? {
        rooms.first { $0.id == id }
    }

    /// Sync while the app is in the foreground, and set our presence;
    /// when it goes to the background, go offline, then stop and save the
    /// sync position, before iOS suspends it. Not sharing our presence,
    /// offline before sync first starts.
    func setForeground(_ on: Bool) {
        guard on != foreground else { return }
        foreground = on
        let previous = syncChange
        let session = self.session
        if on {
            let offline = presence.enterForeground(first: !started)
            started = true
            syncChange = Task {
                await previous?.value
                await offline?.value
                await session.startSync()
            }
        } else {
            let background = BackgroundTask(name: "Stop sync")
            let offline = presence.enterBackground()
            syncChange = Task {
                await previous?.value
                await offline?.value
                await session.stopSync()
                background.end()
            }
        }
    }

    // MARK: Encryption

    /// The banner shows: encrypted messages cannot all be read until the
    /// device is verified or recovery unlocked. Not before the first sync
    /// (recovery unknown).
    var needsEncryptionSetup: Bool {
        guard let encryption, encryption.recovery != .unknown else { return false }
        return !encryption.deviceVerified || encryption.recovery == .incomplete
    }

    /// Learn whether another device can verify this one.
    func loadDevices() async {
        guard let devices = try? await session.devices() else { return }
        hasOtherDevice = devices.contains { !$0.current }
    }

    /// Ask our other devices to verify this one, and show the verification.
    func verifyWithAnotherDevice() {
        guard !requestingVerification, verification == nil else { return }
        requestingVerification = true
        verificationError = nil
        let session = self.session
        verificationRequest = Task {
            do {
                let verification = try await session.requestSelfVerification()
                show(verification, incoming: false)
            } catch {
                verificationError = errorMessage(error)
            }
            requestingVerification = false
            verificationRequest = nil
        }
    }

    /// The verification screen closes: the verification is dropped.
    func closeVerification() {
        verification = nil
    }

    /// Another device asks to verify. Only our own devices are verified
    /// here: requests from other people are cancelled. One that comes while
    /// an earlier request is unanswered replaces it; while a verification
    /// runs, it is cancelled.
    func received(_ request: Verification) {
        let busy = verification.map { !$0.ended && !$0.asking } ?? false
        guard request.otherUser() == userId, !busy else {
            Task { try? await request.cancel() }
            return
        }
        verification?.cancel()
        show(request, incoming: true)
    }

    private func show(_ verification: Verification, incoming: Bool) {
        self.verification = VerificationModel(verification: verification, incoming: incoming)
    }

    // MARK: Rooms

    /// Show `entry`'s rooms (and remember it), or the spaces with none
    /// selected.
    func selectSpace(_ entry: SpaceEntry?) {
        selectedSpace = entry
        if let entry {
            defaults.set(entry.saved, forKey: Self.selectedSpaceKey)
        } else {
            defaults.removeObject(forKey: Self.selectedSpaceKey)
        }
    }

    /// The Spaces tab's pages over its list: the selected space's rooms.
    var spacesPath: [SpaceEntry] {
        get { selectedSpace.map { [$0] } ?? [] }
        set { selectSpace(newValue.last) }
    }

    /// A tab was tapped; tapped again, it goes back to its list.
    func select(_ tab: HomeTab) {
        guard tab == self.tab else {
            self.tab = tab
            return
        }
        switch tab {
        case .chats: chatsPath = []
        case .spaces: selectSpace(nil)
        case .rooms: roomsPath = []
        case .settings: break
        }
    }

    /// Open a room (or a direct chat) where it belongs: a room in the Rooms
    /// tab, over its list; a chat in the Chats tab; a space's rooms in the
    /// Spaces tab. `kind` for one not in the room list yet.
    func open(_ id: String, kind: RoomKind? = nil) {
        switch kind ?? lists.byId[id]?.kind {
        case .direct:
            chatsPath = [id]
            tab = .chats
        case .space:
            selectSpace(.space(id))
            tab = .spaces
        case .room, nil:
            roomsPath = [id]
            tab = .rooms
        }
    }

    /// We left a room, or declined an invite: it goes from where it shows.
    func left(_ id: String) {
        roomsPath.removeAll { $0 == id }
        chatsPath.removeAll { $0 == id }
        if selectedSpace == .space(id) {
            selectSpace(nil)
        }
        expecting.remove(id)
    }

    /// A room or space made or joined here: shown as coming until the room
    /// list has it.
    func expect(_ id: String) {
        expecting.insert(id)
    }

    /// Accept an invite: join, then open what we joined.
    func accept(_ room: RoomSummary) async throws {
        _ = try await session.join(room: room.id, via: [])
        expect(room.id)
        open(room.id)
    }

    /// Decline an invite.
    func decline(_ room: RoomSummary) async throws {
        try await session.leave(roomId: room.id)
        left(room.id)
    }

    /// Our server's name, as in our user ID: "example.org".
    var serverName: String {
        userId.split(separator: ":", maxSplits: 1).last.map(String.init) ?? homeserver
    }

    /// The Chats tab's badge: unread messages in the direct chats.
    var chatsBadge: Int { lists.chatsUnread }

    /// The Rooms tab's badge: unread messages in the rooms.
    var roomsBadge: Int { lists.roomsUnread }

    /// The Spaces tab's badge: unread messages in the rooms in spaces.
    var spacesBadge: Int { lists.spacesUnread }

    /// The number on a room's or chat's back button: unread messages in
    /// the other rooms and direct chats.
    func backBadge(besides id: String) -> Int {
        lists.unread(besides: id)
    }

    /// The Rooms tab's groups, folded as on this phone.
    var roomGroups: [ListSection] {
        lists.roomGroups().map { group in
            var group = group
            group.folded = foldedGroups.contains(group.key)
            return group
        }
    }

    /// Fold or unfold the Rooms tab's group with this key, and remember it.
    func toggleGroupFold(_ key: String) {
        if foldedGroups.contains(key) {
            foldedGroups.remove(key)
        } else {
            foldedGroups.insert(key)
        }
        defaults.set(foldedGroups.sorted(), forKey: Self.foldedGroupsKey)
    }

    /// Order the spaces' rooms by `sort`, from now on.
    func setRoomSort(_ sort: RoomSort) {
        roomSort = sort
        defaults.set(sort.rawValue, forKey: Self.roomSortKey)
    }

    func timeline(for room: RoomSummary) -> TimelineModel {
        TimelineModel(session: session, room: room)
    }

    func info(for room: RoomSummary) -> RoomInfoModel {
        RoomInfoModel(session: session, roomId: room.id, userId: userId)
    }

    /// Sign out on the server and delete the account's data here.
    func logout() async throws {
        subscriptions.cancelAll()
        verification = nil
        await presence.signOut()?.value
        try await session.logout()
        forgetPlaces()
    }

    /// Delete the data of a session the server no longer accepts.
    func forget() async throws {
        subscriptions.cancelAll()
        verification = nil
        // The server no longer accepts the session: nothing to set.
        presence.forget()
        try await session.forget()
        forgetPlaces()
    }

    /// The selected space, the spaces' sections and the Rooms tab's folded
    /// groups are the account's.
    private func forgetPlaces() {
        defaults.removeObject(forKey: Self.selectedSpaceKey)
        defaults.removeObject(forKey: Self.foldedGroupsKey)
        foldedGroups = []
        sectionLayouts.forget()
    }
}

/// The tabs at the bottom.
enum HomeTab: Hashable {
    /// Direct chats.
    case chats
    /// The rooms, by space.
    case rooms
    /// The spaces and their rooms.
    case spaces
    case settings
}

/// A room by ID, to present a sheet for.
struct RoomRef: Identifiable, Hashable {
    let id: String
}

/// The spaces' rooms as their servers last listed them, by space ID.
@MainActor
final class SpaceChildrenCache {
    private var rooms: [String: [DirectoryRoom]] = [:]

    subscript(spaceId: String) -> [DirectoryRoom]? {
        get { rooms[spaceId] }
        set { rooms[spaceId] = newValue }
    }
}

/// Time iOS grants to finish work after the app went to the background.
@MainActor
final class BackgroundTask {
    private var id = UIBackgroundTaskIdentifier.invalid

    init(name: String) {
        id = UIApplication.shared.beginBackgroundTask(withName: name) { [weak self] in
            // Time is up: called on the main thread.
            MainActor.assumeIsolated { self?.end() }
        }
    }

    func end() {
        guard id != .invalid else { return }
        UIApplication.shared.endBackgroundTask(id)
        id = .invalid
    }
}
