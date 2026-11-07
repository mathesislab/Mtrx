package org.mtrx.app.timeline

import java.time.Instant
import java.time.LocalDate
import java.time.ZoneOffset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mtrx.app.eventItem
import org.mtrx.app.messageEvent
import org.mtrx.app.paragraph
import org.mtrx.app.plain
import org.mtrx.app.textEvent
import org.mtrx.app.textMessage
import org.mtrx.app.user
import uniffi.mtrx.EventContent
import uniffi.mtrx.Media
import uniffi.mtrx.MediaKind
import uniffi.mtrx.Message
import uniffi.mtrx.MessageAction
import uniffi.mtrx.MessageKind
import uniffi.mtrx.QuoteState
import uniffi.mtrx.Reaction
import uniffi.mtrx.ReplyQuote
import uniffi.mtrx.RowContent
import uniffi.mtrx.SendState
import uniffi.mtrx.TimelineRow as LibraryRow

class TimelineRowsTest {
    private val alice = "@alice:example.org"
    private val bob = "@bob:example.org"
    private val me = "@me:example.org"

    @Test
    fun showsTheLibrarysRows() {
        val rows = shownRows(
            listOf(
                LibraryRow("start", RowContent.Start),
                LibraryRow("day-0", RowContent.Day(Instant.EPOCH)),
                textEvent("1", alice, "hi"),
                LibraryRow("read-marker", RowContent.ReadMarker),
                LibraryRow("j", RowContent.Summary("bob joined")),
                textEvent("2", "@me:example.org", "mine", own = true),
            ),
            ZoneOffset.UTC,
        )
        assertEquals(listOf("start", "day-0", "1", "read-marker", "j", "2"), rows.map { it.id })
        assertEquals(LocalDate.of(1970, 1, 1), (rows[1] as DayRow).date)
        assertEquals("bob joined", (rows[4] as StateRow).text)
        val alices = rows[2] as EventRow
        assertTrue(alices.showSender && alices.avatarColumn)
        assertEquals(listOf(MessageAction.REACT, MessageAction.REPLY, MessageAction.COPY, MessageAction.SELECT_TEXT), alices.actions)
        // Our own messages show no sender.
        assertFalse((rows[5] as EventRow).showSender)
        assertEquals("2", latestEventId(rows))
    }

    private fun text(body: String, edited: Boolean = false, reactions: List<Reaction> = emptyList(), replyTo: ReplyQuote? = null) =
        textMessage(body, edited = edited, reactions = reactions, replyTo = replyTo)

    private fun row(row: LibraryRow): EventRow = shownRows(listOf(row)).single() as EventRow

    private fun quoted(state: QuoteState, kind: MessageKind? = null, body: String? = null) = ReplyQuote(
        eventId = "\$original",
        state = state,
        sender = if (state == QuoteState.LOADED) user(alice) else null,
        kind = kind,
        body = body,
    )

    @Test
    fun rowsCarryReactionsQuotesEditsAndReaders() {
        val reactions = listOf(
            Reaction(key = "👍", count = 2uL, byUs = true, senders = listOf(user(me, "Me"), user(bob))),
            Reaction(key = "🎉", count = 1uL, byUs = false, senders = listOf(user(bob, "Bob"))),
        )
        val readers = listOf("a", "b", "c", "d", "e").map { user("@$it:example.org") }
        val row = row(
            messageEvent(
                "2",
                bob,
                text("yes", edited = true, reactions = reactions, replyTo = quoted(QuoteState.LOADED, MessageKind.TEXT, "shall we?")),
                readBy = readers,
            ),
        )
        assertTrue(row.edited)
        assertEquals(listOf("👍" to true, "🎉" to false), row.reactions.map { it.key to it.byUs })
        assertEquals(Quote("\$original", "alice", "shall we?", QuoteState.LOADED), row.quote)
        assertTrue(row.quote!!.tappable)
        // At most three avatars, then "+2".
        assertEquals(listOf("@a:example.org", "@b:example.org", "@c:example.org"), row.readers.shown.map { it.userId })
        assertEquals(2, row.readers.more)
        assertEquals(0, Readers(readers.take(3)).more)

        val plain = row(textEvent("3", bob, "plain"))
        assertFalse(plain.edited)
        assertNull(plain.quote)
        assertTrue(plain.reactions.isEmpty())
        assertTrue(plain.readers.all.isEmpty())
    }

    @Test
    fun quotesSayWhatTheOriginalIs() {
        assertEquals(Quote("\$original", null, "Loading…", QuoteState.LOADING), quote(quoted(QuoteState.LOADING)))
        val failed = quote(quoted(QuoteState.FAILED))
        assertEquals("Could not load the message", failed.text)
        assertFalse(failed.tappable)
        assertEquals("Image · ice.png", quote(quoted(QuoteState.LOADED, MessageKind.IMAGE, "ice.png")).text)
        assertEquals("File · data.nc", quote(quoted(QuoteState.LOADED, MessageKind.FILE, "data.nc")).text)
        // A deleted original: no kind, the library's text.
        assertEquals("Message deleted", quote(quoted(QuoteState.LOADED, null, "Message deleted")).text)
        assertEquals("alice", quote(quoted(QuoteState.LOADED)).sender)
    }

    @Test
    fun swipingRepliesWhereTheMenuDoes() {
        assertTrue(row(textEvent("1", bob, "hi")).canReply)
        assertFalse(
            "not sent yet",
            row(messageEvent("3", me, text("on its way"), own = true, eventId = null, sendState = SendState.Sending)).canReply,
        )
        assertFalse("deleted", row(eventItem("5", bob, EventContent.Redacted)).canReply)
    }

    @Test
    fun anImageWithoutACaptionPreviewsAsItsName() {
        val image = Message(
            kind = MessageKind.IMAGE,
            body = "ice.png",
            edited = false,
            media = Media(MediaKind.IMAGE, "ice.png", null, "image/png", 800u, 600u, "{}"),
        )
        val imageRow = row(messageEvent("8", bob, image))
        assertNull(messageText(image))
        assertEquals("Image · ice.png", preview(imageRow))
        // With a caption, the library gives it as text.
        val captioned = image.copy(body = "The ice sheet", text = listOf(paragraph(plain("The ice sheet"))))
        assertEquals("The ice sheet", messageText(captioned))
        assertEquals("Image · The ice sheet", preview(row(messageEvent("9", bob, captioned))))
    }

    @Test
    fun undecryptableRowSaysDecryptingWhileKeysArrive() {
        val undecryptable = eventItem("1", bob, EventContent.UnableToDecrypt)
        assertEquals("Could not decrypt this message.", row(undecryptable).undecryptableText)
        val content = undecryptable.content as RowContent.Message
        assertEquals("Decrypting…", row(LibraryRow("1", content.copy(decrypting = true))).undecryptableText)
    }
}
