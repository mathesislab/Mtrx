package org.mtrx.app.timeline

import java.time.LocalDate
import java.time.ZoneId
import uniffi.mtrx.EventContent
import uniffi.mtrx.MediaKind
import uniffi.mtrx.Message
import uniffi.mtrx.MessageAction
import uniffi.mtrx.MessageKind
import uniffi.mtrx.QuoteState
import uniffi.mtrx.Reaction
import uniffi.mtrx.ReplyQuote
import uniffi.mtrx.RowContent
import uniffi.mtrx.TimelineEvent
import uniffi.mtrx.User
import uniffi.mtrx.TimelineRow as LibraryRow

/** A row of the timeline screen; [id] is stable (list identity). */
sealed interface TimelineRow {
    val id: String
}

/** The room's beginning. */
data class StartRow(override val id: String) : TimelineRow

data class DayRow(override val id: String, val date: LocalDate) : TimelineRow

/** Below are unread messages. */
data class ReadMarkerRow(override val id: String) : TimelineRow

/**
 * A message, or a deleted or undecryptable one, as the library gives it:
 * [showSender] (name and avatar) at the first of a group by someone else
 * outside direct chats, [avatarColumn] for others' outside direct chats,
 * [decrypting] while its key may still arrive, and [actions] in the menu's
 * order. Actions on it take [id] (`TimelineRow.id`).
 */
data class EventRow(
    override val id: String,
    val event: TimelineEvent,
    val showSender: Boolean,
    val avatarColumn: Boolean,
    val decrypting: Boolean,
    val actions: List<MessageAction>,
) : TimelineRow {
    /** The message; null if deleted, undecryptable or not a message. */
    val message: Message? get() = (event.content as? EventContent.Message)?.message

    val edited: Boolean get() = message?.edited == true

    /** In the order first given. */
    val reactions: List<Reaction> get() = message?.reactions.orEmpty()

    /** The message this one answers. */
    val quote: Quote? get() = message?.replyTo?.let(::quote)

    /** Who read up to here. */
    val readers: Readers get() = Readers(event.readBy)

    /** Swiping it left replies, as Reply in its menu: a sent message. */
    val canReply: Boolean get() = MessageAction.REPLY in actions

    /** What it says in place of a message without its key. */
    val undecryptableText: String
        get() = if (decrypting) "Decrypting…" else "Could not decrypt this message."
}

/** A menu's name for the action. */
val MessageAction.label: String
    get() = when (this) {
        MessageAction.REACT -> "React"
        MessageAction.REPLY -> "Reply"
        MessageAction.COPY -> "Copy"
        MessageAction.SELECT_TEXT -> "Select text"
        MessageAction.EDIT -> "Edit"
        MessageAction.PIN -> "Pin"
        MessageAction.UNPIN -> "Unpin"
        MessageAction.DELETE -> "Delete"
        MessageAction.RETRY -> "Retry"
        MessageAction.UNSEND -> "Cancel and put back"
    }

/** A message's text, plain, if it shows any (a file's only with a caption). */
fun messageText(message: Message): String? = message.body.takeIf { message.text.isNotEmpty() && it.isNotBlank() }

/** The emoji offered first for a reaction. */
val COMMON_REACTIONS = listOf("👍", "❤️", "😂", "😮", "😢", "🎉")

/** The quote of the message a reply answers, above the reply's text. */
data class Quote(
    /** The original's event ID: go to the row with it. */
    val eventId: String,
    /** Null until loaded. */
    val sender: String?,
    val text: String,
    val state: QuoteState,
) {
    /** A loaded quote goes to the original. */
    val tappable: Boolean get() = state == QuoteState.LOADED
}

fun quote(reply: ReplyQuote): Quote = Quote(
    eventId = reply.eventId,
    sender = reply.sender?.displayName,
    text = when (reply.state) {
        QuoteState.LOADING -> "Loading…"
        QuoteState.FAILED -> "Could not load the message"
        QuoteState.LOADED -> listOfNotNull(reply.kind?.let(::kindLabel), reply.body).joinToString(" · ")
    },
    state = reply.state,
)

/** "Image", "Video", "Audio" or "File" for a message of that kind. */
fun kindLabel(kind: MessageKind): String? = when (kind) {
    MessageKind.IMAGE -> "Image"
    MessageKind.VIDEO -> "Video"
    MessageKind.AUDIO -> "Audio"
    MessageKind.FILE -> "File"
    else -> null
}

private fun mediaLabel(kind: MediaKind): String = when (kind) {
    MediaKind.IMAGE -> "Image"
    MediaKind.VIDEO -> "Video"
    MediaKind.AUDIO -> "Audio"
    MediaKind.FILE -> "File"
}

/** The first line of a message, as the composer shows what it answers. */
fun preview(row: EventRow): String {
    val message = row.message ?: return ""
    val media = message.media
    val text = messageText(message)?.lineSequence()?.firstOrNull()
    return when {
        media != null -> listOfNotNull(mediaLabel(media.kind), text ?: media.filename).joinToString(" · ")
        else -> text.orEmpty()
    }
}

/** Who read up to a message: the first [MAX_SHOWN] shown, then "+[more]". */
data class Readers(val all: List<User>) {
    val shown: List<User> get() = all.take(MAX_SHOWN)
    val more: Int get() = (all.size - MAX_SHOWN).coerceAtLeast(0)

    companion object {
        const val MAX_SHOWN = 3
    }
}

/** Consecutive state events (joins, renames, …) in one line. */
data class StateRow(override val id: String, val text: String) : TimelineRow

/** The library's rows (oldest first), each as the screen shows it; days in [zone]. */
fun shownRows(rows: List<LibraryRow>, zone: ZoneId = ZoneId.systemDefault()): List<TimelineRow> = rows.map { row ->
    when (val content = row.content) {
        RowContent.Start -> StartRow(row.id)
        is RowContent.Day -> DayRow(row.id, content.date.atZone(zone).toLocalDate())
        RowContent.ReadMarker -> ReadMarkerRow(row.id)
        is RowContent.Summary -> StateRow(row.id, content.text)
        is RowContent.Message -> EventRow(
            row.id,
            content.event,
            showSender = content.showSender,
            avatarColumn = content.avatarColumn,
            decrypting = content.decrypting,
            actions = content.actions,
        )
    }
}

/** The newest message's row, to mark as read. */
fun latestEventId(rows: List<TimelineRow>): String? = rows.lastOrNull { it is EventRow }?.id
