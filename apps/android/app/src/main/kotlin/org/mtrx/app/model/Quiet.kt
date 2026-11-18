package org.mtrx.app.model

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.mtrx.MtrxException
import uniffi.mtrx.PauseFor
import uniffi.mtrx.PauseMenuItem
import uniffi.mtrx.QuietHours
import uniffi.mtrx.QuietListener
import uniffi.mtrx.QuietState
import uniffi.mtrx.Session
import uniffi.mtrx.TaskHandle

/**
 * Pausing notifications and quiet hours, on all our devices with Mtrx, as
 * the library tells them ([state], null until it did). The library
 * decides on each push: while quiet it gives nothing to show. Changes
 * show at once; one that fails goes to [failed]. The [SignedInSession]
 * calls [start] and [close]; changes run in [scope].
 */
class QuietModel(
    private val session: Session,
    private val scope: CoroutineScope,
    private val failed: (String) -> Unit,
) {
    private val _state = MutableStateFlow<QuietState?>(null)
    val state: StateFlow<QuietState?> = _state.asStateFlow()

    private var subscription: TaskHandle? = null

    fun start() {
        subscription = session.subscribeQuiet(object : QuietListener {
            // On a library thread.
            override fun onUpdate(state: QuietState) {
                _state.value = state
            }
        })
    }

    fun close() {
        subscription?.let {
            it.cancel()
            it.close()
        }
        subscription = null
    }

    /** The pause menu, its titles as of now: read when it shows. */
    fun pauseMenu(): List<PauseMenuItem> = session.pauseMenu()

    fun pause(pause: PauseFor): Job = change("pause notifications") { session.pauseNotifications(pause) }

    fun resume(): Job = change("resume notifications") { session.resumeNotifications() }

    fun setHours(hours: QuietHours): Job = change("change quiet hours") { session.setQuietHours(hours) }

    fun setMentionsThrough(on: Boolean): Job = change("change what gets through") { session.setMentionsThrough(on) }

    private fun change(what: String, call: suspend () -> Unit): Job = scope.launch {
        try {
            call()
        } catch (e: MtrxException) {
            describe(e)?.let { failed("Could not $what: $it") }
        }
    }
}
