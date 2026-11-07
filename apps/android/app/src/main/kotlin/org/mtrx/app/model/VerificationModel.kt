package org.mtrx.app.model

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.mtrx.MtrxException
import uniffi.mtrx.TaskHandle
import uniffi.mtrx.Verification
import uniffi.mtrx.VerificationListener
import uniffi.mtrx.VerificationState

/**
 * An emoji verification of this device with another of ours, for the
 * verification screen: one we ask for ([request]) or one we received and
 * accept ([accept]). Holds the [Verification] and its subscription, which
 * must live until it ends (dropping them stops following the protocol), so
 * it belongs to the [SignedInSession], not to a screen; calls run in
 * [scope] (the account's, so that the cancel at [close] still goes out).
 * Methods are called on the main thread; [close] when done.
 */
class VerificationModel private constructor(
    /** We asked (the other device accepts) rather than accepted. */
    val outgoing: Boolean,
    private val scope: CoroutineScope,
) {
    private val _state = MutableStateFlow<VerificationState>(VerificationState.Waiting)
    val state: StateFlow<VerificationState> = _state.asStateFlow()

    /** A call in progress (starting, confirming, cancelling): buttons wait. */
    private val _busy = MutableStateFlow(false)
    val busy: StateFlow<Boolean> = _busy.asStateFlow()

    /** The last call that failed. */
    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    private var verification: Verification? = null
    private var subscription: TaskHandle? = null
    private var starting: Job? = null
    private var closed = false

    /** Verified or cancelled: nothing more happens. */
    val ended: Boolean
        get() = isEnded(_state.value)

    fun confirm() = call { it.confirm() }

    fun mismatch() = call { it.mismatch() }

    fun cancel() = call { it.cancel() }

    /** Stop following it; cancels it first unless it ended. */
    fun close() {
        if (closed) return
        closed = true
        starting?.cancel()
        subscription?.let {
            it.cancel()
            it.close()
        }
        subscription = null
        val current = verification ?: return
        verification = null
        if (ended) {
            current.close()
        } else {
            // The other device learns at once, instead of timing out.
            scope.launch {
                try {
                    current.cancel()
                } catch (e: MtrxException) {
                    // Ended meanwhile, or offline: it times out on the other device.
                } finally {
                    current.close()
                }
            }
        }
    }

    private fun start(action: suspend () -> Verification) {
        _busy.value = true
        starting = scope.launch {
            val started = try {
                action()
            } catch (e: MtrxException) {
                _state.value = VerificationState.Cancelled(describe(e) ?: "")
                return@launch
            } finally {
                _busy.value = false
            }
            if (closed) {
                started.close()
                return@launch
            }
            verification = started
            follow(started)
        }
    }

    private fun follow(started: Verification) {
        subscription = started.subscribe(object : VerificationListener {
            // On a library thread; the whole state, now and after each change.
            override fun onUpdate(state: VerificationState) {
                _state.value = state
            }
        })
    }

    private fun call(action: suspend (Verification) -> Unit) {
        val current = verification ?: return
        if (_busy.value || ended) return
        _busy.value = true
        _error.value = null
        scope.launch {
            try {
                action(current)
            } catch (e: MtrxException) {
                _error.value = describe(e)
            } finally {
                _busy.value = false
            }
        }
    }

    companion object {
        /** Ask our other devices ([open] is `Session.requestSelfVerification`). */
        fun request(scope: CoroutineScope, open: suspend () -> Verification): VerificationModel =
            VerificationModel(outgoing = true, scope).also { it.start(open) }

        /** Accept a request another of our devices sent. */
        fun accept(scope: CoroutineScope, request: Verification): VerificationModel =
            VerificationModel(outgoing = false, scope).also { model ->
                model.start {
                    try {
                        request.accept()
                    } catch (e: Throwable) {
                        request.close()
                        throw e
                    }
                    request
                }
            }

        fun isEnded(state: VerificationState): Boolean =
            state is VerificationState.Done || state is VerificationState.Cancelled
    }
}
