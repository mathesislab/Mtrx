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

    private let session: Session

    init(session: Session, roomId: String, userId: String) {
        self.session = session
        self.roomId = roomId
        self.userId = userId
    }

    /// Read the settings and members, again if read before.
    func load() async {
        async let settings: () = loadSettings()
        async let members: () = loadMembers()
        _ = await (settings, members)
    }

    func loadSettings() async {
        do {
            settings = try await session.roomSettings(roomId: roomId)
        } catch {
            loadError = errorMessage(error)
        }
    }

    func loadMembers() async {
        do {
            members = try await session.members(roomId: roomId)
        } catch {
            loadError = errorMessage(error)
        }
    }

    // MARK: Settings

    /// Save `topic` (as `composeTopic` makes it): the room list shows it at
    /// once. Returns whether saved.
    func setTopic(_ topic: String) async -> Bool {
        await call { try await $0.setRoomTopic(roomId: $1, topic: topic) }
    }

    @discardableResult
    func rename(_ name: String) -> Task<Void, Never> {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return change { _ in } call: { try await $0.setRoomName(roomId: $1, name: name) }
    }

    @discardableResult
    func setNotify(_ mode: NotifyMode) -> Task<Void, Never> {
        change { $0.notify = mode } call: { try await $0.setNotifyMode(roomId: $1, mode: mode) }
    }

    @discardableResult
    func setAccess(_ access: Access) -> Task<Void, Never> {
        change { $0.access = access } call: { try await $0.setRoomAccess(roomId: $1, access: access) }
    }

    @discardableResult
    func setListed(_ listed: Bool) -> Task<Void, Never> {
        change { $0.listed = listed } call: { try await $0.setListed(roomId: $1, listed: listed) }
    }

    @discardableResult
    func setHistory(_ history: History) -> Task<Void, Never> {
        change { $0.history = history } call: { try await $0.setHistory(roomId: $1, history: history) }
    }

    @discardableResult
    func setPermissions(_ permissions: Permissions) -> Task<Void, Never> {
        change { $0.permissions = permissions } call: {
            try await $0.setPermissions(roomId: $1, permissions: permissions)
        }
    }

    /// Encrypt from now on: not to be undone.
    @discardableResult
    func encrypt() -> Task<Void, Never> {
        change { $0.encrypted = true } call: { try await $0.encryptRoom(roomId: $1) }
    }

    /// The address `#name:ourserver`, or none (`name` empty).
    @discardableResult
    func setAddress(_ name: String) -> Task<Void, Never> {
        let name = name.trimmingCharacters(in: .whitespaces)
        return change { _ in } call: { try await $0.setAddress(roomId: $1, name: name.isEmpty ? nil : name) }
    }

    @discardableResult
    func setSuggested(in space: String, _ suggested: Bool) -> Task<Void, Never> {
        change { settings in
            if let at = settings.spaces.firstIndex(where: { $0.spaceId == space }) {
                settings.spaces[at].suggested = suggested
            }
        } call: { try await $0.setSuggested(spaceId: space, roomId: $1, suggested: suggested) }
    }

    @discardableResult
    func addToSpace(_ space: String) -> Task<Void, Never> {
        change { settings in
            settings.canAddTo.removeAll { $0 == space }
            settings.spaces.append(SpaceLink(spaceId: space, suggested: false, canChange: true))
        } call: { try await $0.addToSpace(spaceId: space, roomId: $1) }
    }

    @discardableResult
    func removeFromSpace(_ space: String) -> Task<Void, Never> {
        change { settings in
            settings.spaces.removeAll { $0.spaceId == space }
            settings.canAddTo.append(space)
        } call: { try await $0.removeFromSpace(spaceId: space, roomId: $1) }
    }

    @discardableResult
    func unban(_ user: String) -> Task<Void, Never> {
        change { $0.banned.removeAll { $0.userId == user } } call: { try await $0.unban(roomId: $1, userId: user) }
    }

    // MARK: Members

    /// Give someone (or ourselves) a power level.
    @discardableResult
    func setLevel(_ level: Int64, of user: String) -> Task<Void, Never> {
        Task {
            if await call({ try await $0.setLevel(roomId: $1, userId: user, level: level) }) {
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
                try await $0.removeMember(roomId: $1, userId: user, reason: reason.isEmpty ? nil : reason, ban: ban)
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
        let invited = await call { try await $0.invite(roomId: $1, userId: user) }
        if invited {
            await loadMembers()
        }
        return invited
    }

    /// Leave the room or space. Returns whether left.
    func leave() async -> Bool {
        await call { try await $0.leave(roomId: $1) }
    }

    // MARK: Calls

    /// Show `update` at once, make the change, then read the settings again
    /// (also on failure, which shows why).
    private func change(
        _ update: (inout RoomSettings) -> Void, call: @escaping (Session, String) async throws -> Void
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

    /// Run `call` on the session with the room's ID; a failure shows as
    /// `error`. Returns whether it succeeded.
    private func call(_ call: (Session, String) async throws -> Void) async -> Bool {
        saving = true
        defer { saving = false }
        do {
            try await call(session, roomId)
            return true
        } catch {
            self.error = errorMessage(error)
            return false
        }
    }
}
