import MtrxFFI
import UIKit

/// A signed-in account: its connection, pictures, presence and downloads,
/// sync, which runs while the app is in the foreground, and signing out.
/// Its room lists, the home screen's navigation and its encryption are
/// models of their own (`rooms`, `home`, `encryption`), which views
/// observe apart: a room-list update draws again only the views that show
/// rooms.
@MainActor
final class SessionModel: ObservableObject {
    let session: Session
    let images: ImageCache
    let presence: PresenceModel
    let downloads: DownloadsModel
    let rooms: RoomListModel
    let home: HomeNavigation
    let encryption: EncryptionModel
    let userId: String
    let deviceId: String
    let homeserver: String

    @Published private(set) var connection: ConnectionState = .online
    @Published var error: String?

    private let subscriptions = Subscriptions()
    /// The app is in the foreground: sync runs.
    private var foreground = false
    /// Sync has started once.
    private var started = false
    /// The last start or stop of sync; the next waits for it.
    private var syncChange: Task<Void, Never>?

    init(session: Session, defaults: UserDefaults = .standard) {
        self.session = session
        images = ImageCache(session: session)
        presence = PresenceModel(session: session, defaults: defaults)
        downloads = DownloadsModel(session: session)
        userId = session.userId()
        deviceId = session.deviceId()
        homeserver = session.homeserver()
        rooms = RoomListModel(session: session)
        home = HomeNavigation(rooms: rooms, defaults: defaults)
        encryption = EncryptionModel(session: session, userId: userId)
        rooms.failed = { [weak self] in self?.error = $0 }
    }

    /// Follow the connection, the room list, the encryption state, the
    /// downloads and verification requests, for the session (sync starts
    /// right after). The encryption state is pushed now and on every change:
    /// its recovery is `unknown` until sync has run.
    func start() {
        let connection = Feed<ConnectionState>()
        subscriptions.add(session.subscribeConnection(listener: connection), connection) { [weak self] in
            self?.connection = $0
        }
        downloads.start()
        let state = Feed<EncryptionState>()
        subscriptions.add(session.subscribeEncryptionState(listener: state), state) { [weak self] in
            self?.encryption.apply($0)
        }
        let roomLists = Feed<RoomLists>()
        let session = self.session
        subscriptions.add(Task { [weak self] in
            do {
                let handle = try await session.subscribeRoomLists(listener: roomLists)
                self?.subscriptions.add(handle, roomLists) { [weak self] lists in
                    self?.apply(lists)
                }
            } catch {
                self?.error = errorMessage(error)
            }
        })
        // Requests arrive while sync runs.
        let requests = Feed<Verification>(keepingAll: true)
        subscriptions.add(session.subscribeVerificationRequests(listener: requests), requests) { [weak self] in
            self?.encryption.received($0)
        }
    }

    private func apply(_ lists: RoomLists) {
        rooms.apply(lists)
        // The first lists may be from a cache: rooms may come with sync.
        if lists.loaded() {
            presence.follow(lists.chats())
        }
    }

    /// Sync while the app is in the foreground, and set our presence;
    /// when it goes to the background, go offline, then stop and save the
    /// sync position, before iOS suspends it. Not sharing our presence,
    /// offline before sync starts.
    func setForeground(_ on: Bool) {
        guard on != foreground else { return }
        foreground = on
        let previous = syncChange
        let session = self.session
        if on {
            let ours = presence.enterForeground(first: !started)
            started = true
            syncChange = Task {
                await previous?.value
                await ours?.value
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

    /// Our server's name, as in our user ID: "example.org".
    var serverName: String {
        userId.split(separator: ":", maxSplits: 1).last.map(String.init) ?? homeserver
    }

    /// Accept an invite: join, then open what we joined.
    func accept(_ room: RoomSummary) async throws {
        _ = try await session.join(room: room.id, via: [])
        home.open(room.id)
    }

    /// Decline an invite.
    func decline(_ room: RoomSummary) async throws {
        try await session.room(roomId: room.id).leave()
        home.left(room.id)
    }

    /// Sign out on the server and delete the account's data here.
    func logout() async throws {
        subscriptions.cancelAll()
        encryption.closeVerification()
        await presence.signOut()?.value
        try await session.logout()
        home.forget()
    }

    /// Delete the data of a session the server no longer accepts.
    func forget() async throws {
        subscriptions.cancelAll()
        encryption.closeVerification()
        // The server no longer accepts the session: nothing to set.
        presence.forget()
        try await session.forget()
        home.forget()
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
