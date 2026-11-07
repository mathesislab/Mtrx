import Combine
import Foundation
import MtrxFFI

/// Where the home screen is, and how this phone lays out the room lists:
/// the tab shown, the rooms open over the Chats and Rooms tabs, the Spaces
/// tab's selected space, an invite to answer, the order of the spaces'
/// rooms, the folded groups and the spaces' sections. The selected space,
/// the order, the folds and the sections are remembered across launches
/// (all but the order until sign-out).
@MainActor
final class HomeNavigation: ObservableObject {
    /// The tab shown.
    @Published var tab = HomeTab.chats
    /// The chats opened over the Chats tab's list.
    @Published var chatsPath: [String] = []
    /// The rooms opened over the Rooms tab's list.
    @Published var roomsPath: [String] = []
    /// An invite to answer, while its sheet shows.
    @Published var invite: RoomRef?
    /// The Spaces tab's selected space, whose rooms it shows.
    @Published private(set) var selectedSpace: SpaceKey?
    /// The order of every space's rooms (not of the chats).
    @Published private(set) var roomSort: RoomSort
    /// The Rooms tab's groups folded on this phone, by `ListSection.key`
    /// (a space's ID, or Home's).
    @Published private(set) var foldedGroups: Set<String>
    /// How this phone shows each space's sections.
    let sectionLayouts: SectionLayouts

    private let rooms: RoomListModel
    private let defaults: UserDefaults
    private var follows: [AnyCancellable] = []
    private static let selectedSpaceKey = "selectedSpace"
    private static let roomSortKey = "roomSort"
    private static let foldedGroupsKey = "foldedRoomGroups"
    /// The rooms opened lately, kept by earlier versions.
    private static let recentKey = "recentRooms"

    init(rooms: RoomListModel, defaults: UserDefaults) {
        self.rooms = rooms
        self.defaults = defaults
        roomSort = defaults.string(forKey: Self.roomSortKey).flatMap(RoomSort.init(saved:)) ?? .listOrder
        // Opens on the space's rooms, until the room list says it is gone.
        selectedSpace = defaults.string(forKey: Self.selectedSpaceKey).map(SpaceKey.init(saved:))
        sectionLayouts = SectionLayouts(defaults: defaults)
        foldedGroups = Set(defaults.stringArray(forKey: Self.foldedGroupsKey) ?? [])
        defaults.removeObject(forKey: Self.recentKey)
        follows = [
            sectionLayouts.objectWillChange.sink { [weak self] _ in
                MainActor.assumeIsolated { self?.objectWillChange.send() }
            },
            // Sent on the main actor, as the lists are set.
            rooms.$lists.sink { [weak self] lists in
                MainActor.assumeIsolated { self?.dropGoneSpace(lists) }
            },
        ]
    }

    /// Once the lists are loaded (the first may be from a cache: rooms may
    /// come with sync), a selected space they don't list, and that is not
    /// coming, is let go.
    private func dropGoneSpace(_ lists: RoomLists) {
        guard lists.loaded(), let selectedSpace, lists.space(key: selectedSpace) == nil else { return }
        if case .space(let id) = selectedSpace, lists.joining().contains(id) { return }
        selectSpace(nil)
    }

    /// Show `entry`'s rooms (and remember it), or the spaces with none
    /// selected.
    func selectSpace(_ entry: SpaceKey?) {
        selectedSpace = entry
        if let entry {
            defaults.set(entry.saved, forKey: Self.selectedSpaceKey)
        } else {
            defaults.removeObject(forKey: Self.selectedSpaceKey)
        }
    }

    /// The Spaces tab's pages over its list: the selected space's rooms.
    var spacesPath: [SpaceKey] {
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
        switch kind ?? rooms.lists.room(roomId: id)?.kind {
        case .direct:
            chatsPath = [id]
            tab = .chats
        case .space:
            selectSpace(.space(id: id))
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
        if selectedSpace == .space(id: id) {
            selectSpace(nil)
        }
    }

    /// The Rooms tab's groups, folded as on this phone.
    var roomGroups: [ListSection] {
        rooms.lists.roomGroups(among: nil, folded: foldedGroups.sorted())
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
        defaults.set(sort.saved, forKey: Self.roomSortKey)
    }

    /// The selected space, the spaces' sections and the Rooms tab's folded
    /// groups are the account's: forgotten at sign-out.
    func forget() {
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
