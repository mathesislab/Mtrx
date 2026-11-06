import MtrxFFI
import XCTest
@testable import Mtrx

/// Rows of rooms to join: their details and what they offer.
final class DirectoryRowsTests: XCTestCase {
    func testDetails() {
        XCTAssertEqual(
            DirectoryRows.details(Make.directoryRoom("!s", space: true, members: 12, topic: "\nIce\nmore", children: 4)),
            "Space · 4 rooms · Ice · 12 members")
        XCTAssertEqual(DirectoryRows.details(Make.directoryRoom("!r", members: 1)), "1 member")
    }

    func testRole() {
        XCTAssertEqual(DirectoryRows.role(Make.directoryRoom("!a", role: .moderator)), "Mod")
    }

    /// The library's: our membership as the room lists have it first,
    /// then what the room lets us do.
    func testActions() {
        let lists = Make.lists([
            Make.room("!lab"), Make.room("!offer", membership: .invited), Make.room("!door", membership: .asked),
        ])
        let action = { (room: DirectoryRoom) in lists.joinButton(room: room).action }
        XCTAssertEqual(action(Make.directoryRoom("!a")), .join)
        XCTAssertEqual(action(Make.directoryRoom("!a", joinRule: .spaceMembers)), .join)
        XCTAssertEqual(action(Make.directoryRoom("!a", joinRule: .knock)), .askToJoin)
        XCTAssertEqual(action(Make.directoryRoom("!a", joinRule: .invite)), .inviteOnly)
        XCTAssertEqual(action(Make.directoryRoom("!lab")), .open)
        XCTAssertEqual(action(Make.directoryRoom("!offer")), .invited)
        XCTAssertEqual(action(Make.directoryRoom("!door", joinRule: .knock)), .asked)
        XCTAssertEqual(lists.joinButton(room: Make.directoryRoom("!a", joinRule: .knock)).title, "Ask to join")
        XCTAssertFalse(JoinAction.asked.pressable)
    }
}

@MainActor
final class SpaceOverviewModelTests: XCTestCase {
    private var session: FakeSession!

    override func setUp() async throws {
        session = FakeSession(noHandle: .init())
    }

    func testShowsTheLastListAtOnceThenTheNewOne() async {
        session.listed["!team"] = [Make.directoryRoom("!old")]
        session.children = .success([Make.directoryRoom("!lab", name: "Lab"), Make.directoryRoom("!deep", name: "Deep")])
        let model = SpaceOverviewModel(session: session, spaceId: "!team")
        XCTAssertEqual(model.rooms.map(\.id), ["!old"])
        XCTAssertFalse(model.unknown)
        await model.load()
        XCTAssertEqual(model.rooms.map(\.id), ["!lab", "!deep"])
        model.filter = "de"
        XCTAssertEqual(model.shown.map(\.id), ["!deep"])
    }

    func testNeverListedUntilLoaded() async {
        session.children = .success([Make.directoryRoom("!lab")])
        let model = SpaceOverviewModel(session: session, spaceId: "!team")
        XCTAssertTrue(model.unknown)
        await model.load()
        XCTAssertFalse(model.unknown)
        XCTAssertEqual(model.rooms.map(\.id), ["!lab"])
    }

    func testAFailureKeepsTheLastList() async {
        session.listed["!team"] = [Make.directoryRoom("!old")]
        session.children = .failure(MtrxError.Other(reason: "timed out"))
        let model = SpaceOverviewModel(session: session, spaceId: "!team")
        await model.load()
        XCTAssertTrue(model.failed)
        XCTAssertEqual(model.rooms.map(\.id), ["!old"])
    }

    /// Joins, or asks to join a room that takes requests; busy meanwhile.
    /// Open or Asked after is the room lists' (the library's).
    func testJoinsOrAsks() async {
        let lab = Make.directoryRoom("!lab")
        let door = Make.directoryRoom("!door", joinRule: .knock)
        session.children = .success([lab, door])
        let model = SpaceOverviewModel(session: session, spaceId: "!team")
        await model.load()
        let joined = await model.join(lab)
        XCTAssertEqual(joined, "!lab")
        let asked = await model.join(door)
        XCTAssertNil(asked)
        XCTAssertEqual(session.changes, ["join !lab", "knock !door"])
        XCTAssertFalse(model.isBusy(door))
    }

    func testAFailedJoinSaysWhy() async {
        session.changeError = MtrxError.Other(reason: "Not allowed")
        let model = SpaceOverviewModel(session: session, spaceId: "!team")
        let joined = await model.join(Make.directoryRoom("!lab"))
        XCTAssertNil(joined)
        XCTAssertEqual(model.error, "Not allowed")
        XCTAssertFalse(model.isBusy(Make.directoryRoom("!lab")))
    }
}

