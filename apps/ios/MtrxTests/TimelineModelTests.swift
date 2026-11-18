import MtrxFFI
import XCTest
@testable import Mtrx

@MainActor
final class TimelineModelTests: XCTestCase {
    private var session: FakeSession!
    private var model: TimelineModel!

    override func setUp() async throws {
        session = FakeSession(noHandle: .init())
        model = TimelineModel(session: session, room: Make.room("!room:example.org"))
        await model.open()
    }

    func testShowsTheListenersItems() async {
        XCTAssertFalse(model.opening)
        session.fakeTimeline.listener?.onUpdate(rows: [Make.row("1", Make.event("alice", Make.text("Hi")))])
        await eventually { self.model.rows.map(\.id) == ["1"] }
    }

    func testMarksReadWhenVisible() async {
        model.setVisible(true)
        session.fakeTimeline.listener?.onUpdate(rows: [Make.row("1", Make.event("alice", Make.text("Hi")))])
        await eventually { self.session.fakeTimeline.markedRead == 1 }
    }

    func testSendsTheDraft() async {
        model.draft = "  Hello  "
        await model.send()?.value
        XCTAssertEqual(session.fakeTimeline.sent, ["Hello"])
        XCTAssertEqual(model.draft, "")
    }

    func testKeepsTheDraftWhenSendingFails() async {
        session.fakeTimeline.sendError = MtrxError.Other(reason: "No connection")
        model.draft = "Hello"
        await model.send()?.value
        XCTAssertEqual(model.draft, "Hello")
        XCTAssertEqual(model.error, "No connection")
    }

    // MARK: Files

    /// A file in the temporary folder, as Files would hand it over.
    private func pickedFile(_ name: String) throws -> URL {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let url = folder.appendingPathComponent(name)
        try Data("Ice core notes".utf8).write(to: url)
        return url
    }

    func testSendsPickedFilesAloneOrWithTheTextAndThenDeletesTheirCopies() async throws {
        XCTAssertFalse(model.canSend)
        await model.attach([.file(try pickedFile("notes.txt"))]).value
        let copy = try XCTUnwrap(model.picked.first?.file)
        XCTAssertEqual(copy.filename, "notes.txt")
        XCTAssertTrue(copy.path.hasPrefix(Outgoing.folder.path))
        XCTAssertTrue(model.canSend)
        await model.send()?.value
        XCTAssertEqual(session.fakeTimeline.sent, [""])
        XCTAssertEqual(session.fakeTimeline.files, [["notes.txt"]])
        XCTAssertEqual(model.picked, [])
        XCTAssertFalse(FileManager.default.fileExists(atPath: copy.path))

        // With text, as a reply: the text answers, the files follow it.
        await show([bobsMessage])
        model.startReply(to: "b")
        await model.attach([.file(try pickedFile("a.pdf")), .file(try pickedFile("b.pdf"))]).value
        XCTAssertEqual(model.picked.compactMap(\.file?.filename), ["a.pdf", "b.pdf"])
        model.draft = "Here"
        await model.send()?.value
        XCTAssertEqual(session.fakeTimeline.replies.map(\.itemId), ["b"])
        XCTAssertEqual(session.fakeTimeline.replies.map(\.text), ["Here"])
        XCTAssertEqual(session.fakeTimeline.files.last, ["a.pdf", "b.pdf"])
    }

    func testAFileTakenAwayIsNotSentAndFilesComeBackWhenSendingFails() async throws {
        await model.attach([.file(try pickedFile("a.pdf")), .file(try pickedFile("b.pdf"))]).value
        let taken = try XCTUnwrap(model.picked.first)
        model.detach(taken.id)
        XCTAssertEqual(model.picked.compactMap(\.file?.filename), ["b.pdf"])
        XCTAssertFalse(FileManager.default.fileExists(atPath: try XCTUnwrap(taken.file?.path)))

        session.fakeTimeline.sendError = MtrxError.Other(reason: "a.pdf is too large")
        await model.send()?.value
        XCTAssertEqual(model.picked.compactMap(\.file?.filename), ["b.pdf"])
        XCTAssertEqual(model.error, "a.pdf is too large")
        XCTAssertTrue(FileManager.default.fileExists(atPath: try XCTUnwrap(model.picked.first?.file?.path)))
    }

    func testAnEditSendsNoFiles() async throws {
        await show([ourMessage])
        await model.attach([.file(try pickedFile("a.pdf"))]).value
        model.startEdit("o")
        model.draft = "Hello"
        await model.send()?.value
        XCTAssertEqual(session.fakeTimeline.edits.map(\.text), ["Hello"])
        XCTAssertEqual(session.fakeTimeline.files, [])
        XCTAssertEqual(model.picked.count, 1)
    }

