import Foundation
import MtrxFFI

/// Rooms to join, as a space or a directory lists them: the details line,
/// the role, and what a row offers.
enum DirectoryRows {
    /// "Space · 4 rooms · the topic's first line · 12 members".
    static func details(_ room: DirectoryRoom) -> String {
        var parts: [String] = []
        if room.isSpace {
            switch room.children {
            case 0: parts.append("Space")
            case 1: parts.append("Space · 1 room")
            case let n: parts.append("Space · \(n) rooms")
            }
        }
        if let line = Topics.firstLine(room.topic) {
            parts.append(line)
        }
        parts.append(room.members == 1 ? "1 member" : "\(room.members) members")
        return parts.joined(separator: " · ")
    }

    /// Ours where we are a member: "Admin", "Mod".
    static func role(_ room: DirectoryRoom) -> String? {
        switch room.role {
        case .admin: return "Admin"
        case .moderator: return "Mod"
        case .member, nil: return nil
        }
    }

    /// What a row offers, as the room allows.
    enum Action: Equatable {
        case join
        case askToJoin
        case open
        /// An invite, to answer.
        case invited
        /// We asked, and wait.
        case asked
        case inviteOnly
        /// Joining or asking, on its way.
        case busy(knock: Bool)

        var title: String {
            switch self {
            case .join: return "Join"
            case .askToJoin: return "Ask to join"
            case .open: return "Open"
            case .invited: return "Invited"
            case .asked: return "Asked"
            case .inviteOnly: return "Invite only"
            case .busy(let knock): return knock ? "Asking…" : "Joining…"
            }
        }

        /// A button to press (in the primary colour for joining), not a
        /// word.
        var pressable: Bool {
            switch self {
            case .join, .askToJoin, .open, .invited: return true
            case .asked, .inviteOnly, .busy: return false
            }
        }

        var primary: Bool {
            self == .join || self == .askToJoin
        }
    }

    static func action(_ room: DirectoryRoom, busy: Bool) -> Action {
        if busy {
            return .busy(knock: room.access == .knock)
        }
        switch room.membership {
        case .joined: return .open
        case .invited: return .invited
        case .asked: return .asked
        case nil:
            switch room.access {
            case .public, .spaceMembers: return .join
            case .knock: return .askToJoin
            case .invite: return .inviteOnly
            }
        }
    }
}

/// Joining rooms from a list: join or ask to join, the row changing as
/// it goes (Joining…, then Open or Asked); a failure shows why.
@MainActor
class JoiningModel: ObservableObject {
    /// Rooms being joined or asked to join.
    @Published private(set) var busy: Set<String> = []
    /// Rooms asked to join here, until the list is read again.
    @Published private(set) var asked: Set<String> = []
    @Published var error: String?

    let session: Session

    init(session: Session) {
        self.session = session
    }

    /// `room` as shown: `joined` as the room list has it (rooms joined here
    /// are there at once), else asked if asked here.
    func shown(_ room: DirectoryRoom, joined: Bool) -> DirectoryRoom {
        var room = room
        if joined {
            room.membership = .joined
        } else if asked.contains(room.id) {
            room.membership = .asked
        }
        return room
    }

    func action(_ room: DirectoryRoom, joined: Bool) -> DirectoryRows.Action {
        DirectoryRows.action(shown(room, joined: joined), busy: busy.contains(room.id))
    }

    /// The list was read again: it has our memberships.
    func relisted() {
        asked = []
    }

    /// Join `room`, or ask to (rooms that take requests). Returns the room's
    /// ID once joined (nil if asked or failed).
    @discardableResult
    func join(_ room: DirectoryRoom) async -> String? {
        guard !busy.contains(room.id) else { return nil }
        busy.insert(room.id)
        defer { busy.remove(room.id) }
        let address = room.alias ?? room.id
        do {
            if room.access == .knock {
                _ = try await session.knock(room: address, via: room.via)
                asked.insert(room.id)
                return nil
            }
            return try await session.join(room: address, via: room.via)
        } catch {
            self.error = errorMessage(error)
            return nil
        }
    }
}

