import Foundation
import MtrxFFI

/// A room's or space's settings and members, for its header, topic panel,
/// info screen and overview, changed where we may. The settings follow the
/// library: a change made here shows at once, as do those made elsewhere;
/// a failed one shows why. The members are read, and read again after we
/// change them.
@MainActor
final class RoomInfoModel: ObservableObject {
    let roomId: String
    /// Ours.
    let userId: String
    /// Nil until read.
    @Published private(set) var settings: RoomSettings?
    /// Joined and invited members, admins first; nil until read.
    @Published private(set) var members: [Member]?
    /// Why the settings or members could not be read.
    @Published private(set) var loadError: String?
    /// Why the last change failed.
    @Published var error: String?

    /// The room, or why it is not known.
    private let room: Result<Room, Error>
    /// For the spaces the room is in.
    private let session: Session
    /// The settings' subscription, once started, and the changes.
    private let work = Work()
    private var following = false

    init(session: Session, roomId: String, userId: String) {
        self.session = session
        self.roomId = roomId
        self.userId = userId
        room = Result { try session.room(roomId: roomId) }
    }

    /// Follow the settings (from the first call on) and read the members,
    /// again if read before.
    func load() async {
        follow()
        await loadMembers()
    }

    private func follow() {
        guard !following else { return }
        following = true
        do {
            let room = try self.room.get()
            let feed = Feed<RoomSettings>()
            work.follow(room.subscribeSettings(listener: feed), feed) { [weak self] settings in
                self?.settings = settings
            }
        } catch {
            loadError = errorMessage(error)
        }
    }

    func loadMembers() async {
        do {
            members = try await room.get().members()
        } catch {
            loadError = errorMessage(error)
        }
    }

    // MARK: Settings

    /// Save `topic` (as `composeTopic` makes it): the room list shows it at
    /// once. `then` says whether it was saved.
    @discardableResult
    func setTopic(_ topic: String, then: @escaping @MainActor (Bool) -> Void = { _ in }) -> Task<Void, Never> {
        change({ try await $0.setTopic(topic: topic) }, then: then)
    }

    @discardableResult
    func rename(_ name: String) -> Task<Void, Never> {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return change { try await $0.setName(name: name) }
    }

    @discardableResult
    func setNotify(_ mode: NotifyMode) -> Task<Void, Never> {
        change { try await $0.setNotify(mode: mode) }
    }

    @discardableResult
    func setJoinRule(_ rule: JoinRule) -> Task<Void, Never> {
        change { try await $0.setJoinRule(rule: rule) }
    }

    @discardableResult
    func setListed(_ listed: Bool) -> Task<Void, Never> {
        change { try await $0.setListed(listed: listed) }
    }

    @discardableResult
    func setHistory(_ history: History) -> Task<Void, Never> {
        change { try await $0.setHistory(history: history) }
    }

    @discardableResult
    func setPermissions(_ permissions: Permissions) -> Task<Void, Never> {
        change { try await $0.setPermissions(permissions: permissions) }
    }

    /// Encrypt from now on: not to be undone.
    @discardableResult
    func encrypt() -> Task<Void, Never> {
        change { try await $0.encrypt() }
    }

    /// The address `#name:ourserver`, or none (`name` empty).
    @discardableResult
    func setAddress(_ name: String) -> Task<Void, Never> {
        let name = name.trimmingCharacters(in: .whitespaces)
        return change { try await $0.setAddress(name: name.isEmpty ? nil : name) }
    }

    @discardableResult
    func setSuggested(in space: String, _ suggested: Bool) -> Task<Void, Never> {
        let (session, roomId) = (self.session, self.roomId)
        return change { _ in try await session.room(roomId: space).setSuggested(roomId: roomId, suggested: suggested) }
    }

    @discardableResult
    func addToSpace(_ space: String) -> Task<Void, Never> {
        let (session, roomId) = (self.session, self.roomId)
        return change { _ in try await session.room(roomId: space).addChild(roomId: roomId) }
    }

    @discardableResult
    func removeFromSpace(_ space: String) -> Task<Void, Never> {
        let (session, roomId) = (self.session, self.roomId)
        return change { _ in try await session.room(roomId: space).removeChild(roomId: roomId) }
    }

    @discardableResult
    func unban(_ user: String) -> Task<Void, Never> {
        change { try await $0.unban(userId: user) }
    }

    // MARK: Members

    /// Give someone (or ourselves) a power level.
    @discardableResult
    func setLevel(_ level: Int64, of user: String) -> Task<Void, Never> {
        changeMembers { try await $0.setLevel(userId: user, level: level) }
    }

    /// Remove someone, or ban them; they see `reason`.
    @discardableResult
    func remove(_ user: String, reason: String, ban: Bool) -> Task<Void, Never> {
        let reason = reason.trimmingCharacters(in: .whitespacesAndNewlines)
        return changeMembers {
            try await $0.remove(userId: user, ban: ban, reason: reason.isEmpty ? nil : reason)
        }
    }

    /// Invite someone by user ID. `then` says whether invited.
    @discardableResult
    func invite(_ user: String, then: @escaping @MainActor (Bool) -> Void = { _ in }) -> Task<Void, Never> {
        let user = user.trimmingCharacters(in: .whitespaces)
        return changeMembers({ try await $0.invite(userId: user) }, then: then)
    }

    /// Leave the room or space. `then` says whether left.
    @discardableResult
    func leave(then: @escaping @MainActor (Bool) -> Void = { _ in }) -> Task<Void, Never> {
        change({ try await $0.leave() }, then: then)
    }

    // MARK: Requests

    /// Make a change, which the settings (or the room list) show at once; a
    /// failure shows as `error`. `then` says whether it was made.
    private func change(
        _ call: @escaping @MainActor (Room) async throws -> Void,
        then: @escaping @MainActor (Bool) -> Void = { _ in }
    ) -> Task<Void, Never> {
        let room = self.room
        return work.request {
            try await call(room.get())
            then(true)
        } failed: { message in
            self.error = message
            then(false)
        }
    }

    /// Change the members, then read them again.
    private func changeMembers(
        _ call: @escaping @MainActor (Room) async throws -> Void,
        then: @escaping @MainActor (Bool) -> Void = { _ in }
    ) -> Task<Void, Never> {
        change(
            {
                try await call($0)
                await self.loadMembers()
            }, then: then)
    }
}