    func testPhotosAndVideosAreNamedFromTheDate() throws {
        let date = try XCTUnwrap(
            Calendar.current.date(from: DateComponents(year: 2026, month: 11, day: 18, hour: 14, minute: 3)))
        XCTAssertEqual(Outgoing.name(.photo, at: date, extension: "JPEG"), "Photo 2026-11-18 14.03.jpg")
        XCTAssertEqual(Outgoing.name(.video, at: date, extension: "mp4"), "Video 2026-11-18 14.03.mp4")
        XCTAssertEqual(Outgoing.name(.video, at: date, extension: ""), "Video 2026-11-18 14.03.mp4")
    }

    func testUploadsAreFollowedByRow() async {
        session.fakeTimeline.uploadsListener?.onUpdate(uploads: [Upload(itemId: "f", fraction: 0.25)])
        await eventually { self.model.uploads.fraction(of: "f") == 0.25 }
        XCTAssertNil(model.uploads.fraction(of: "g"))
        session.fakeTimeline.uploadsListener?.onUpdate(uploads: [])
        await eventually { self.model.uploads.fraction(of: "f") == nil }
    }

    func testLoadsOlderWhileTheTopIsVisible() async {
        session.fakeTimeline.pages = [false, false, true]
        await model.setTopVisible(true)?.value
        XCTAssertEqual(session.fakeTimeline.paginations, 3)
        XCTAssertTrue(model.reachedStart)
        XCTAssertFalse(model.loadingOlder)
        // At the start: nothing more to load.
        await model.loadOlder()?.value
        XCTAssertEqual(session.fakeTimeline.paginations, 3)
    }

    func testTheButtonToTheBottomShowsWhenScrolledUp() async {
        XCTAssertTrue(model.atBottom)
        XCTAssertFalse(model.showsScrollToBottom)
        // A little up, or pulled past the bottom: still at the bottom.
        model.scrolled(distanceFromBottom: TimelineModel.bottomTolerance)
        XCTAssertFalse(model.showsScrollToBottom)
        model.scrolled(distanceFromBottom: -30)
        XCTAssertFalse(model.showsScrollToBottom)
        model.scrolled(distanceFromBottom: 300)
        XCTAssertFalse(model.atBottom)
        XCTAssertTrue(model.showsScrollToBottom)
        model.scrolled(distanceFromBottom: 0)
        XCTAssertFalse(model.showsScrollToBottom)
    }

    func testNoButtonWhileOpening() async {
        let opening = TimelineModel(session: session, room: Make.room("!other:example.org"))
        opening.scrolled(distanceFromBottom: 300)
        XCTAssertFalse(opening.showsScrollToBottom)
        await opening.open()
        XCTAssertTrue(opening.showsScrollToBottom)
    }

    // MARK: Replying and editing

    private var timeline: FakeTimeline { session.fakeTimeline }

    /// Show `items` and wait until the model has them.
    private func show(_ items: [TimelineRow]) async {
        timeline.listener?.onUpdate(rows: items)
        await eventually { self.model.rows.count == items.count }
    }

    private let bobsMessage = Make.row("b", Make.event("bob", Make.text("Lunch?\nAt noon"), eventId: "$b"))
    private let ourMessage = Make.row(
        "o", Make.event("me", Make.text("Hello world", rawBody: "Hello *world*"), own: true, eventId: "$o", editable: true))

    func testRepliesWithTheDraft() async {
        await show([bobsMessage])
        model.startReply(to: "b")
        XCTAssertEqual(model.compose, .reply(ComposeTarget(itemId: "b", eventId: "$b", sender: "Bob", preview: "Lunch?")))
        model.draft = "Sure"
        await model.send()?.value
        XCTAssertEqual(timeline.replies.map(\.itemId), ["b"])
        XCTAssertEqual(timeline.replies.map(\.text), ["Sure"])
        XCTAssertEqual(timeline.sent, [])
        XCTAssertNil(model.compose)
        XCTAssertEqual(model.draft, "")
    }

    func testEditsFromTheTextAsWritten() async {
        await show([ourMessage])
        model.draft = "Half written"
        model.startEdit("o")
        XCTAssertEqual(model.compose?.target.itemId, "o")
        XCTAssertEqual(model.draft, "Hello *world*")
        model.draft = "Hello *everyone*"
        await model.send()?.value
        XCTAssertEqual(timeline.edits.map(\.itemId), ["o"])
        XCTAssertEqual(timeline.edits.map(\.text), ["Hello *everyone*"])
        XCTAssertNil(model.compose)
        // The draft from before the edit is back.
        XCTAssertEqual(model.draft, "Half written")
    }