/// A space's rooms, as its server lists them with the joined ones it
/// leaves out (rooms only invited people may join left out): the last
/// list at once, updated behind it.
@MainActor
final class SpaceOverviewModel: JoiningModel {
    let spaceId: String
    /// As last listed.
    @Published private(set) var rooms: [DirectoryRoom]
    /// Reading the list again.
    @Published private(set) var updating = false
    /// The list could not be read.
    @Published private(set) var failed = false
    /// Never listed yet.
    @Published private(set) var unknown: Bool
    @Published var filter = ""

    init(session: Session, spaceId: String) {
        self.spaceId = spaceId
        let listed = session.listedSpaceChildren(spaceId: spaceId)
        rooms = listed ?? []
        unknown = listed == nil
        super.init(session: session)
    }

    /// The rooms whose name or topic has the filter, else all.
    var shown: [DirectoryRoom] {
        let filter = filter.trimmingCharacters(in: .whitespaces)
        guard !filter.isEmpty else { return rooms }
        return rooms.filter {
            $0.name.localizedCaseInsensitiveContains(filter)
                || ($0.topic?.localizedCaseInsensitiveContains(filter) ?? false)
        }
    }

    /// Read the list (again).
    func load() async {
        guard !updating else { return }
        updating = true
        failed = false
        defer { updating = false }
        do {
            rooms = try await session.spaceChildren(spaceId: spaceId)
            unknown = false
            relisted()
        } catch {
            failed = true
        }
    }
}

/// Finding a space or room to join: a server's directory, searched, or the
/// room an address or link names.
@MainActor
final class JoinModel: JoiningModel {
    /// The directory searched: ours if nil.
    @Published private(set) var server: String?
    /// Servers browsed before, most recent first.
    @Published private(set) var servers: [String] = []
    @Published var query = ""
    @Published var kind = DirectoryKind.all
    @Published private(set) var results: [DirectoryRoom] = []
    /// For the next page, if any.
    @Published private(set) var next: String?
    @Published private(set) var searching = false
    /// The room a pasted address names, once found.
    @Published private(set) var preview: DirectoryRoom?
    /// The address pasted, being looked up or shown.
    @Published private(set) var address: Address?
    /// "Nothing found.", or why the search failed.
    @Published private(set) var message: String?

    /// Our server's name.
    let ownServer: String
    private var search: Task<Void, Never>?

    init(session: Session, ownServer: String) {
        self.ownServer = ownServer
        super.init(session: session)
        servers = session.directoryServers().filter { $0 != ownServer }
    }

    /// Browse `server`'s directory (nil: ours).
    func choose(server: String?) {
        let server = server?.trimmingCharacters(in: .whitespaces).lowercased()
        self.server = server == ownServer || server?.isEmpty == true ? nil : server
        searchSoon(delay: 0)
    }

    /// Search again, after a pause for more typing.
    func searchSoon(delay: UInt64 = 300_000_000) {
        search?.cancel()
        search = Task {
            if delay > 0 {
                try? await Task.sleep(nanoseconds: delay)
                guard !Task.isCancelled else { return }
            }
            await self.run()
        }
    }

    /// The search, or the room an address names.
    func run() async {
        if let address = parseAddress(text: query) {
            await lookUp(address)
            return
        }
        address = nil
        preview = nil
        searching = true
        message = nil
        defer { searching = false }
        let server = self.server
        do {
            let page = try await session.publicRooms(server: server, query: query, kind: kind, page: nil)
            guard !Task.isCancelled else { return }
            results = page.rooms
            next = page.next
            message = page.rooms.isEmpty ? "Nothing found." : nil
            if let server {
                try? await session.rememberDirectoryServer(server: server)
                servers = [server] + servers.filter { $0 != server }
            }
        } catch {
            guard !Task.isCancelled else { return }
            results = []
            next = nil
            if let server, let failure = error as? MtrxError, case .InvalidInput = failure {
                message = "“\(server)” is not a server name."
            } else {
                message = errorMessage(error)
            }
        }
    }

