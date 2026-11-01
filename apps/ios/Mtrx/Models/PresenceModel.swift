import Foundation
import MtrxFFI
import os

/// Presence: the direct chats' people (online, idle or away), read while
/// the app is in the foreground, and ours, which the library sets from the
/// app's lifecycle and input while "Share my presence" is on. When the
/// server does not share presence, all of it is hidden for the rest of the
/// session.
@MainActor
final class PresenceModel: ObservableObject {
    private static let sharingKey = "sharePresence"
    private static let log = Logger(subsystem: "org.mtrx.Mtrx", category: "presence")

    /// The last presences read, by user ID; a user missing shows nothing.
    @Published private(set) var presences: [String: Presence] = [:]
    /// The server shares presence (until it says it does not).
    @Published private(set) var available = true
    /// "Share my presence": a device preference, on by default, kept at
    /// sign-out.
    @Published private(set) var sharing: Bool

    private let session: Session
    private let userId: String
    private let defaults: UserDefaults
    private var foreground = false
    /// The users to read: the direct chats' people (nil before the first
    /// loaded room list).
    private var users: Set<String>?
    private var subscription: Subscriptions?
    /// The last call telling the library what the app does.
    private var lastCall: Task<Void, Never>?

    init(session: Session, defaults: UserDefaults = .standard) {
        self.session = session
        self.defaults = defaults
        userId = session.userId()
        sharing = defaults.object(forKey: Self.sharingKey) as? Bool ?? true
    }

    // MARK: Others

    /// The presence to show for `userId`, if any.
    func presence(of userId: String) -> Presence? {
        presences[userId]
    }

    /// The presence to show on a room: a direct chat's person (not ours).
    func presence(of room: RoomSummary) -> Presence? {
        Self.person(of: room, ours: userId).flatMap(presence(of:))
    }

    /// The person of a direct chat whose presence is read: not the Notes
    /// to self.
    nonisolated static func person(of room: RoomSummary, ours: String) -> String? {
        guard room.kind == .direct, let user = room.directUserId, user != ours else { return nil }
        return user
    }

    /// Read the presence of the people of `rooms`' direct chats, from a
    /// loaded room list; subscribed anew only when they change.
    func follow(_ rooms: [RoomSummary]) {
        let users = Set(rooms.compactMap { Self.person(of: $0, ours: userId) })
        guard available, users != self.users else { return }
        self.users = users
        if foreground {
            subscribe()
        }
    }

    /// Fetch `userId`'s presence now (their info sheet opens); a failure
    /// keeps what is known.
    func refresh(_ userId: String) async {
        guard available else { return }
        do {
            let presence = try await session.presence(userId: userId)
            guard available else { return }
            presences[userId] = presence
        } catch {
            failed(error, "Reading \(userId)'s presence")
        }
    }

    /// Subscribe for `users`, then end the subscription before.
    private func subscribe() {
        let previous = subscription
        subscription = nil
        defer { previous?.cancelAll() }
        guard let users, !users.isEmpty else {
            presences = [:]
            return
        }
        let feed = Feed<PresenceUpdate>()
        do {
            let handle = try session.subscribePresence(userIds: users.sorted(), listener: feed)
            let subscription = Subscriptions()
            subscription.add(handle, feed) { [weak self] update in
                switch update {
                case .presences(let presences): self?.presences = presences
                case .unavailable: self?.hide()
                }
            }
            self.subscription = subscription
        } catch {
            failed(error, "Subscribing to presence")
        }
    }

    /// The server does not share presence: hide it for the session.
    private func hide() {
        Self.log.info("The server does not share presence")
        available = false
        presences = [:]
        users = nil
        stopReading()
    }

    private func stopReading() {
        subscription?.cancelAll()
        subscription = nil
    }

    private func failed(_ error: Error, _ doing: String) {
        if case MtrxError.PresenceUnavailable = error {
            hide()
        } else if !(error is CancellationError) {
            Self.log.error("\(doing, privacy: .public) failed: \(String(describing: error), privacy: .public)")
        }
    }

    // MARK: Ours and the lifecycle

    /// The app comes to the foreground (also at start and sign-in): read
    /// others' presence again, and tell the library, which sets ours (the
    /// first time after "Share my presence"). Returns that call, to wait for
    /// before sync starts (so that sync says the same).
    func enterForeground(first: Bool) -> Task<Void, Never>? {
        foreground = true
        guard available else { return nil }
        if users != nil {
            subscribe()
        }
        let sharing = first ? self.sharing : nil
        return tell { session in
            if let sharing {
                try await session.setPresenceSharing(on: sharing)
            }
            try await session.presenceActivity(activity: .foreground)
        }
    }

    /// The app goes to the background: stop reading others' presence, and
    /// go offline. Returns that call, to wait for before sync stops (within
    /// the background task).
    func enterBackground() -> Task<Void, Never>? {
        foreground = false
        stopReading()
        guard available else { return nil }
        return tell { try await $0.presenceActivity(activity: .background) }
    }

    /// Signing out: no more reading, and offline (to wait for before
    /// `logout`).
    func signOut() -> Task<Void, Never>? {
        foreground = false
        users = nil
        stopReading()
        guard available else { return nil }
        return tell { try await $0.presenceActivity(activity: .signOut) }
    }

    /// The session is deleted without signing out: no more reading or
    /// setting.
    func forget() {
        foreground = false
        users = nil
        stopReading()
    }

    /// Share our presence or stop: offline at once, or online if in the
    /// foreground.
    func setSharing(_ on: Bool) {
        guard on != sharing else { return }
        sharing = on
        defaults.set(on, forKey: Self.sharingKey)
        guard available else { return }
        tell { try await $0.setPresenceSharing(on: on) }
    }

    /// A touch or key press: online again if idle.
    func input() {
        guard foreground, available else { return }
        tell { try await $0.presenceActivity(activity: .input) }
    }

    /// Tell the library with `call`, after the call before (the library
    /// counts each as of when it gets it), which sets ours; failures are
    /// logged (or hide presence).
    @discardableResult
    private func tell(_ call: @escaping @Sendable (Session) async throws -> Void) -> Task<Void, Never> {
        let previous = lastCall
        let session = self.session
        let task = Task { [weak self] in
            await previous?.value
            do {
                try await call(session)
            } catch {
                self?.failed(error, "Setting our presence")
            }
        }
        lastCall = task
        return task
    }
}

/// What a presence subscription reports.
enum PresenceUpdate: Sendable {
    /// All presences known, by user ID.
    case presences([String: Presence])
    /// The server does not share presence; the last report.
    case unavailable
}

extension Feed: PresenceListener where Value == PresenceUpdate {
    func onUpdate(presences: [String: Presence]) { send(.presences(presences)) }
    func onUnavailable() { send(.unavailable) }
}
