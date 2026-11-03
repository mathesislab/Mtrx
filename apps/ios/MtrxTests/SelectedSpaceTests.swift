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
        await eventually { model.lists === lists }
    }

    func testOpensOnTheSpacesWithoutASavedSpace() async {
        let model = await launch()
        XCTAssertNil(model.selectedSpace)
        XCTAssertEqual(model.spacesPath, [])
    }

    func testTheSelectedSpaceIsRestoredOnTheRoomsPage() async {
        let first = await launch()
        await send(Make.roomsBySpace(), to: first)
        first.selectSpace(.space(id: "!team"))
        XCTAssertEqual(first.spacesPath, [.space(id: "!team")])

        let next = await launch()
        XCTAssertEqual(next.selectedSpace, .space(id: "!team"))
        XCTAssertEqual(next.spacesPath, [.space(id: "!team")])
        await send(Make.roomsBySpace(), to: next)
        XCTAssertEqual(next.selectedSpace, .space(id: "!team"))
        XCTAssertEqual(next.spacesPath, [.space(id: "!team")])
    }

    func testHomeIsRestoredToo() async {
        let first = await launch()
        first.selectSpace(.outside)
        let next = await launch()
        await send(Make.roomsBySpace(), to: next)
        XCTAssertEqual(next.selectedSpace, .outside)
    }

    func testAGoneSpaceIsLetGoOnceTheListIsLoaded() async {
        defaults.set("!left", forKey: "selectedSpace")
        let model = await launch()
        XCTAssertEqual(model.selectedSpace, .space(id: "!left"))

        // A cache: the space may come with sync.
        await send(Make.roomsBySpace(loaded: false), to: model)
        XCTAssertEqual(model.selectedSpace, .space(id: "!left"))
        XCTAssertEqual(model.spacesPath, [.space(id: "!left")])

        await send(Make.roomsBySpace(), to: model)
        XCTAssertNil(model.selectedSpace)
        XCTAssertEqual(model.spacesPath, [])
        XCTAssertNil(defaults.string(forKey: "selectedSpace"))
    }

    func testASpaceThatDisappearsWhileShownIsLetGo() async {
        let model = await launch()
        await send(Make.roomsBySpace(), to: model)
        model.selectSpace(.outside)

        // Party and Loose gone: Home lists none.
        let inSpaces = Make.roomsBySpaceRooms.filter { !["!party", "!loose"].contains($0.id) }
        await send(Make.lists(inSpaces, children: Make.roomsBySpaceChildren), to: model)
        XCTAssertNil(model.selectedSpace)
        XCTAssertEqual(model.spacesPath, [])
    }

    /// The Rooms tab's groups folded here: Home and spaces alike, the
    /// others unfolded.
    func testTheRoomsTabsFoldedGroupsAreRestored() async {
        // The key of Home's group, as the library gives it.
        let home = "home"
        let first = await launch()
        await send(Make.roomsBySpace(), to: first)
        XCTAssertEqual(first.roomGroups.map(\.folded), [false, false])
        first.toggleGroupFold("!team")
        first.toggleGroupFold(home)
        first.toggleGroupFold(home)
        XCTAssertEqual(first.roomGroups.map(\.folded), [false, true])

        let next = await launch()
        await send(Make.roomsBySpace(), to: next)
        XCTAssertEqual(next.roomGroups.map(\.key), [home, "!team"])
        XCTAssertEqual(next.roomGroups.map(\.folded), [false, true])
        XCTAssertEqual(
            next.roomGroups.map(\.rooms), next.lists.roomGroups(among: nil, folded: []).map(\.rooms))
    }

    func testSigningOutForgetsTheSpace() async throws {
        let model = await launch()
        model.selectSpace(.space(id: "!team"))
        model.toggleGroupFold("!team")
        try await model.logout()
        XCTAssertNil(defaults.string(forKey: "selectedSpace"))
        XCTAssertEqual(model.foldedGroups, [])
        XCTAssertNil(defaults.stringArray(forKey: "foldedRoomGroups"))

        let next = await launch()
        next.selectSpace(.space(id: "!team"))
        try await next.forget()
        XCTAssertNil(defaults.string(forKey: "selectedSpace"))
    }
}
