package org.mtrx.app.timeline

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.job
import kotlinx.coroutines.launch
import org.mtrx.app.model.describe
import uniffi.mtrx.Download
import uniffi.mtrx.DownloadsListener
import uniffi.mtrx.Draft
import uniffi.mtrx.DraftKeeper
import uniffi.mtrx.Media
import uniffi.mtrx.MtrxException
import uniffi.mtrx.Session
import uniffi.mtrx.TaskHandle
import uniffi.mtrx.Timeline
import uniffi.mtrx.TimelineRow as LibraryRow
import uniffi.mtrx.TimelineRowsListener
import uniffi.mtrx.TypingListener

/**
 * A room's timeline screen: its rows, older events on demand, the composer
 * (a new message, a reply or an edit), who is typing, the actions on a
 * message, going to the original of a reply and a direct chat's person
 * ([personInfo]). The pinned messages are [PinnedModel]'s, the topic is
 * saved by the room's `RoomInfoModel`, and the rooms shared with the person
 * are loaded by their panel. Methods are called on the main thread.
 *
 * Actions on a message take its row's id from the latest [rows]. Sends and
 * the other actions (invites too) run in a scope of their own, so leaving
 * the room (which clears this model) does not cancel them. [onRead] follows
 * each successful [markRead] (the room's notifications go); [onInput] each
 * change of the draft (typing on the keyboard, which the activity does not
 * see, is not idle).
 *
 * The draft is kept on this device by the library's [DraftKeeper] (#42):
 * back in the composer when the room opens, unless something was written
 * meanwhile (as a reply once the message it answers is in the rows), told
 * every change (saved once typing stops), saved at once when the room is
 * left or the app stops ([keepDraft]), cleared on send; during an edit,
 * the text from before it.
 */
