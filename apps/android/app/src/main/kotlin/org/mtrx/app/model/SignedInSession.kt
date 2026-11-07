package org.mtrx.app.model

import org.mtrx.app.rooms.NoRooms
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.mtrx.ConnectionListener
import uniffi.mtrx.ConnectionState
import uniffi.mtrx.DeviceInfo
import uniffi.mtrx.EncryptionState
import uniffi.mtrx.EncryptionStateListener
import uniffi.mtrx.MtrxException
import uniffi.mtrx.RecoveryListener
import uniffi.mtrx.RecoveryStep
import uniffi.mtrx.RoomLists
import uniffi.mtrx.RoomListsInterface
import uniffi.mtrx.RoomListsListener
import uniffi.mtrx.Session
import uniffi.mtrx.TaskHandle
import uniffi.mtrx.Verification
import uniffi.mtrx.VerificationRequestListener

/** What the settings show of the session. */
data class SessionInfo(val userId: String, val homeserver: String, val deviceId: String)

/**
 * A signed-in session and what lives with it: its room list, connection,
 * encryption, devices, recovery, verification and presence, and the space
 * selected on the Spaces tab. The [Account] opens it ([open]), starts and
 * stops its sync, and closes it at sign-out; nothing of it outlives
 * [close]. Its work runs in a scope of its own, cancelled at [close];
 * verifications and declined requests run in [parent], so that a cancel at
 * [close] still reaches the other device. Used on [parent]'s (main)
 * thread; listener callbacks arrive on the library's threads and only set
 * flows or launch. Rooms read elsewhere lose their [notifications];
 * [presence] reads and tells as [sharePresence] says; [log] has what goes
 * wrong.
 */
