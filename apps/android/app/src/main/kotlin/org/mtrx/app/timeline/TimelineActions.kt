package org.mtrx.app.timeline

import uniffi.mtrx.Topic

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

    /** False if there is nothing to send. */
    fun send(): Boolean

    fun react(itemId: String, key: String)

    fun delete(itemId: String)

    fun retry(itemId: String)

    /** Take a message not sent yet out of the send queue, its text back in the composer. */
    fun unsend(itemId: String)

    fun goToOriginal(eventId: String)

    fun showPersonInfo(userId: String)

    /** The rooms shared with a direct chat's person [userId] are shown: load them. */
    fun showSharedRooms(userId: String)

    fun clearError()

    fun saveTopic(topic: Topic)

    fun setPinned(eventId: String, pin: Boolean)

    fun showPinned()

    fun hidePinned()
}