class TimelineViewModel(
    private val roomId: String,
    private val session: Session,
    private val onRead: () -> Unit,
    private val onInput: () -> Unit = {},
) : ViewModel(), TimelineActions {
    private val _rows = MutableStateFlow<List<TimelineRow>>(emptyList())
    val rows: StateFlow<List<TimelineRow>> = _rows.asStateFlow()

    private val _draft = MutableStateFlow("")
    val draft: StateFlow<String> = _draft.asStateFlow()

    /** What the draft answers or edits; null for a new message. */
    private val _composing = MutableStateFlow<Composing?>(null)
    val composing: StateFlow<Composing?> = _composing.asStateFlow()

    /** Names of the others typing. */
    private val _typing = MutableStateFlow<List<String>>(emptyList())
    val typing: StateFlow<List<String>> = _typing.asStateFlow()

    /** Older events are loaded to find a reply's original. */
    private val _seeking = MutableStateFlow(false)
    val seeking: StateFlow<Boolean> = _seeking.asStateFlow()

    /** The row gone to, marked for a moment. */
    private val _highlighted = MutableStateFlow<String?>(null)
    val highlighted: StateFlow<String?> = _highlighted.asStateFlow()

    /** Rows to scroll to. */
    private val _scrollTo = MutableSharedFlow<String>(extraBufferCapacity = 1)
    val scrollTo: SharedFlow<String> = _scrollTo.asSharedFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    /** The sheet about a direct chat's person, while open. */
    private val _personInfo = MutableStateFlow<PersonInfoModel?>(null)
    val personInfo: StateFlow<PersonInfoModel?> = _personInfo.asStateFlow()

    /** Each message's file's newest download, by its source. */
    private val _downloads = MutableStateFlow<Map<String, Download>>(emptyMap())
    val downloads: StateFlow<Map<String, Download>> = _downloads.asStateFlow()

    /** Files downloaded for the screen to open, share or save. */
    private val _files = MutableSharedFlow<FileRequest>(extraBufferCapacity = 4)
    val files: SharedFlow<FileRequest> = _files.asSharedFlow()

    private val downloadsSubscription: TaskHandle = session.subscribeDownloads(object : DownloadsListener {
        // On a library thread.
        override fun onUpdate(downloads: List<Download>) {
            _downloads.value = downloadsBySource(downloads)
        }
    })

    private var timeline: Timeline? = null
    private var subscription: TaskHandle? = null
    private var typingSubscription: TaskHandle? = null
    private var nearTop = false
    private var paginating = false
    private var atStart = false
    private var markedRead: String? = null
    private var seek: Job? = null

    // Not cancelled with viewModelScope: completed when the room is left, it
    // ends once the actions under way are done.
    private val actionsJob = SupervisorJob()
    private val actions = CoroutineScope(actionsJob + Dispatchers.Main.immediate)

    // Typing notices in order, the latest only when calls wait.
    private val typingNotices = Channel<Boolean>(Channel.CONFLATED)
    private var typingOn = false

    private val drafts: DraftKeeper = session.draftKeeper(roomId)

    /** The event the restored draft answers, until it is answered or another reply or edit starts. */
    private var pendingReply: String? = null

    /** [timeline], once open. */
    private val timelineOpened = CompletableDeferred<Timeline>()

    init {
        viewModelScope.launch { restoreDraft() }
        viewModelScope.launch {
            try {
                val opened = session.timeline(roomId)
                timeline = opened
                timelineOpened.complete(opened)
                subscription = opened.subscribeRows(object : TimelineRowsListener {
                    // On a library thread.
                    override fun onUpdate(rows: List<LibraryRow>) {
                        _rows.value = shownRows(rows)
                    }
                })
                typingSubscription = opened.subscribeTyping(object : TypingListener {
                    // On a library thread.
                    override fun onUpdate(names: List<String>) {
                        _typing.value = names
                    }
                })
                launch { sendTypingNotices(opened) }
                if (nearTop) loadOlder()
            } catch (e: MtrxException) {
                _error.value = describe(e)
            }
        }
    }

    /** Pin or unpin the message with [eventId]. */
    override fun setPinned(eventId: String, pin: Boolean) {
        actions.launch {
            try {
                session.room(roomId).use { it.setPinned(eventId, pin) }
            } catch (e: MtrxException) {
                _error.value = describe(e)
            }
        }
    }

    override fun file(media: Media, eventId: String?, use: FileUse) {
        actions.launch {
            val stopped = _downloads.value[media.source]?.takeIf { it.stopped }
            val id = try {
                if (stopped != null) {
                    session.retryDownload(stopped.id)
                    stopped.id
                } else {
                    session.room(roomId).use { it.download(media, eventId) }
                }
            } catch (e: MtrxException) {
                _error.value = describe(e)
                return@launch
            }
            val path = try {
                session.downloadFinished(id)
            } catch (e: MtrxException) {
                // Failed or cancelled: its message says so.
                return@launch
            }
            _files.emit(FileRequest(path, media, use))
        }
    }

    override fun cancelDownload(id: ULong) {
        session.cancelDownload(id)
    }

    /** The list is scrolled (nearly) to its top: load older events while it is. */
    override fun onNearTop(value: Boolean) {
        nearTop = value
        if (value) loadOlder()
    }

    private fun loadOlder() {
        val current = timeline ?: return
        if (paginating || atStart) return
        paginating = true
        viewModelScope.launch {
            val more = try {
                paginate(current, PAGE)
                true
            } catch (e: MtrxException) {
                _error.value = describe(e)
                false
            } finally {
                paginating = false
            }
            // Short pages may not move the list away from its top.
            if (more && nearTop) loadOlder()
        }
    }

    /** Load [events] older events (after the loading under way); true at the room's start. */
    private suspend fun paginate(current: Timeline, events: UShort): Boolean =
        current.paginateBack(events).also { if (it) atStart = true }

    /** The newest message is on screen. */
    override fun markRead() {
        val current = timeline ?: return
        val latest = latestEventId(_rows.value) ?: return
        if (latest == markedRead) return
        markedRead = latest
        viewModelScope.launch {
            try {
                current.markRead()
            } catch (e: MtrxException) {
                markedRead = null
                return@launch
            }
            onRead()
        }
    }

    /** The user changed the text. */
    override fun onDraft(value: String) {
        onInput()
        _draft.value = value
        drafts.changed(currentDraft())
        typed(value.isNotEmpty())
    }

    /** The draft left in the room, in the composer unless one was started meanwhile; its reply once its message is there. */
    private suspend fun restoreDraft() {
        val saved = drafts.restore() ?: return
        if (_draft.value.isNotEmpty() || _composing.value != null) return
        _draft.value = saved.text
        val replyTo = saved.replyTo ?: return
        pendingReply = replyTo
        answerOnceShown(replyTo)
    }

    /** Answer [replyTo], the [pendingReply], once its message is among the rows. */
    private fun answerOnceShown(replyTo: String) {
        viewModelScope.launch {
            // Found by the library, which loads nothing for it.
            val itemId = try {
                timelineOpened.await().itemOf(replyTo)
            } catch (e: MtrxException) {
                return@launch
            }
            // The rows arrive apart from the call.
            _rows.first { rows -> rows.any { it.id == itemId } }
            if (pendingReply == replyTo) startReply(itemId)
        }
    }

    /**
     * Put [unsent]'s text in the composer: after what is written and a blank line, or as is in an empty one, then
     * answering its message unless another is answered. During an edit, in the draft that comes back after it.
     */
    private fun putBack(unsent: Draft) {
        val mode = _composing.value
        if (mode is Composing.Editing) {
            _composing.value = mode.copy(draft = appending(unsent.text, mode.draft))
        } else {
            val empty = _draft.value.isBlank()
            _draft.value = appending(unsent.text, _draft.value)
            val replyTo = unsent.replyTo
            if (empty && mode == null && pendingReply == null && replyTo != null) {
                pendingReply = replyTo
                answerOnceShown(replyTo)
            }
        }
        drafts.changed(currentDraft())
    }

    /** The room's draft: the text and what it answers, or during an edit the text from before it. */
    private fun currentDraft(): Draft = when (val mode = _composing.value) {
        is Composing.Editing -> Draft(mode.draft, null)
        is Composing.Replying -> Draft(_draft.value, mode.eventId)
        null -> Draft(_draft.value, pendingReply)
    }

    /** Save the draft now (the app stops; leaving the room and sending do it too). */
    fun keepDraft() {
        actions.launch { drafts.flush() }
    }

    /** Tell the others: on every change while there is text (the library throttles), once when it is gone. */
    private fun typed(typing: Boolean) {
        if (!typing && !typingOn) return
        typingOn = typing
        typingNotices.trySend(typing)
    }

    private suspend fun sendTypingNotices(current: Timeline) {
        for (typing in typingNotices) {
            try {
                current.setTyping(typing)
            } catch (e: MtrxException) {
                // Only a hint to the others.
            }
        }
    }

    /**
     * Reply to row [itemId] (from its menu, or swiped): the composer says to
     * whom; an edit under way ends. Only a sent message can be answered.
     */
    override fun startReply(itemId: String) {
        val row = eventRow(itemId)?.takeIf { it.canReply } ?: return
        val eventId = row.event.eventId ?: return
        endEdit()
        pendingReply = null
        _composing.value = Composing.Replying(itemId, row.event.sender.displayName, preview(row), eventId)
        drafts.changed(currentDraft())
    }

    /** Edit row [itemId]: the composer gets its text as we wrote it; a reply under way ends. */
    override fun startEdit(itemId: String) {
        val message = eventRow(itemId)?.message ?: return
        val original = message.rawBody.ifEmpty { message.body }
        val draft = (_composing.value as? Composing.Editing)?.draft ?: _draft.value
        pendingReply = null
        _composing.value = Composing.Editing(itemId, original, draft)
        _draft.value = original
        drafts.changed(currentDraft())
    }

    /** Back to a new message; an edit gives back the text from before it. */
    override fun cancelComposing() {
        endEdit()
        pendingReply = null
        _composing.value = null
        drafts.changed(currentDraft())
    }

    private fun endEdit() {
        val editing = _composing.value as? Composing.Editing ?: return
        _composing.value = null
        _draft.value = editing.draft
        drafts.changed(currentDraft())
    }

    /** Send the draft, as a reply or an edit if one is under way; false if there is nothing to send. */
    override fun send(): Boolean {
        val current = timeline ?: return false
        val text = _draft.value
        if (text.isBlank()) return false
        val mode = _composing.value
        pendingReply = null
        _composing.value = null
        _draft.value = (mode as? Composing.Editing)?.draft ?: ""
        drafts.changed(currentDraft())
        keepDraft()
        typed(false)
        if (mode is Composing.Editing && text == mode.original) return true
        actions.launch {
            try {
                when (mode) {
                    null -> current.sendText(text)
                    is Composing.Replying -> current.reply(mode.itemId, text)
                    is Composing.Editing -> current.edit(mode.itemId, text)
                }
            } catch (e: MtrxException) {
                _error.value = describeAction(e)
                restore(text, mode)
                // Kept, even if the room was left meanwhile.
                drafts.changed(currentDraft())
                drafts.flush()
            }
        }
        return true
    }

    /** Back in the composer after a failed send, unless another was started. */
    private fun restore(text: String, mode: Composing?) {
        if (_composing.value != null) return
        when (mode) {
            is Composing.Editing -> {
                _composing.value = mode.copy(draft = _draft.value)
                _draft.value = text
            }
            else -> if (_draft.value.isEmpty()) {
                _composing.value = mode
                _draft.value = text
            }
        }
    }

    /** The name or avatar of a direct chat's header was tapped: the sheet about [userId]. */
    override fun showPersonInfo(userId: String) {
        if (_personInfo.value?.userId == userId) return
        _personInfo.value?.close()
        _personInfo.value = PersonInfoModel(userId, session, viewModelScope, actions)
    }

    fun closePersonInfo() {
        _personInfo.value?.close()
        _personInfo.value = null
    }

    /** Add our reaction [key] to row [itemId], or remove it. */
    override fun react(itemId: String, key: String) = act { it.react(itemId, key) }

    /** Delete row [itemId]'s message for everyone. */
    override fun delete(itemId: String) = act { it.delete(itemId) }

    /** Send row [itemId]'s failed message again. */
    override fun retry(itemId: String) = act { it.retry(itemId) }

    /** Take row [itemId]'s message, not sent yet, out of the send queue: its text comes back in the composer. */
    override fun unsend(itemId: String) = act { timeline -> timeline.unsend(itemId)?.let(::putBack) }

    private fun act(action: suspend (Timeline) -> Unit) {
        val current = timeline ?: return
        actions.launch {
            try {
                action(current)
            } catch (e: MtrxException) {
                _error.value = describeAction(e)
            }
        }
    }

    /**
     * Go to the original of a reply: scroll to its row and mark it, once the
     * library loaded the older events it is in ([Timeline.seek]) and the
     * rows have it. A new one replaces one under way.
     */
    override fun goToOriginal(eventId: String) {
        val current = timeline ?: return
        seek?.cancel()
        // Started once it is `seek`: it may end without suspending.
        val job = viewModelScope.launch(start = CoroutineStart.LAZY) {
            val me = coroutineContext.job
            val found = rowOf(_rows.value, eventId) ?: run {
                _seeking.value = true
                try {
                    current.seek(eventId)
                } catch (e: MtrxException) {
                    _error.value = describe(e)
                    return@launch
                } finally {
                    if (seek === me) _seeking.value = false
                }
            }
            // The rows arrive apart from the call.
            _rows.first { rows -> rows.any { it.id == found } }
            _scrollTo.tryEmit(found)
            _highlighted.value = found
            delay(HIGHLIGHT_MILLIS)
            if (_highlighted.value == found) _highlighted.value = null
        }
        seek = job
        job.start()
    }

    private fun rowOf(rows: List<TimelineRow>, eventId: String): String? =
        rows.firstOrNull { it is EventRow && it.event.eventId == eventId }?.id

    private fun eventRow(itemId: String): EventRow? = _rows.value.firstOrNull { it.id == itemId } as? EventRow

    /** A row gone meanwhile is invalid input to the library. */
    private fun describeAction(e: MtrxException): String? =
        if (e is MtrxException.InvalidInput) GONE else describe(e)

    fun clearError() {
        _error.value = null
    }

    override fun onCleared() {
        keepDraft()
        for (handle in listOfNotNull(subscription, typingSubscription, downloadsSubscription)) {
            handle.cancel()
            handle.close()
        }
        typingNotices.close()
        val current = timeline
        if (current != null) {
            actions.launch {
                try {
                    if (typingOn) current.setTyping(false)
                } catch (e: MtrxException) {
                    // Only a hint to the others.
                } finally {
                    // Calls under way keep the library's timeline until they end.
                    current.close()
                }
            }
        }
        actionsJob.complete()
    }

    companion object {
        val PAGE: UShort = 30u
        const val HIGHLIGHT_MILLIS = 2_000L

        const val GONE = "The message is no longer in the timeline"
    }
}

/** [text] after [written] and a blank line; as is if nothing is written. */
internal fun appending(text: String, written: String): String {
    val before = written.trimEnd()
    return if (before.isEmpty()) text else "$before\n\n$text"
}