class SignedInSession private constructor(
    val session: Session,
    private val parent: CoroutineScope,
    selectedSpace: SelectedSpaceStore,
    sharePresence: SharePresence,
    private val notifications: RoomNotifications,
    private val log: Logger,
) : AutoCloseable {
    private val scope = CoroutineScope(parent.coroutineContext + SupervisorJob(parent.coroutineContext[Job]))

    val info = SessionInfo(session.userId(), session.homeserver(), session.deviceId())

    /** Avatars and message images, decoded. */
    val images = Images(session)

    private val _rooms = MutableStateFlow<RoomListsInterface>(NoRooms)
    val rooms: StateFlow<RoomListsInterface> = _rooms.asStateFlow()

    val presence = PresenceModel(session, info.userId, rooms, sharePresence, scope, log)

    /** The space selected on the Spaces tab. */
    val spaceSelection = SpaceSelection(selectedSpace, rooms, scope)

    private val _connection = MutableStateFlow(session.connection())
    val connection: StateFlow<ConnectionState> = _connection.asStateFlow()

    /**
     * Whether this device can read encrypted messages, and whether keys are
     * arriving: as the library reports it once sync started; null before.
     */
    private val _encryption = MutableStateFlow<EncryptionState?>(null)
    val encryption: StateFlow<EncryptionState?> = _encryption.asStateFlow()

    /** Our devices, this one included; null until [loadDevices] gets them. */
    private val _devices = MutableStateFlow<List<DeviceInfo>?>(null)
    val devices: StateFlow<List<DeviceInfo>?> = _devices.asStateFlow()

    private val _recovery = MutableStateFlow<Recovery>(Recovery.Idle)
    val recovery: StateFlow<Recovery> = _recovery.asStateFlow()

    /** The verification on the verification screen, if any. */
    private val _verification = MutableStateFlow<VerificationModel?>(null)
    val verification: StateFlow<VerificationModel?> = _verification.asStateFlow()

    /** A request from another of our devices, to accept or decline. */
    private val _verificationRequest = MutableStateFlow<Verification?>(null)
    val verificationRequest: StateFlow<Verification?> = _verificationRequest.asStateFlow()

    /** Errors of background work, to show briefly. */
    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 8)
    val messages: SharedFlow<String> = _messages.asSharedFlow()

    // The connection, room list and verification requests: from [open] to [close].
    private val subscriptions = mutableListOf<TaskHandle>()

    // From the first sync on (before it, the state is not known).
    private var encryptionSubscription: TaskHandle? = null

    // Changed by the Account, under its lock.
    private var syncing = false

    private var closed = false

    /** Keep the Notes to self at the top, or not: the account's choice, for all its devices. */
    fun setNotesAtTop(on: Boolean): Job = scope.launch {
        try {
            session.setNotesAtTop(on)
        } catch (e: MtrxException) {
            _messages.tryEmit("Could not change Notes at the top: ${describe(e)}")
        }
    }

    /**
     * Join [roomId], or accept its invite; [onDone] says whether it worked
     * (errors go to [messages]).
     */
    fun accept(roomId: String, name: String, onDone: (Boolean) -> Unit = {}): Job = membership(onDone) {
        try {
            session.join(roomId, emptyList())
            null
        } catch (e: MtrxException) {
            "Could not join $name: ${describe(e)}"
        }
    }

    /**
     * Leave [roomId], or decline its invite; its draft stays on this device.
     * [onDone] says whether it worked (errors go to [messages]).
     */
    fun leave(roomId: String, name: String, onDone: (Boolean) -> Unit = {}): Job = membership(onDone) {
        try {
            session.room(roomId).use { room -> room.leave() }
            null
        } catch (e: MtrxException) {
            "Could not leave $name: ${describe(e)}"
        }
    }

    // [change] returns its error, if any.
    private fun membership(onDone: (Boolean) -> Unit, change: suspend () -> String?): Job = scope.launch {
        val error = change()
        if (error != null) _messages.tryEmit(error)
        onDone(error == null)
    }

    /** Get our devices into [devices] (for the encryption settings). */
    fun loadDevices(): Job = scope.launch {
        _devices.value = try {
            session.devices()
        } catch (e: MtrxException) {
            return@launch
        }
    }

    /**
     * Unlock encryption with the recovery key; [recovery] follows its steps.
     * Ignored while one runs. [encryption] follows as the library reports it.
     */
    fun recover(recoveryKey: String) {
        if (_recovery.value is Recovery.Running) return
        _recovery.value = Recovery.Running(step = null)
        scope.launch {
            try {
                session.recover(recoveryKey, object : RecoveryListener {
                    // On a library thread, during the call.
                    override fun onUpdate(step: RecoveryStep) {
                        _recovery.value = Recovery.Running(step)
                    }
                })
            } catch (e: MtrxException) {
                _recovery.value = recoveryFailure(e)
                return@launch
            }
            _recovery.value = Recovery.Done
        }
    }

    /** Forget a recovery's result (failed or done), to show the form again. */
    fun resetRecovery() {
        if (_recovery.value !is Recovery.Running) _recovery.value = Recovery.Idle
    }

    /** Ask our other devices to verify this one; then [verification] has it. */
    fun verifyWithAnotherDevice() {
        replaceVerification(VerificationModel.request(parent) { session.requestSelfVerification() })
    }

    /** Accept [verificationRequest]; then [verification] has it. */
    fun acceptVerificationRequest() {
        val request = _verificationRequest.value ?: return
        _verificationRequest.value = null
        replaceVerification(VerificationModel.accept(parent, request))
    }

    fun declineVerificationRequest() {
        val request = _verificationRequest.value ?: return
        _verificationRequest.value = null
        decline(request)
    }

    /** The verification screen is done with [model]. */
    fun closeVerification(model: VerificationModel) {
        if (_verification.value === model) replaceVerification(null) else model.close()
    }

    private fun replaceVerification(model: VerificationModel?) {
        _verification.value?.close()
        _verification.value = model
    }

    // On the main thread.
    private fun onVerificationRequest(request: Verification) {
        val busy = _verification.value?.ended == false
        // Verifying other people is not in this app yet; one arriving as the
        // session closes is not answered either.
        if (closed || request.otherUser() != info.userId || busy) {
            decline(request)
            return
        }
        // A newer request replaces one not answered yet.
        _verificationRequest.value?.let { decline(it) }
        _verificationRequest.value = request
    }

    private fun decline(request: Verification) {
        parent.launch {
            try {
                request.cancel()
            } catch (e: MtrxException) {
                // Already ended.
            } finally {
                request.close()
            }
        }
    }

    // By the Account, under its lock: sync, our presence first.
    internal suspend fun startSync() {
        if (syncing) return
        // Not sharing, offline before sync can say otherwise.
        presence.foreground()
        session.startSync()
        syncing = true
        if (encryptionSubscription == null) encryptionSubscription = followEncryption()
    }

    // By the Account, under its lock. Offline first.
    internal suspend fun stopSync() {
        if (!syncing) return
        presence.background()
        session.stopSync()
        syncing = false
    }

    // By the Account, under its lock, sync stopped: false (said in
    // [messages]) if the server did not sign us out.
    internal suspend fun logout(): Boolean = try {
        session.logout()
        true
    } catch (e: MtrxException) {
        _messages.tryEmit("Could not sign out: ${describe(e)}")
        false
    }

    // By the Account, under its lock, sync stopped: delete the session's data.
    internal suspend fun forget() {
        try {
            session.forget()
        } catch (e: MtrxException) {
            _messages.tryEmit("Could not delete the session's data: ${describe(e)}")
        }
    }

    private fun followEncryption(): TaskHandle =
        session.subscribeEncryptionState(object : EncryptionStateListener {
            // On a library thread: the state now and after every change.
            override fun onUpdate(state: EncryptionState) {
                _encryption.value = state
            }
        })

    private suspend fun follow() {
        subscriptions += session.subscribeConnection(object : ConnectionListener {
            override fun onUpdate(state: ConnectionState) {
                _connection.value = state
            }
        })
        try {
            subscriptions += session.subscribeRoomLists(object : RoomListsListener {
                override fun onUpdate(lists: RoomLists) {
                    val previous = _rooms.value
                    _rooms.value = lists
                    roomsRead(previous, lists).forEach(notifications::clearRoom)
                }
            })
        } catch (e: MtrxException) {
            log.warn(TAG, "No room list: ${describe(e)}")
        }
        subscriptions += session.subscribeVerificationRequests(object : VerificationRequestListener {
            // On a library thread.
            override fun onRequest(verification: Verification) {
                parent.launch { onVerificationRequest(verification) }
            }
        })
    }

    /** Stop following the session and close it; its work stops, a verification under way is cancelled. */
    override fun close() {
        closed = true
        scope.cancel()
        (subscriptions + listOfNotNull(encryptionSubscription)).forEach {
            it.cancel()
            it.close()
        }
        presence.close()
        _verification.value?.close()
        _verificationRequest.value?.let { decline(it) }
        session.close()
    }

    companion object {
        private const val TAG = "Session"

        /** Open [session]: follow its connection, room list and verification requests. */
        suspend fun open(
            session: Session,
            parent: CoroutineScope,
            selectedSpace: SelectedSpaceStore,
            sharePresence: SharePresence,
            notifications: RoomNotifications,
            log: Logger,
        ): SignedInSession =
            SignedInSession(session, parent, selectedSpace, sharePresence, notifications, log).also { it.follow() }
    }
}
