import MtrxFFI
import XCTest
@testable import Mtrx

@MainActor
final class PersonInfoModelTests: XCTestCase {
    private var session: FakeSession!
    private var copied: [String] = []
    private let chat = Make.room(
        "!bob", kind: .direct, name: "Bob", directUser: "@bob:example.org", avatarUrl: "mxc://example.org/chat")
    private let team = Make.room("!team", kind: .space, name: "Team")

    override func setUp() async throws {
        session = FakeSession(noHandle: .init())
        copied = []
    }

    private func makeModel() -> PersonInfoModel {
        PersonInfoModel(session: session, userId: "@bob:example.org", room: chat) { [unowned self] in
            self.copied.append($0)
        }
    }

    func testTheProfileReplacesTheChatsNameAndPicture() async {
        session.profile = .success(
            UserProfile(userId: "@bob:example.org", name: "Bob Builder", avatarUrl: "mxc://example.org/bob"))
        let model = makeModel()
        // The chat's at once.
        XCTAssertEqual(model.name, "Bob")
        XCTAssertEqual(model.avatarUrl, "mxc://example.org/chat")
        model.load()
        await eventually { model.name == "Bob Builder" }
        XCTAssertEqual(model.avatarUrl, "mxc://example.org/bob")
        XCTAssertEqual(session.profileRequests, ["@bob:example.org"])
    }

    func testWhatTheProfileLacksStaysTheChats() async {
        session.profile = .success(UserProfile(userId: "@bob:example.org", name: nil, avatarUrl: "mxc://example.org/bob"))
        let model = makeModel()
        model.load()
        await eventually { model.avatarUrl == "mxc://example.org/bob" }
        XCTAssertEqual(model.name, "Bob")
    }

    func testAFailedProfileKeepsTheChatsWithoutAMessage() async {
        session.profile = .failure(MtrxError.Other(reason: "No connection"))
        let model = makeModel()
        model.load()
        await eventually { self.session.profileRequests.count == 1 }
        try? await Task.sleep(nanoseconds: 50_000_000)
        XCTAssertEqual(model.name, "Bob")
        XCTAssertEqual(model.avatarUrl, "mxc://example.org/chat")
        XCTAssertNil(model.inviteError)
    }

    func testCopiesTheUserId() {
        let model = makeModel()
        XCTAssertFalse(model.copied)
        model.copyId()
        XCTAssertEqual(copied, ["@bob:example.org"])
        XCTAssertTrue(model.copied)
    }

    func testInvitingToASpaceClosesTheListAndSaysSo() async {
        let model = makeModel()
        model.choosingSpace = true
        model.invite(to: team)
        XCTAssertEqual(model.inviting, "!team")
        await eventually { model.inviting == nil }
        XCTAssertEqual(session.invites.map(\.roomId), ["!team"])
        XCTAssertEqual(session.invites.map(\.userId), ["@bob:example.org"])
        XCTAssertFalse(model.choosingSpace)
        XCTAssertEqual(model.confirmation, "Invited to Team.")
        XCTAssertNil(model.inviteError)
    }

    func testAFailedInviteSaysWhyAndKeepsTheList() async {
        session.inviteError = MtrxError.Other(reason: "bob is already in the room")
        let model = makeModel()
        model.choosingSpace = true
        model.invite(to: team)
        await eventually { model.inviting == nil }
        XCTAssertEqual(model.inviteError, "bob is already in the room")
        XCTAssertTrue(model.choosingSpace)
        XCTAssertNil(model.confirmation)
        XCTAssertTrue(session.invites.isEmpty)

        // The next attempt clears it.
        session.inviteError = nil
        model.invite(to: team)
        XCTAssertNil(model.inviteError)
        await eventually { model.inviting == nil }
        XCTAssertEqual(model.confirmation, "Invited to Team.")
    }

    func testOneInviteAtATime() async {
        let model = makeModel()
        model.invite(to: team)
        model.invite(to: Make.room("!other", kind: .space, name: "Other"))
        await eventually { model.inviting == nil }
        XCTAssertEqual(session.invites.map(\.roomId), ["!team"])
    }

    func testADirectChatsHeaderOpensThePersonsSheet() async {
        let timeline = TimelineModel(session: session, room: chat)
        XCTAssertEqual(timeline.person, "@bob:example.org")
        session.profile = .success(UserProfile(userId: "@bob:example.org", name: "Bob Builder", avatarUrl: nil))
        timeline.showPersonInfo()
        XCTAssertEqual(timeline.personInfo?.userId, "@bob:example.org")
        // The profile is asked for as it opens.
        await eventually { timeline.personInfo?.name == "Bob Builder" }
    }

    func testNoPersonInTheNotesToSelfOrARoom() {
        let notes = Make.room("!notes", kind: .direct, name: "Notes", directUser: "@me:example.org")
        for room in [notes, Make.room("!lab", name: "Lab")] {
            let timeline = TimelineModel(session: session, room: room)
            XCTAssertNil(timeline.person)
            timeline.showPersonInfo()
            XCTAssertNil(timeline.personInfo)
        }
    }
}
