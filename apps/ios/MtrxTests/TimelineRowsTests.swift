import MtrxFFI
import XCTest
@testable import Mtrx

final class TimelineRowsTests: XCTestCase {
    func testUndecryptableMessagesSayDecryptingWhileKeysArrive() {
        let undecryptable = Make.event("alice", .unableToDecrypt, eventId: "$1")
        XCTAssertEqual(messageRow(undecryptable)?.undecryptableText, "Could not decrypt this message.")
        XCTAssertEqual(messageRow(undecryptable, decrypting: true)?.undecryptableText, "Decrypting…")
        XCTAssertNil(messageRow(Make.event("alice", Make.text("Hi"), eventId: "$2"))?.undecryptableText)
    }

    private func messageRow(
        _ event: TimelineEvent, decrypting: Bool = false, actions: [MessageAction]? = nil
    ) -> MessageRow? {
        Make.row("m", event, decrypting: decrypting, actions: actions).messageRow
    }

    func testRowCarriesReactionsQuoteEditedAndReaders() {
        let reactions = [
            Reaction(key: "👍", count: 2, byUs: true, senders: [Make.reader("me"), Make.reader("bob")]),
            Reaction(key: "🎉", count: 1, byUs: false, senders: [Make.reader("bob")]),
        ]
        let quote = ReplyQuote(
            eventId: "$orig", state: .loaded, senderId: "@bob:example.org", senderName: "Bob", kind: .text,
            body: "Hello")
        let event = Make.event(
            "alice", Make.text("Hi", edited: true, reactions: reactions, replyTo: quote), eventId: "$1",
            readBy: [Make.reader("bob"), Make.reader("carol")])
        guard let row = messageRow(event) else { return XCTFail("no message") }
        XCTAssertEqual(row.reactions, reactions)
        XCTAssertEqual(row.quote, quote)
        XCTAssertTrue(row.edited)
        XCTAssertEqual(row.readers.map(\.userId), ["@bob:example.org", "@carol:example.org"])
        // Back as the library gave it.
        XCTAssertEqual(Make.row("m", event).content, row.content)
    }

    func testMessagesThatCanBeRepliedToSwipeToReply() {
        let hi = Make.event("bob", Make.text("Hi"), eventId: "$1")
        XCTAssertEqual(messageRow(hi, actions: [.react, .reply, .copy])?.canSwipeToReply, true)
        XCTAssertEqual(messageRow(hi, actions: [.copy, .selectText])?.canSwipeToReply, false)
    }

    func testSwipeToReplyBeginsLeftwardAndRepliesPastTheThreshold() {
        XCTAssertTrue(SwipeToReply.begins(velocity: CGPoint(x: -300, y: 40)))
        // Scrolling, the back gesture, and diagonal drags are left alone.
        XCTAssertFalse(SwipeToReply.begins(velocity: CGPoint(x: -100, y: 300)))
        XCTAssertFalse(SwipeToReply.begins(velocity: CGPoint(x: 300, y: 0)))
        XCTAssertFalse(SwipeToReply.begins(velocity: CGPoint(x: -100, y: -60)))

        var swipe = SwipeToReply()
        swipe.update(CGPoint(x: -30, y: 5))
        XCTAssertEqual(swipe.offset, -30)
        XCTAssertFalse(swipe.triggers)
        XCTAssertEqual(swipe.progress, 30 / SwipeToReply.threshold)
        swipe.update(CGPoint(x: -SwipeToReply.threshold, y: 0))
        XCTAssertTrue(swipe.triggers)
        XCTAssertEqual(swipe.progress, 1)
        // The bubble stops at the limit, and does not go right.
        swipe.update(CGPoint(x: -500, y: 0))
        XCTAssertEqual(swipe.offset, -SwipeToReply.limit)
        swipe.update(CGPoint(x: 40, y: 0))
        XCTAssertEqual(swipe.offset, 0)
        XCTAssertFalse(swipe.triggers)
    }

    func testReadersBeyondThreeShowAsACount() {
        let readers = ["a", "b", "c", "d", "e"].map { Make.reader($0) }
        let (shown, more) = ReaderAvatars.split(readers)
        XCTAssertEqual(shown.map(\.userId), readers.prefix(3).map(\.userId))
        XCTAssertEqual(more, 2)
        XCTAssertEqual(ReaderAvatars.split(Array(readers.prefix(2))).more, 0)
        // A name not known yet: the user ID's local part.
        XCTAssertEqual(Make.reader("anna", named: false).displayName, "anna")
        XCTAssertEqual(Make.reader("anna").displayName, "Anna")
    }

    func testQuoteSaysWhatTheOriginalIs() {
        func quote(_ state: QuoteState, kind: MessageKind? = .text, body: String? = "Hello", name: String? = "Bob")
            -> ReplyQuote
        {
            ReplyQuote(
                eventId: "$orig", state: state, senderId: state == .loaded ? "@bob:example.org" : nil,
                senderName: state == .loaded ? name : nil, kind: kind, body: body)
        }
        XCTAssertEqual(quote(.loading, body: nil).preview, "Loading…")
        XCTAssertFalse(quote(.loading, body: nil).canGoTo)
        XCTAssertEqual(quote(.failed, body: nil).preview, "Could not load the message")
        XCTAssertFalse(quote(.failed, body: nil).canGoTo)
        XCTAssertNil(quote(.failed, body: nil).senderLabel)

        let text = quote(.loaded)
        XCTAssertEqual(text.senderLabel, "Bob")
        XCTAssertEqual(text.preview, "Hello")
        XCTAssertTrue(text.canGoTo)
        XCTAssertEqual(quote(.loaded, name: nil).senderLabel, "bob")
        XCTAssertEqual(quote(.loaded, kind: .image, body: "cat.png").preview, "Image: cat.png")
        XCTAssertEqual(quote(.loaded, kind: .file, body: "").preview, "File")
        XCTAssertEqual(quote(.loaded, kind: nil, body: "Message deleted").preview, "Message deleted")
    }

    func testEmojiSearchByName() {
        XCTAssertTrue(Emoji.all.count > 1000)
        XCTAssertEqual(Set(Emoji.all.map(\.symbol)).count, Emoji.all.count)
        XCTAssertEqual(Emoji.all.first?.symbol, "😀")
        XCTAssertTrue(Emoji.search("tears joy").map(\.symbol).contains("😂"))
        XCTAssertFalse(Emoji.search("tears joy").map(\.symbol).contains("😀"))
        XCTAssertEqual(Emoji.search("  ").count, Emoji.all.count)
    }

    func testLocalpart() {
        XCTAssertEqual(localpart("@alice:example.org"), "alice")
        XCTAssertEqual(localpart("bob"), "bob")
    }
}