    /// The next page of results.
    func more() async {
        guard let next, !searching else { return }
        searching = true
        defer { searching = false }
        do {
            let page = try await session.publicRooms(server: server, query: query, kind: kind, page: next)
            results += page.rooms
            self.next = page.next
        } catch {
            message = errorMessage(error)
        }
    }

    private func lookUp(_ address: Address) async {
        self.address = address
        preview = nil
        results = []
        next = nil
        message = "Looking it up…"
        searching = true
        defer { searching = false }
        do {
            preview = try await session.preview(address: address)
            message = nil
        } catch {
            guard !Task.isCancelled else { return }
            message = "Could not find it: \(errorMessage(error) ?? "")"
        }
    }
}

/// A new room or space: its name, topic, encryption and who edits it, and
/// people to invite, some of them co-owners where the server offers it.
@MainActor
final class NewRoomModel: ObservableObject {
    /// A space, not a room.
    let space: Bool
    /// The space to make it in, if any: its ID and name.
    let parent: (id: String, name: String)?
    @Published var name = ""
    @Published var topic = ""
    @Published var encrypted = true
    @Published var moderated = false
    /// The user ID being typed, to invite.
    @Published var person = ""
    @Published private(set) var invited: [String] = []
    @Published private(set) var coOwners: Set<String> = []
    /// The server gives new rooms co-owners.
    @Published private(set) var offersOwners = false
    @Published private(set) var creating = false
    @Published var error: String?

    private let session: Session

    init(session: Session, space: Bool, parent: (id: String, name: String)? = nil) {
        self.session = session
        self.space = space
        self.parent = parent
    }

    var title: String {
        if space { return "New space" }
        if let parent { return "New room in \(parent.name)" }
        return "New room"
    }

    func load() async {
        offersOwners = await session.offersOwners()
    }

    /// The typed ID looks like one: "@name:server".
    var personValid: Bool {
        Self.isUserId(person.trimmingCharacters(in: .whitespaces))
    }

    static func isUserId(_ text: String) -> Bool {
        guard text.hasPrefix("@"), let colon = text.firstIndex(of: ":") else { return false }
        return colon > text.index(after: text.startIndex) && colon < text.index(before: text.endIndex)
    }

    /// Invite the person typed.
    func addPerson() {
        let id = person.trimmingCharacters(in: .whitespaces)
        guard Self.isUserId(id) else { return }
        if !invited.contains(id) {
            invited.append(id)
        }
        person = ""
    }

    func removePerson(_ id: String) {
        invited.removeAll { $0 == id }
        coOwners.remove(id)
    }

    func setCoOwner(_ id: String, _ on: Bool) {
        if on {
            coOwners.insert(id)
        } else {
            coOwners.remove(id)
        }
    }

    var canCreate: Bool {
        !name.trimmingCharacters(in: .whitespaces).isEmpty && !creating
    }

    /// Make it; its room ID, or nil on failure (`error`).
    func create() async -> String? {
        guard canCreate else { return nil }
        creating = true
        defer { creating = false }
        let new = NewRoom(
            name: name.trimmingCharacters(in: .whitespaces), topic: topic, space: space,
            encrypted: encrypted && !space, moderated: moderated, invite: invited,
            coOwners: offersOwners ? invited.filter(coOwners.contains) : [], parent: parent?.id)
        do {
            return try await session.createRoom(new: new)
        } catch {
            let what = space ? "space" : "room"
            self.error = "Could not create the \(what) “\(new.name)”: \(errorMessage(error) ?? "")"
            return nil
        }
    }
}