@MainActor
final class JoinModelTests: XCTestCase {
    private var session: FakeSession!
    private var model: JoinModel!

    override func setUp() async throws {
        session = FakeSession(noHandle: .init())
        model = JoinModel(session: session, ownServer: "example.org")
    }

    func testSearchesOurDirectory() async {
        session.directory = .success(DirectoryPage(rooms: [Make.directoryRoom("!ice")], next: "2"))
        model.query = "ice"
        model.kind = .rooms
        await model.run()
        XCTAssertEqual(model.results.map(\.id), ["!ice"])
        XCTAssertEqual(model.next, "2")
        XCTAssertNil(session.directoryRequests.last?.server)
        XCTAssertEqual(session.directoryRequests.last?.query, "ice")
        XCTAssertEqual(session.directoryRequests.last?.kind, .rooms)
        XCTAssertEqual(model.servers, ["other.org"])
    }

    func testAnotherServerIsRemembered() async {
        model.choose(server: "Uni-Hamburg.de")
        await model.run()
        XCTAssertEqual(session.directoryRequests.last?.server, "uni-hamburg.de")
        XCTAssertEqual(Set(session.browsedServers), ["uni-hamburg.de"])
        XCTAssertEqual(model.servers.first, "uni-hamburg.de")
        XCTAssertEqual(model.message, "Nothing found.")
        model.choose(server: "example.org")
        XCTAssertNil(model.server)
    }

    func testNotAServer() async {
        session.directory = .failure(MtrxError.InvalidInput(reason: "bad"))
        model.choose(server: "no such")
        await model.run()
        XCTAssertEqual(model.message, "“no such” is not a server name.")
    }

    func testAPastedAddressIsLookedUp() async {
        session.previewResult = .success(Make.directoryRoom("!ice", name: "Ice"))
        model.query = "https://matrix.to/#/#ice:example.org"
        await model.run()
        XCTAssertEqual(model.address?.room, "#ice:example.org")
        XCTAssertEqual(model.preview?.name, "Ice")
        XCTAssertTrue(session.directoryRequests.isEmpty)
    }
}

@MainActor
final class NewRoomModelTests: XCTestCase {
    private var session: FakeSession!

    override func setUp() async throws {
        session = FakeSession(noHandle: .init())
    }

    func testMakesARoomInASpaceWithCoOwners() async {
        session.offersOwnersResult = true
        let model = NewRoomModel(session: session, space: false, parent: (id: "!team", name: "Team"))
        await model.load()
        XCTAssertEqual(model.title, "New room in Team")
        XCTAssertFalse(model.canCreate)
        model.name = " Lab "
        model.person = "@ann:example.org"
        model.addPerson()
        model.person = "@bob:example.org"
        model.addPerson()
        model.person = "not an id"
        XCTAssertFalse(model.personValid)
        model.setCoOwner("@bob:example.org", true)
        let id = await model.create()
        XCTAssertEqual(id, "!new:example.org")
        XCTAssertEqual(
            session.created,
            [
                NewRoom(
                    name: "Lab", topic: "", space: false, encrypted: true, moderated: false,
                    invite: ["@ann:example.org", "@bob:example.org"], coOwners: ["@bob:example.org"],
                    parent: "!team")
            ])
    }

    func testNoCoOwnersWhereNotOffered() async {
        let model = NewRoomModel(session: session, space: true)
        await model.load()
        XCTAssertEqual(model.title, "New space")
        model.name = "Team"
        model.person = "@ann:example.org"
        model.addPerson()
        model.setCoOwner("@ann:example.org", true)
        _ = await model.create()
        XCTAssertEqual(session.created.first?.coOwners, [])
        XCTAssertEqual(session.created.first?.encrypted, false)
        XCTAssertEqual(session.created.first?.space, true)
    }

    func testAFailureSaysWhy() async {
        session.changeError = MtrxError.Other(reason: "Too many rooms")
        let model = NewRoomModel(session: session, space: false)
        model.name = "Lab"
        let id = await model.create()
        XCTAssertNil(id)
        XCTAssertEqual(model.error, "Could not create the room “Lab”: Too many rooms")
    }

    func testUserIds() {
        XCTAssertTrue(NewRoomModel.isUserId("@a:b.org"))
        XCTAssertFalse(NewRoomModel.isUserId("@:b.org"))
        XCTAssertFalse(NewRoomModel.isUserId("a:b.org"))
        XCTAssertFalse(NewRoomModel.isUserId("@a:"))
    }
}
