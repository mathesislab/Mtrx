import MtrxFFI
import XCTest
@testable import Mtrx

/// A room's settings, as the library gives them, and changing them and
/// the members.
@MainActor
final class RoomInfoModelTests: XCTestCase {
    private var session: FakeSession!
    private var model: RoomInfoModel!

    override func setUp() async throws {
        session = FakeSession(noHandle: .init())
        session.memberList = [Make.member("me", level: 100)]
        model = RoomInfoModel(session: session, roomId: "!room:example.org", userId: "@me:example.org")
        await model.load()
        await eventually { self.model.settings != nil }
    }

    func testReadsTheSettingsAndMembers() {
        XCTAssertEqual(model.settings, Make.settings())
        XCTAssertEqual(model.members?.map(\.user.userId), ["@me:example.org"])
    }

    func testAChangeShowsAsTheLibraryGivesIt() async {
        await model.setNotify(.mentions).value
        XCTAssertEqual(session.changes, ["notify mentions"])
        await eventually { self.model.settings?.notify == .mentions }
    }

    func testAChangeMadeElsewhereShows() async {
        session.settings = Make.settings(joinRule: .public)
        await eventually { self.model.settings?.joinRule == .public }
    }

    func testAFailedChangeSaysWhy() async {
        session.changeError = MtrxError.Other(reason: "Not allowed")
        await model.setJoinRule(.public).value
        XCTAssertEqual(model.settings?.joinRule, .invite)
        XCTAssertEqual(model.error, "Not allowed")
    }

    /// The library's room list shows it at once.
    func testATopicIsSaved() async {
        var saved: Bool?
        await model.setTopic("New") { saved = $0 }.value
        XCTAssertEqual(saved, true)
        XCTAssertEqual(session.changes, ["topic New"])
    }

    func testAnUnsavedTopicSaysWhy() async {
        session.changeError = MtrxError.Other(reason: "Not allowed")
        var saved: Bool?
        await model.setTopic("New") { saved = $0 }.value
        XCTAssertEqual(saved, false)
        XCTAssertEqual(model.error, "Not allowed")
    }

    func testSpacesChange() async {
        session.settings = Make.settings(
            spaces: [SpaceLink(spaceId: "!a", suggested: false, canChange: true)], canAddTo: ["!b"])
        await model.addToSpace("!b").value
        await eventually { self.model.settings?.spaces.map(\.spaceId) == ["!a", "!b"] }
        XCTAssertEqual(model.settings?.canAddTo, [])
        await model.setSuggested(in: "!a", true).value
        await model.removeFromSpace("!a").value
        XCTAssertEqual(session.changes, ["add to !b", "suggested !a true", "remove from !a"])
        await eventually { self.model.settings?.spaces.map(\.spaceId) == ["!b"] }
    }

    func testMembersChange() async {
        await model.setLevel(50, of: "@bob:example.org").value
        await model.remove("@bob:example.org", reason: "  ", ban: false).value
        await model.remove("@cy:example.org", reason: "Spam", ban: true).value
        var invited: Bool?
        await model.invite(" @dee:example.org ") { invited = $0 }.value
        XCTAssertEqual(invited, true)
        XCTAssertEqual(
            session.changes,
            ["level @bob:example.org 50", "remove @bob:example.org -", "ban @cy:example.org Spam"])
        XCTAssertEqual(session.invites.map(\.userId), ["@dee:example.org"])
    }

    func testFacts() {
        let room = Make.room("!r", members: 4)
        XCTAssertEqual(
            RoomFacts.line(room, Make.settings(joinRule: .public)),
            "Room · Public: anyone can join · 4 members · encrypted · room version 12")
        XCTAssertEqual(
            RoomFacts.line(Make.room("!s", kind: .space, members: 1), Make.settings(joinRule: .knock)),
            "Space · Private: people ask to join · 1 member · room version 12")
    }
}

/// Editing a topic: the library composes what the editor typed.
final class TopicDraftTests: XCTestCase {
    private let topic = Make.topic(
        "Ice group",
        calls: [Make.link("https://meet.example.org/ice", title: "Weekly", passcode: "42")],
        links: [Make.link("https://example.org/drive", name: "example.org")])

    func testAnEditedTopicRoundTrips() {
        let draft = TopicDraft(topic)
        XCTAssertEqual(draft.calls.map(\.passcode), ["42"])
        XCTAssertEqual(draft.links.map(\.title), [""])
        XCTAssertEqual(
            draft.topic,
            "Ice group\n[call Weekly passcode 42] https://meet.example.org/ice\n[link] https://example.org/drive")
    }

    func testAnAddressThatIsNotAWebAddressIsMarkedAndLeftOut() {
        var draft = TopicDraft(topic)
        draft.add(call: false)
        XCTAssertEqual(draft.dropped, 0, "nothing typed in yet")
        draft.links[1].url = "mailto:ann@example.org"
        XCTAssertFalse(draft.links[1].valid)
        XCTAssertEqual(draft.dropped, 1)
        draft.links[0].url = "example.org/papers"
        XCTAssertTrue(draft.links[0].valid, "https:// may be left out")
        XCTAssertEqual(
            draft.topic,
            "Ice group\n[call Weekly passcode 42] https://meet.example.org/ice\n[link] https://example.org/papers")
        draft.remove(call: false, at: [1])
        XCTAssertEqual(draft.dropped, 0)
    }

    func testAMovedCallBecomesTheRoomsCall() {
        var draft = TopicDraft(topic)
        draft.add(call: true)
        draft.calls[1].url = "zoom.us/j/1"
        draft.move(call: true, from: [1], to: 0)
        XCTAssertEqual(
            draft.topic,
            "Ice group\n[call] https://zoom.us/j/1\n[call Weekly passcode 42] https://meet.example.org/ice\n"
                + "[link] https://example.org/drive")
    }
}

/// The room header: its topic line, and Join for the room's call.
final class RoomHeaderTests: XCTestCase {
    func testTheLineIsThePreviewElseAddATopic() {
        XCTAssertEqual(RoomHeader.line(Make.topic("About.\nMore"), canEdit: false), "About.")
        XCTAssertEqual(RoomHeader.line(.none, canEdit: true), "Add a topic")
        XCTAssertNil(RoomHeader.line(.none, canEdit: false))
    }

    func testJoinOnlyWithACall() throws {
        XCTAssertNil(HeaderLinks(Make.topic("About."), canEdit: true), "nothing to offer")

        let drive = Make.link("https://example.org/drive", name: "example.org")
        let links = try XCTUnwrap(HeaderLinks(Make.topic(links: [drive]), canEdit: false))
        XCTAssertNil(links.join, "links only: the menu, no Join")
        XCTAssertEqual(links.groups.map(\.title), ["Links"])
        XCTAssertEqual(links.canEdit, false)

        let weekly = Make.link("https://meet.example.org/ice", title: "Weekly", passcode: "42")
        let daily = Make.link("https://zoom.us/j/1", name: "Zoom")
        let header = try XCTUnwrap(HeaderLinks(Make.topic(calls: [weekly, daily], links: [drive]), canEdit: true))
        XCTAssertEqual(header.join, weekly, "the first call is the room's")
        XCTAssertEqual(header.joinLabel, "Join Weekly, passcode 42")
        XCTAssertEqual(header.groups.map(\.title), ["Calls", "Links"], "calls first")
        XCTAssertEqual(header.groups.map(\.links), [[weekly, daily], [drive]])
        XCTAssertEqual(header.canEdit, true, "Edit links… last")
    }
}
