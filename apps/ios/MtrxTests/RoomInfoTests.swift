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

/// The topic's preview line and editing it.
final class TopicsTests: XCTestCase {
    func testPreviewsTheTextElseTheLinks() {
        let text = parseTopic(topic: "About.\nMore")
        XCTAssertEqual(Topics.preview(text, canEdit: false), "About.")
        let links = parseTopic(topic: "[call Weekly] https://zoom.us/j/1\n[link] https://overleaf.com/a")
        XCTAssertEqual(Topics.preview(links, canEdit: false), "Weekly · overleaf.com")
        XCTAssertEqual(Topics.preview(parseTopic(topic: ""), canEdit: true), "Add a topic")
        XCTAssertNil(Topics.preview(parseTopic(topic: ""), canEdit: false))
    }

    func testAnEditedTopicRoundTrips() {
        let topic = "Ice group\n[call Weekly passcode 42] https://meet.example.org/ice\n[link] https://example.org/drive"
        var draft = TopicDraft(parseTopic(topic: topic))
        XCTAssertEqual(draft.calls.map(\.passcode), ["42"])
        XCTAssertEqual(parseTopic(topic: draft.topic), parseTopic(topic: topic))
        draft.add(call: false)
        draft.links[2].url = "not a link"
        XCTAssertEqual(draft.dropped, 1)
        XCTAssertEqual(parseTopic(topic: draft.topic).links.count, 2)
        draft.remove(draft.links[0].id)
        XCTAssertEqual(parseTopic(topic: draft.topic).links.map(\.call), [false])
    }

    func testFirstLine() {
        XCTAssertEqual(Topics.firstLine("\n  Sea ice  \nmore"), "Sea ice")
        XCTAssertNil(Topics.firstLine(nil))
    }
}
