import MtrxFFI
import XCTest
@testable import Mtrx

/// The Spaces tab's selected space and the Rooms tab's folded groups,
/// remembered across launches until sign-out.
@MainActor
final class SelectedSpaceTests: XCTestCase {
    private var defaults: UserDefaults!
    private var session: FakeSession!

    override func setUp() async throws {
        defaults = UserDefaults(suiteName: "SelectedSpaceTests")
        defaults.removePersistentDomain(forName: "SelectedSpaceTests")
        session = FakeSession(noHandle: .init())
    }

    override func tearDown() async throws {
        defaults.removePersistentDomain(forName: "SelectedSpaceTests")
    }

    /// A model as at a launch, following the room list.
    private func launch() async -> SessionModel {
        session.roomListListener = nil
        let model = SessionModel(session: session, defaults: defaults)
        model.start()
        await eventually { self.session.roomListListener != nil }
        return model
    }

    private func send(_ lists: RoomLists, to model: SessionModel) async {
        session.roomListListener?.onUpdate(lists: lists)
        await eventually { model.rooms.lists === lists }
    }

    func testOpensOnTheSpacesWithoutASavedSpace() async {
        let model = await launch()
        XCTAssertNil(model.home.selectedSpace)
        XCTAssertEqual(model.home.spacesPath, [])
    }

    func testTheSelectedSpaceIsRestoredOnTheRoomsPage() async {
        let first = await launch()
        await send(Make.roomsBySpace(), to: first)
        first.home.selectSpace(.space(id: "!team"))
        XCTAssertEqual(first.home.spacesPath, [.space(id: "!team")])

        let next = await launch()
        XCTAssertEqual(next.home.selectedSpace, .space(id: "!team"))
        XCTAssertEqual(next.home.spacesPath, [.space(id: "!team")])
        await send(Make.roomsBySpace(), to: next)
        XCTAssertEqual(next.home.selectedSpace, .space(id: "!team"))
        XCTAssertEqual(next.home.spacesPath, [.space(id: "!team")])
    }

    func testHomeIsRestoredToo() async {
        let first = await launch()
        first.home.selectSpace(.outside)
        let next = await launch()
        await send(Make.roomsBySpace(), to: next)
        XCTAssertEqual(next.home.selectedSpace, .outside)
    }

    func testAGoneSpaceIsLetGoOnceTheListIsLoaded() async {
        defaults.set("!left", forKey: "selectedSpace")
        let model = await launch()
        XCTAssertEqual(model.home.selectedSpace, .space(id: "!left"))

        // A cache: the space may come with sync.
        await send(Make.roomsBySpace(loaded: false), to: model)
        XCTAssertEqual(model.home.selectedSpace, .space(id: "!left"))
        XCTAssertEqual(model.home.spacesPath, [.space(id: "!left")])

        await send(Make.roomsBySpace(), to: model)
        XCTAssertNil(model.home.selectedSpace)
        XCTAssertEqual(model.home.spacesPath, [])
        XCTAssertNil(defaults.string(forKey: "selectedSpace"))
    }

    func testASpaceThatDisappearsWhileShownIsLetGo() async {
        let model = await launch()
        await send(Make.roomsBySpace(), to: model)
        model.home.selectSpace(.outside)

        // Party and Loose gone: Home lists none.
        let inSpaces = Make.roomsBySpaceRooms.filter { !["!party", "!loose"].contains($0.id) }
        await send(Make.lists(inSpaces, children: Make.roomsBySpaceChildren), to: model)
        XCTAssertNil(model.home.selectedSpace)
        XCTAssertEqual(model.home.spacesPath, [])
    }

    /// The Rooms tab's groups folded here: Home and spaces alike, the
    /// others unfolded.
    func testTheRoomsTabsFoldedGroupsAreRestored() async {
        // The key of Home's group, as the library gives it.
        let home = "home"
        let first = await launch()
        await send(Make.roomsBySpace(), to: first)
        XCTAssertEqual(first.home.roomGroups.map(\.folded), [false, false])
        first.home.toggleGroupFold("!team")
        first.home.toggleGroupFold(home)
        first.home.toggleGroupFold(home)
        XCTAssertEqual(first.home.roomGroups.map(\.folded), [false, true])

        let next = await launch()
        await send(Make.roomsBySpace(), to: next)
        XCTAssertEqual(next.home.roomGroups.map(\.key), [home, "!team"])
        XCTAssertEqual(next.home.roomGroups.map(\.folded), [false, true])
        XCTAssertEqual(
            next.home.roomGroups.map(\.rooms), next.rooms.lists.roomGroups(among: nil, folded: []).map(\.rooms))
    }

    func testSigningOutForgetsTheSpace() async throws {
        let model = await launch()
        model.home.selectSpace(.space(id: "!team"))
        model.home.toggleGroupFold("!team")
        try await model.logout()
        XCTAssertNil(defaults.string(forKey: "selectedSpace"))
        XCTAssertEqual(model.home.foldedGroups, [])
        XCTAssertNil(defaults.stringArray(forKey: "foldedRoomGroups"))

        let next = await launch()
        next.home.selectSpace(.space(id: "!team"))
        try await next.forget()
        XCTAssertNil(defaults.string(forKey: "selectedSpace"))
    }
}
