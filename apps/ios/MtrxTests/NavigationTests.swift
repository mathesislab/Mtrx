import MtrxFFI
import XCTest
@testable import Mtrx

@MainActor
final class NavigationTests: XCTestCase {
    private var defaults: UserDefaults!
    private var session: FakeSession!

    override func setUp() async throws {
        defaults = UserDefaults(suiteName: "NavigationTests")
        defaults.removePersistentDomain(forName: "NavigationTests")
        session = FakeSession(noHandle: .init())
    }

    override func tearDown() async throws {
        defaults.removePersistentDomain(forName: "NavigationTests")
    }

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

    /// Not on the last room, as before: the rooms opened lately are no
    /// longer kept.
    func testTheRoomsTabOpensOnItsList() async {
        defaults.set(["!lab"], forKey: "recentRooms")
        let model = await launch()
        await send(Make.roomsBySpace(), to: model)
        XCTAssertEqual(model.home.roomsPath, [])
        XCTAssertNil(defaults.stringArray(forKey: "recentRooms"))
    }

    func testRoomsOpenInTheirTabs() async {
        let model = await launch()
        await send(Make.roomsBySpace(), to: model)
        model.home.open("!lab")
        XCTAssertEqual(model.home.tab, .rooms)
        XCTAssertEqual(model.home.roomsPath, ["!lab"])
        model.home.open("!bob")
        XCTAssertEqual(model.home.tab, .chats)
        XCTAssertEqual(model.home.chatsPath, ["!bob"])
        model.home.open("!team")
        XCTAssertEqual(model.home.tab, .spaces)
        XCTAssertEqual(model.home.spacesPath, [.space(id: "!team")])
        // Not in the list yet: as said.
        model.home.open("!new", kind: .space)
        XCTAssertEqual(model.home.selectedSpace, .space(id: "!new"))
    }

    func testTappingTheRoomsTabAgainGoesToItsList() async {
        let model = await launch()
        model.home.select(.chats)
        model.home.open("!lab")
        model.home.select(.chats)
        XCTAssertEqual(model.home.tab, .chats)
        model.home.select(.rooms)
        XCTAssertEqual(model.home.roomsPath, ["!lab"])
        model.home.select(.rooms)
        XCTAssertEqual(model.home.roomsPath, [])
        XCTAssertEqual(model.home.tab, .rooms)
    }

    /// Coming as the library says, until listed.
    func testAJustMadeSpaceIsNotLetGoBeforeItComes() async {
        let model = await launch()
        await send(Make.roomsBySpace(), to: model)
        model.home.open("!new", kind: .space)
        await send(Make.roomsBySpace(joining: ["!new"]), to: model)
        XCTAssertEqual(model.home.selectedSpace, .space(id: "!new"))
        let action = { (id: String) in model.rooms.button(for: Make.directoryRoom(id)).action }
        XCTAssertEqual(action("!new"), .open)
        XCTAssertEqual(action("!lab"), .open)
        XCTAssertEqual(action("!party"), .invited)
        XCTAssertEqual(action("!elsewhere"), .join)
        // Not coming any more, nor listed: gone.
        await send(Make.roomsBySpace(), to: model)
        XCTAssertNil(model.home.selectedSpace)
    }

    func testAcceptingAnInviteOpensTheRoom() async throws {
        let model = await launch()
        await send(Make.roomsBySpace(), to: model)
        try await model.accept(Make.room("!party", membership: .invited))
        XCTAssertEqual(session.changes, ["join !party"])
        XCTAssertEqual(model.home.tab, .rooms)
        XCTAssertEqual(model.home.roomsPath, ["!party"])
    }

    func testDecliningAnInviteLeavesIt() async throws {
        let model = await launch()
        await send(Make.roomsBySpace(), to: model)
        model.home.open("!party")
        try await model.decline(Make.room("!party", membership: .invited))
        XCTAssertEqual(session.changes, ["leave !party"])
        XCTAssertEqual(model.home.roomsPath, [])
    }

    func testOurServerName() async {
        let model = await launch()
        XCTAssertEqual(model.serverName, "example.org")
    }
}