    func testCancellingAnEditBringsBackTheDraft() async {
        await show([ourMessage])
        model.draft = "Half written"
        model.startEdit("o")
        model.cancelCompose()
        XCTAssertNil(model.compose)
        XCTAssertEqual(model.draft, "Half written")
    }

    func testReplyingAndEditingExcludeEachOther() async {
        await show([bobsMessage, ourMessage])
        model.draft = "Draft"
        model.startEdit("o")
        model.startReply(to: "b")
        XCTAssertEqual(model.compose?.target.itemId, "b")
        if case .edit = model.compose { XCTFail("still editing") }
        XCTAssertEqual(model.draft, "Draft")
        model.startEdit("o")
        XCTAssertEqual(model.compose, .edit(ComposeTarget(itemId: "o", eventId: "$o", sender: "Me", preview: "Hello world")))
    }

    func testAFailedReplyKeepsReplying() async {
        await show([bobsMessage])
        timeline.sendError = MtrxError.InvalidInput(reason: "no message `b` in the timeline")
        model.startReply(to: "b")
        model.draft = "Sure"
        await model.send()?.value
        XCTAssertEqual(model.compose?.target.itemId, "b")
        XCTAssertEqual(model.draft, "Sure")
        XCTAssertEqual(model.error, "no message `b` in the timeline")
    }

    // MARK: Disappearing

    func testWhatIsSentFollowsTheRoomsModeUntilOneIsChosen() async {
        XCTAssertNil(model.disappearAfterRead)
        session.disappearingMode = 300
        await eventually { self.model.disappearAfterRead == 300 }
        model.draft = "Gone soon"
        await model.send()?.value
        model.chooseDisappear(nil)
        XCTAssertNil(model.disappearAfterRead)
        // Changed on another device: the choice holds.
        session.disappearingMode = 60
        try? await Task.sleep(nanoseconds: 50_000_000)
        XCTAssertNil(model.disappearAfterRead)
        model.draft = "Kept"
        await model.send()?.value
        XCTAssertEqual(timeline.sent, ["Gone soon", "Kept"])
        XCTAssertEqual(timeline.disappearing, [300, nil])
    }

    func testARoomOpenedAgainStartsAtItsMode() async {
        session.disappearingMode = 300
        model.chooseDisappear(3_600)
        let again = TimelineModel(session: session, room: Make.room("!room:example.org"))
        await again.open()
        await eventually { again.disappearAfterRead == 300 }
    }

    func testAReplyDisappearsAsChosenAndAnEditTakesNoChoice() async {
        await show([bobsMessage, ourMessage])
        model.chooseDisappear(3_600)
        model.startReply(to: "b")
        model.draft = "Sure"
        await model.send()?.value
        model.startEdit("o")
        model.draft = "Hello *everyone*"
        await model.send()?.value
        XCTAssertEqual(timeline.replies.map(\.text), ["Sure"])
        XCTAssertEqual(timeline.edits.map(\.text), ["Hello *everyone*"])
        XCTAssertEqual(timeline.disappearing, [3_600])
    }

    func testTheHourglassSaysWhatIsSent() {
        XCTAssertEqual(disappearWords(afterRead: nil), "Disappearing messages: off")
        XCTAssertEqual(disappearWords(afterRead: 300), "Disappears 5 minutes after everyone has read it")
        XCTAssertEqual(Disappearing.choices.first?.label, "Off")
        XCTAssertNil(Disappearing.choices.first?.afterRead)
    }

    // MARK: Typing

    func testTellsWhetherWeType() async {
        model.draft = "H"
        model.draft = "Hi"
        model.draft = ""
        await eventually { self.timeline.typing == [true, true, false] }
        model.draft = "Hello"
        await model.send()?.value
        await eventually { self.timeline.typing.last == false && self.timeline.typing.count == 5 }
    }

    func testShowsWhoTypesWhileVisible() async {
        XCTAssertNil(timeline.typingListener)
        model.setVisible(true)
        timeline.typingListener?.onUpdate(names: ["Anna", "Ben"])
        await eventually { self.model.typing == ["Anna", "Ben"] }
        model.setVisible(false)
        XCTAssertEqual(timeline.typingSubscription?.cancelled, true)
        XCTAssertEqual(model.typing, [])
        await eventually { self.timeline.typing == [false] }
    }

