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
    /// The settings' subscription, once started.
    private let subscriptions = Subscriptions()
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
            subscriptions.add(room.subscribeSettings(listener: feed), feed) { [weak self] settings in
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
    /// once. Returns whether saved.
    func setTopic(_ topic: String) async -> Bool {
        await call { try await $0.setTopic(topic: topic) }
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
        Task {
            if await call({ try await $0.setLevel(userId: user, level: level) }) {
                await loadMembers()
            }
        }
    }

    /// Remove someone, or ban them; they see `reason`.
    @discardableResult
    func remove(_ user: String, reason: String, ban: Bool) -> Task<Void, Never> {
        let reason = reason.trimmingCharacters(in: .whitespacesAndNewlines)
        return Task {
            if await call({
                try await $0.remove(userId: user, ban: ban, reason: reason.isEmpty ? nil : reason)
            }) {
                await loadMembers()
            }
        }
    }

    /// Invite someone by user ID. Returns whether invited.
    func invite(_ user: String) async -> Bool {
        let user = user.trimmingCharacters(in: .whitespaces)
        let invited = await call { try await $0.invite(userId: user) }
        if invited {
            await loadMembers()
        }
        return invited
    }

    /// Leave the room or space. Returns whether left.
    func leave() async -> Bool {
        await call { try await $0.leave() }
    }

    // MARK: Calls

    /// Make a change, which the settings (or the room list) show at once.
    private func change(_ call: @escaping (Room) async throws -> Void) -> Task<Void, Never> {
        Task { _ = await self.call(call) }
    }

    /// Run `call` on the room; a failure shows as `error`. Returns whether
    /// it succeeded.
    private func call(_ call: (Room) async throws -> Void) async -> Bool {
        do {
            try await call(room.get())
            return true
        } catch {
            self.error = errorMessage(error)
            return false
        }
    }
}
