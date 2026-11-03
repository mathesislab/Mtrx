import Foundation
import MtrxFFI

/// A room's or space's settings and members, for its header, topic panel,
/// info screen and overview: read from the server, changed where we may.
/// A change shows at once; the settings are read again once the server
/// has it (the library returns once sync brought it back), or on failure,
/// with the reason.
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
    /// A change is on its way.
    @Published private(set) var saving = false
    /// Why the last change failed.
    @Published var error: String?

    /// The room, or why it is not known.
    private let room: Result<Room, Error>
    /// For the spaces the room is in.
    private let session: Session

    init(session: Session, roomId: String, userId: String) {
        self.session = session
        self.roomId = roomId
        self.userId = userId
        room = Result { try session.room(roomId: roomId) }
    }

    /// Read the settings and members, again if read before.
    func load() async {
        async let settings: () = loadSettings()
        async let members: () = loadMembers()
        _ = await (settings, members)
    }

    func loadSettings() async {
        do {
            settings = try await room.get().settings()
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
        return change { _ in } call: { try await $0.setName(name: name) }
    }

    @discardableResult
    func setNotify(_ mode: NotifyMode) -> Task<Void, Never> {
        change { $0.notify = mode } call: { try await $0.setNotify(mode: mode) }
    }

    @discardableResult
    func setAccess(_ access: Access) -> Task<Void, Never> {
        change { $0.access = access } call: { try await $0.setJoinRule(rule: access) }
    }

    @discardableResult
    func setListed(_ listed: Bool) -> Task<Void, Never> {
        change { $0.listed = listed } call: { try await $0.setListed(listed: listed) }
    }

    @discardableResult
    func setHistory(_ history: History) -> Task<Void, Never> {
        change { $0.history = history } call: { try await $0.setHistory(history: history) }
    }

    @discardableResult
    func setPermissions(_ permissions: Permissions) -> Task<Void, Never> {
        change { $0.permissions = permissions } call: { try await $0.setPermissions(permissions: permissions) }
    }

    /// Encrypt from now on: not to be undone.
    @discardableResult
    func encrypt() -> Task<Void, Never> {
        change { $0.encrypted = true } call: { try await $0.encrypt() }
    }

    /// The address `#name:ourserver`, or none (`name` empty).
    @discardableResult
    func setAddress(_ name: String) -> Task<Void, Never> {
        let name = name.trimmingCharacters(in: .whitespaces)
        return change { _ in } call: { try await $0.setAddress(name: name.isEmpty ? nil : name) }
    }

    @discardableResult
    func setSuggested(in space: String, _ suggested: Bool) -> Task<Void, Never> {
        let (session, roomId) = (self.session, self.roomId)
        return change { settings in
            if let at = settings.spaces.firstIndex(where: { $0.spaceId == space }) {
                settings.spaces[at].suggested = suggested
            }
        } call: { _ in try await session.room(roomId: space).setSuggested(roomId: roomId, suggested: suggested) }
    }

    @discardableResult
    func addToSpace(_ space: String) -> Task<Void, Never> {
        let (session, roomId) = (self.session, self.roomId)
        return change { settings in
            settings.canAddTo.removeAll { $0 == space }
            settings.spaces.append(SpaceLink(spaceId: space, suggested: false, canChange: true))
        } call: { _ in try await session.room(roomId: space).addChild(roomId: roomId) }
    }

    @discardableResult
    func removeFromSpace(_ space: String) -> Task<Void, Never> {
        let (session, roomId) = (self.session, self.roomId)
        return change { settings in
            settings.spaces.removeAll { $0.spaceId == space }
            settings.canAddTo.append(space)
        } call: { _ in try await session.room(roomId: space).removeChild(roomId: roomId) }
    }

    @discardableResult
    func unban(_ user: String) -> Task<Void, Never> {
        change { $0.banned.removeAll { $0.user.userId == user } } call: { try await $0.unban(userId: user) }
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
                if ban {
                    await loadSettings()
                }
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

    /// Show `update` at once, make the change, then read the settings again
    /// (also on failure, which shows why).
    private func change(
        _ update: (inout RoomSettings) -> Void, call: @escaping (Room) async throws -> Void
    ) -> Task<Void, Never> {
        if var changed = settings {
            update(&changed)
            settings = changed
        }
        return Task {
            _ = await self.call(call)
            await loadSettings()
        }
    }

    /// Run `call` on the room; a failure shows as `error`. Returns whether
    /// it succeeded.
    private func call(_ call: (Room) async throws -> Void) async -> Bool {
        saving = true
        defer { saving = false }
        do {
            try await call(room.get())
            return true
        } catch {
            self.error = errorMessage(error)
            return false
        }
    }
}
