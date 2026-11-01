import Foundation
import MtrxFFI
import os

/// Presence: the direct chats' people (online, idle or away), read while
/// the app is in the foreground, and ours, set from the app's lifecycle
/// while "Share my presence" is on. When the server does not share
/// presence, all of it is hidden for the rest of the session.
@MainActor
final class PresenceModel: ObservableObject {
    /// Without input for this long in the foreground, we are idle.
    nonisolated static let idleTime: TimeInterval = 5 * 60
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
    private let idleTime: TimeInterval
    private var foreground = false
    /// The users to read: the direct chats' people (nil before the first
    /// loaded room list).
    private var users: Set<String>?
    private var subscription: Subscriptions?
    /// Ours, as last set here.
    private(set) var own: PresenceState?
    /// The last `setPresence` call: each waits for the one before.
    private var ownCall: Task<Void, Never>?
    private var lastInput = Date()
    /// Waits for 5 minutes without input, while online.
    private var idleTimer: Task<Void, Never>?

    init(session: Session, defaults: UserDefaults = .standard, idleTime: TimeInterval = PresenceModel.idleTime) {
        self.session = session
        self.defaults = defaults
        self.idleTime = idleTime
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
        stopIdleTimer()
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
    /// others' presence again and, sharing ours, be online. Not sharing,
    /// the first call returns `setPresence(offline)`, to wait for before
    /// sync starts (so that sync does not report us online either).
    func enterForeground(first: Bool) -> Task<Void, Never>? {
        foreground = true
        guard available else { return nil }
        if users != nil {
            subscribe()
        }
        if sharing {
            beActive()
            return nil
        }
        return first ? setOwn(.offline) : nil
    }

    /// The app goes to the background: stop reading others' presence and,
    /// sharing ours, go offline. Returns that call, to wait for before sync
    /// stops (within the background task).
    func enterBackground() -> Task<Void, Never>? {
        foreground = false
        stopReading()
        stopIdleTimer()
        guard available, sharing else { return nil }
        return setOwn(.offline)
    }

    /// Signing out: no more reading, and offline (to wait for before
    /// `logout`) if shared.
    func signOut() -> Task<Void, Never>? {
        let offline = enterBackground()
        users = nil
        return offline
    }

    /// The session is deleted without signing out: no more reading or
    /// setting.
    func forget() {
        foreground = false
        users = nil
        stopReading()
        stopIdleTimer()
    }

    /// Share our presence or stop: offline at once, or online if in the
    /// foreground.
    func setSharing(_ on: Bool) {
        guard on != sharing else { return }
        sharing = on
        defaults.set(on, forKey: Self.sharingKey)
        guard available else { return }
        if on {
            if foreground {
                beActive()
            }
        } else {
            stopIdleTimer()
            setOwn(.offline)
        }
    }

    /// A touch or key press: online again if idle.
    func input() {
        lastInput = Date()
        guard foreground, sharing, available, idleTimer == nil else { return }
        beActive()
    }

    /// Online, until idle.
    private func beActive() {
        lastInput = Date()
        setOwn(.online)
        startIdleTimer()
    }

    private func stopIdleTimer() {
        idleTimer?.cancel()
        idleTimer = nil
    }

    /// Unavailable once `idleTime` has passed since the last input.
    private func startIdleTimer() {
        stopIdleTimer()
        let idleTime = self.idleTime
        idleTimer = Task { [weak self] in
            while let last = self?.lastInput {
                let left = last.addingTimeInterval(idleTime).timeIntervalSinceNow
                if left <= 0 { break }
                try? await Task.sleep(nanoseconds: UInt64(left * 1_000_000_000))
                if Task.isCancelled { return }
            }
            guard let self else { return }
            self.idleTimer = nil
            self.setOwn(.unavailable)
        }
    }

    /// Set ours, after the previous call.
    @discardableResult
    private func setOwn(_ state: PresenceState) -> Task<Void, Never> {
        own = state
        let previous = ownCall
        let session = self.session
        let call = Task { [weak self] in
            await previous?.value
            do {
                try await session.setPresence(state: state, statusMessage: nil)
            } catch {
                self?.failed(error, "Setting our presence")
            }
        }
        ownCall = call
        return call
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

/// How a presence shows: the dot on the avatar and the line under the
/// name.
enum PresenceText {
    enum Dot: Equatable {
        case online
        case idle

        var label: String {
            switch self {
            case .online: return "Online"
            case .idle: return "Idle"
            }
        }
    }

    /// Green online, amber unavailable; none offline or unknown.
    static func dot(_ presence: Presence?) -> Dot? {
        switch presence?.state {
        case .online: return .online
        case .unavailable: return .idle
        case .offline, nil: return nil
        }
    }

    /// The status message, else "Active now" or "Idle", else when they were
    /// last active; nil for nothing to say.
    static func line(_ presence: Presence?, now: Date = .now, locale: Locale = .current) -> String? {
        guard let presence else { return nil }
        if let status = presence.statusMessage, !status.isEmpty {
            return status
        }
        switch presence.state {
        case .online: return "Active now"
        case .unavailable: return "Idle"
        case .offline:
            guard let last = presence.lastActive else { return nil }
            if now.timeIntervalSince(last) < 60 {
                return "Last active just now"
            }
            let formatter = RelativeDateTimeFormatter()
            formatter.unitsStyle = .abbreviated
            formatter.locale = locale
            return "Last active \(formatter.localizedString(for: last, relativeTo: now))"
        }
    }
}
