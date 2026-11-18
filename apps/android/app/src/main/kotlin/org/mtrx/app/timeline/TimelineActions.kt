package org.mtrx.app.timeline

import java.time.Duration
import uniffi.mtrx.Media

/**
 * What the room screen asks of its model ([TimelineViewModel]); the
 * screenshot tests give one that does nothing. Rows by their
 * `TimelineRow.id`, messages by their event ID.
 */
interface TimelineActions {
    fun onNearTop(value: Boolean)

    fun markRead()

    fun onDraft(value: String)

    fun startReply(itemId: String)

    fun startEdit(itemId: String)

    fun cancelComposing()

    /** What is sent from now on disappears [afterRead] after everyone has read it; null: not. */
    fun chooseDisappear(afterRead: Duration?)

    /** False if there is nothing to send. */
    fun send(): Boolean

    /** A file picked to send, which [prepare] makes ([prepareToSend]): its card shows at once, ready once made. */
    fun attach(prepare: Prepare)

    /** Take picked file [id] away: it is not sent, and its copy goes. */
    fun detach(id: String)

    fun react(itemId: String, key: String)

    fun delete(itemId: String)

    fun retry(itemId: String)

    /** Take a message not sent yet out of the send queue, its text back in the composer. */
    fun unsend(itemId: String)

    fun goToOriginal(eventId: String)

    /** The sheet about [person], shown as they are until their profile loads: a direct chat's, or a mention's. */
    fun showPersonInfo(person: PersonCard)

    fun setPinned(eventId: String, pin: Boolean)

    /** Download a message's file (once; a failed download again), then hand it on for [use] ([TimelineViewModel.files]). */
    fun file(media: Media, eventId: String?, use: FileUse)

    fun cancelDownload(id: ULong)

    /** An audio message's button: play, pause, resume, cancel its fetch or try again ([AudioPlayer.toggle]). */
    fun toggleAudio(media: Media)

    /** Move an audio message to [to]; one not playing starts there. */
    fun seekAudio(media: Media, to: Duration)
}
