import Foundation
import MtrxFFI

// The lists the app shows come from the library (`RoomLists`); what is
// kept on the phone stays here.

extension SpaceKey {
    /// Room IDs start with "!", so the saved form of Home cannot be one
    /// (from when it was called Other rooms).
    private static let homeSaved = "other-rooms"

    /// The form kept in the preferences.
    var saved: String {
        switch self {
        case .space(let id): return id
        case .home, .outside: return Self.homeSaved
        }
    }

    init(saved: String) {
        self = saved == Self.homeSaved ? .outside : .space(id: saved)
    }
}

extension RoomSummary {
    /// A direct chat's other person: not ourselves, in the Notes to self.
    func person(ours: String) -> User? {
        guard kind == .direct, let user = directUser, user.userId != ours else { return nil }
        return user
    }
}

extension SpaceEntry: Identifiable {
    public var id: SpaceKey { key }
    var name: String { space?.name ?? "Home" }
}

extension ListSection: Identifiable {
    public var id: String { key }
}

/// The order of the spaces' rooms, chosen on a space's page; remembered.
extension RoomSort: Identifiable {
    public var id: Self { self }

    static let allCases: [RoomSort] = [.listOrder, .aToZ, .zToA, .mostRecent]

    var title: String {
        switch self {
        case .listOrder: return "Default"
        case .aToZ: return "A to Z"
        case .zToA: return "Z to A"
        case .mostRecent: return "Most recent"
        }
    }

    /// The form kept in the preferences.
    var saved: String {
        switch self {
        case .listOrder: return "default"
        case .aToZ: return "aToZ"
        case .zToA: return "zToA"
        case .mostRecent: return "mostRecent"
        }
    }

    init?(saved: String) {
        guard let sort = Self.allCases.first(where: { $0.saved == saved }) else { return nil }
        self = sort
    }
}
