import MtrxFFI
import XCTest
@testable import Mtrx

/// A room's settings: shown changed at once, read again after.
@MainActor
final class RoomInfoModelTests: XCTestCase {
    private var session: FakeSession!
    private var model: RoomInfoModel!

    override func setUp() async throws {
        session = FakeSession(noHandle: .init())
        session.memberList = [Make.member("me", level: 100)]
        model = RoomInfoModel(session: session, roomId: "!room:example.org", userId: "@me:example.org")
        await model.load()
    }

    func testReadsTheSettingsAndMembers() {
        XCTAssertEqual(model.settings, Make.settings())
        XCTAssertEqual(model.members?.map(\.userId), ["@me:example.org"])
    }

    func testAChangeShowsAtOnceThenAsTheServerHasIt() async {
        let task = model.setNotify(.mentions)
        XCTAssertEqual(model.settings?.notify, .mentions)
        await task.value
        XCTAssertEqual(session.changes, ["notify mentions"])
        XCTAssertEqual(model.settings?.notify, .mentions)
    }

    func testAFailedChangeIsTakenBackWithWhy() async {
        session.changeError = MtrxError.Other(reason: "Not allowed")
        await model.setAccess(.public).value
        XCTAssertEqual(model.settings?.access, .invite)
        XCTAssertEqual(model.error, "Not allowed")
    }

    func testASavedTopicShowsUntilTheListHasIt() async {
        let saved = await model.setTopic("New")
        XCTAssertTrue(saved)
        XCTAssertEqual(session.changes, ["topic New"])
        XCTAssertEqual(model.shownTopic("Old"), "New")
        model.topicListed("New")
        XCTAssertEqual(model.shownTopic("New"), "New")
        XCTAssertEqual(model.shownTopic(nil), "")
    }

    func testAnUnsavedTopicIsNotShown() async {
        session.changeError = MtrxError.Other(reason: "Not allowed")
        let saved = await model.setTopic("New")
        XCTAssertFalse(saved)
        XCTAssertEqual(model.shownTopic("Old"), "Old")
    }

    func testSpacesChange() async {
        session.settings = Make.settings(
            spaces: [SpaceLink(spaceId: "!a", suggested: false, canChange: true)], canAddTo: ["!b"])
        await model.loadSettings()
        let adding = model.addToSpace("!b")
        XCTAssertEqual(model.settings?.spaces.map(\.spaceId), ["!a", "!b"])
        XCTAssertEqual(model.settings?.canAddTo, [])
        await adding.value
        await model.setSuggested(in: "!a", true).value
        await model.removeFromSpace("!a").value
        XCTAssertEqual(session.changes, ["add to !b", "suggested !a true", "remove from !a"])
    }

    func testMembersChange() async {
        await model.setLevel(50, of: "@bob:example.org").value
        await model.remove("@bob:example.org", reason: "  ", ban: false).value
        await model.remove("@cy:example.org", reason: "Spam", ban: true).value
        let invited = await model.invite(" @dee:example.org ")
        XCTAssertTrue(invited)
        XCTAssertEqual(
            session.changes,
            ["level @bob:example.org 50", "remove @bob:example.org -", "ban @cy:example.org Spam"])
        XCTAssertEqual(session.invites.map(\.userId), ["@dee:example.org"])
    }

    func testFacts() {
        let room = Make.room("!r", members: 4)
        XCTAssertEqual(
            RoomFacts.line(room, Make.settings(access: .public)),
            "Room · Public: anyone can join · 4 members · encrypted · room version 12")
        XCTAssertEqual(
            RoomFacts.line(Make.room("!s", kind: .space, members: 1), Make.settings(access: .knock)),
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