    func testTypingLine() {
        XCTAssertNil(TimelineModel.typingLine([]))
        XCTAssertEqual(TimelineModel.typingLine(["Anna"]), "Anna is typing…")
        XCTAssertEqual(TimelineModel.typingLine(["Anna", "Ben"]), "Anna and Ben are typing…")
        XCTAssertEqual(TimelineModel.typingLine(["Anna", "Ben", "Cleo"]), "Anna, Ben and Cleo are typing…")
        XCTAssertEqual(TimelineModel.typingLine(["Anna", "Ben", "Cleo", "Dan"]), "Anna, Ben and 2 others are typing…")
    }

    // MARK: Acting on messages

    func testActionsTakeTheRowsId() async {
        await show([bobsMessage, ourMessage])
        await model.react("b", key: "👍")?.value
        await model.delete("o")?.value
        await model.retry("o")?.value
        await model.unsend("o")?.value
        XCTAssertEqual(timeline.reactions.map(\.itemId), ["b"])
        XCTAssertEqual(timeline.reactions.map(\.key), ["👍"])
        XCTAssertEqual(timeline.deleted, ["o"])
        XCTAssertEqual(timeline.retried, ["o"])
        XCTAssertEqual(timeline.unsent, ["o"])
        XCTAssertNil(model.error)
    }

    func testAMessageTakenBackIsInAnEmptyComposerWithItsReply() async {
        await show([bobsMessage])
        timeline.found = ["$b": "b"]
        timeline.unsentDraft = Draft(text: "**Sure**", replyTo: "$b")
        await model.unsend("u")?.value
        XCTAssertEqual(model.draft, "**Sure**")
        await eventually { self.model.compose?.target.itemId == "b" }
    }

    func testAMessageTakenBackFollowsWhatIsWritten() async {
        await show([bobsMessage])
        timeline.found = ["$b": "b"]
        model.draft = "Half written  \n"
        timeline.unsentDraft = Draft(text: "Sure", replyTo: "$b")
        await model.unsend("u")?.value
        XCTAssertEqual(model.draft, "Half written\n\nSure")
        try? await Task.sleep(nanoseconds: 50_000_000)
        XCTAssertNil(model.compose)
        // A file: nothing comes back.
        timeline.unsentDraft = nil
        await model.unsend("f")?.value
        XCTAssertEqual(model.draft, "Half written\n\nSure")
    }

    func testAFailedActionShowsWhy() async {
        timeline.actionError = MtrxError.InvalidInput(reason: "no message `gone` in the timeline")
        await model.react("gone", key: "👍")?.value
        XCTAssertEqual(model.error, "no message `gone` in the timeline")
    }

    // MARK: Going to a quoted message

    private let start = TimelineRow(id: "s", content: .start)
    private let reply = Make.row(
        "r",
        Make.event(
            "bob",
            Make.text(
                "Yes",
                replyTo: ReplyQuote(
                    eventId: "$o", state: .loaded, sender: Make.user("me"), kind: .text, body: "Hello world")),
            at: 5, eventId: "$r"))

    func testGoesToAQuotedMessageShown() async {
        await show([ourMessage, reply])
        await model.goTo(eventId: "$o")?.value
        XCTAssertEqual(model.scrollTarget?.rowId, "o")
        XCTAssertEqual(model.highlighted, "o")
        XCTAssertEqual(timeline.seeks, [])
        XCTAssertFalse(model.seeking)
    }

    func testTheLibraryLoadsOlderMessagesUntilTheQuotedOneIsThere() async {
        await show([reply])
        timeline.found = ["$o": "o"]
        timeline.onSeek = { [timeline, ourMessage, reply] _ in
            timeline.listener?.onUpdate(rows: [ourMessage, reply])
        }
        await model.goTo(eventId: "$o")?.value
        await eventually { self.model.scrollTarget?.rowId == "o" }
        XCTAssertEqual(model.highlighted, "o")
        XCTAssertEqual(timeline.seeks, ["$o"])
        XCTAssertFalse(model.seeking)
        XCTAssertNil(model.error)
    }

    func testScrollsOnceTheRowsHaveTheMessageFound() async {
        await show([reply])
        timeline.found = ["$o": "o"]
        await model.goTo(eventId: "$o")?.value
        XCTAssertNil(model.scrollTarget)
        timeline.listener?.onUpdate(rows: [ourMessage, reply])
        await eventually { self.model.scrollTarget?.rowId == "o" }
        XCTAssertEqual(model.highlighted, "o")
    }

    func testQuotedMessageNotFound() async {
        await show([start, reply])
        await model.goTo(eventId: "$o")?.value
        XCTAssertEqual(model.error, "The original message was not found in the room's history")
        XCTAssertFalse(model.seeking)
        XCTAssertNil(model.scrollTarget)
    }
}
