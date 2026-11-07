package org.mtrx.app.timeline

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.mtrx.MtrxException
import uniffi.mtrx.Session
import uniffi.mtrx.TaskHandle
import uniffi.mtrx.Timeline
import uniffi.mtrx.TimelineRow as LibraryRow
import uniffi.mtrx.TimelineRowsListener

/**
 * A room's pinned messages, followed while the topic panel shows them
 * ([show] to [hide]). Methods are called on the main thread.
 */
class PinnedModel(private val roomId: String, private val session: Session) : ViewModel() {
    /** The pinned messages while shown; null while loading (or hidden), empty if they cannot be read. */
    private val _pinned = MutableStateFlow<List<EventRow>?>(null)
    val pinned: StateFlow<List<EventRow>?> = _pinned.asStateFlow()

    private var timeline: Timeline? = null
    private var subscription: TaskHandle? = null
    private var loading: Job? = null

    /** The topic panel opened: load the pinned messages into [pinned] and follow them. */
    fun show() {
        if (loading != null) return
        loading = viewModelScope.launch {
            try {
                val opened = session.pinnedTimeline(roomId)
                timeline = opened
                subscription = opened.subscribeRows(object : TimelineRowsListener {
                    // On a library thread.
                    override fun onUpdate(rows: List<LibraryRow>) {
                        // None yet: still loading (the panel asks only where some are pinned).
                        _pinned.value = shownRows(rows).filterIsInstance<EventRow>().ifEmpty { null }
                    }
                })
            } catch (e: MtrxException) {
                _pinned.value = emptyList()
            }
        }
    }

    /** The topic panel closed: the pinned messages are no longer followed. */
    fun hide() {
        loading?.cancel()
        loading = null
        subscription?.let {
            it.cancel()
            it.close()
        }
        subscription = null
        timeline?.close()
        timeline = null
        _pinned.value = null
    }

    override fun onCleared() = hide()
}
