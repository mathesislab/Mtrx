package org.mtrx.app.model

import java.util.concurrent.atomic.AtomicReference
import org.mtrx.app.rooms.NoRooms
import org.mtrx.app.rooms.SectionLayouts
import org.mtrx.app.rooms.SectionStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.mtrx.app.rooms.SpaceChildrenCache
import uniffi.mtrx.ConnectionListener
import uniffi.mtrx.ConnectionState
import uniffi.mtrx.DeviceInfo
import uniffi.mtrx.EncryptionState
import uniffi.mtrx.EncryptionStateListener
import uniffi.mtrx.MtrxException
import uniffi.mtrx.MtrxInterface
import uniffi.mtrx.OAuthLogin
import uniffi.mtrx.RecoveryListener
import uniffi.mtrx.RecoveryStep
import uniffi.mtrx.RoomLists
import uniffi.mtrx.RoomListsInterface
import uniffi.mtrx.RoomListsListener
import uniffi.mtrx.Session
import uniffi.mtrx.TaskHandle
import uniffi.mtrx.Verification
import uniffi.mtrx.VerificationRequestListener

sealed interface AccountState {
    /** Restoring the saved session. */
    data object Loading : AccountState

    data class SignedOut(val error: String? = null) : AccountState

    data class SignedIn(val session: Session, val images: Images, val presence: PresenceModel) : AccountState
}

/** A browser sign-in, from the sign-in page to the redirect. */
sealed interface BrowserLogin {
    data object Idle : BrowserLogin

    /** The sign-in page is open in the browser. */
    data object Waiting : BrowserLogin

    /** The browser returned; completing the sign-in. */
    data object Finishing : BrowserLogin

    data class Failed(val message: String) : BrowserLogin
}

/**
 * The app's one account: signs in, restores and signs out the session, syncs
 * it while the app is in the foreground, and keeps its room list,
 * connection and encryption state as flows. Recovery and verification run
 * here too, so leaving their screens does not stop them. Lives as long as
 * the process; [scope] runs on the main thread. Listener callbacks arrive on
 * the library's threads and only set flows or launch into [scope]. The
 * space selected on the Spaces tab is kept in [selectedSpace] until
 * sign-out; the order of the spaces' rooms in [roomSortStore]. [push] is
 * told when a session opens and before it closes; [notifications] are
 * cleared as rooms are opened or read. Each session's [PresenceModel] reads
 * the direct chats' presence and tells the library what the app does
 * (which sets ours), as [sharePresence] says; [log] has what goes
 * wrong. How this phone arranges the spaces' sections is kept in
 * [sectionStore] (the device's, as the rooms' order).
 */
class Account(
    private val mtrx: MtrxInterface,
    private val scope: CoroutineScope,
    selectedSpace: SelectedSpaceStore,
    roomSortStore: RoomSortStore,
    private val push: PushRegistration,
    private val notifications: RoomNotifications,
    sharePresenceStore: SharePresenceStore,
    sectionStore: SectionStore,
    private val log: Logger = AndroidLog,
) {
    private val _state = MutableStateFlow<AccountState>(AccountState.Loading)
    val state: StateFlow<AccountState> = _state.asStateFlow()

    private val _rooms = MutableStateFlow<RoomListsInterface>(NoRooms)
    val rooms: StateFlow<RoomListsInterface> = _rooms.asStateFlow()

    /** The space selected on the Spaces tab. */
    val spaceSelection = SpaceSelection(selectedSpace, rooms, scope)

    /** The order of a space's rooms and direct messages. */
    val roomSort = RoomSortChoice(roomSortStore, scope)

    /** "Share my presence". */
    val sharePresence = SharePresence(sharePresenceStore, scope)

    /** How this phone arranges each space's sections. */
    val sections = SectionLayouts(sectionStore, scope)

    /** The last list of each space's rooms, for its overview. */
    val spaceChildren = SpaceChildrenCache()

    private val _connection = MutableStateFlow(ConnectionState.ONLINE)
    val connection: StateFlow<ConnectionState> = _connection.asStateFlow()

    private val _browserLogin = MutableStateFlow<BrowserLogin>(BrowserLogin.Idle)
    val browserLogin: StateFlow<BrowserLogin> = _browserLogin.asStateFlow()

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

    /** A room to open, from a tapped notification; cleared once shown. */
    private val _roomRequest = MutableStateFlow<String?>(null)
    val roomRequest: StateFlow<String?> = _roomRequest.asStateFlow()

    // The room on screen: read by pushes.
    @Volatile private var openRoom: String? = null

    /** Errors of background work, to show briefly. */
    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 8)
    val messages: SharedFlow<String> = _messages.asSharedFlow()

    // The session, its subscriptions and its sync, changed under the lock only.
    private val lock = Mutex()
    private var session: Session? = null
    private var subscriptions: List<TaskHandle> = emptyList()
    private var encryptionSubscription: TaskHandle? = null
    private var presence: PresenceModel? = null
    private var syncing = false

    // Set at once, followed by sync under the lock; read by pushes too.
    @Volatile private var foreground = false

    // On the main thread.
    private var recoveryJob: Job? = null

    private val pendingLogin = AtomicReference<OAuthLogin?>(null)

    /** Restore the saved session, if any. Call once. */
    fun start(): Job = scope.launch {
        val restored = try {
            mtrx.restore()
        } catch (e: MtrxException) {
            _state.value = AccountState.SignedOut(describe(e))
            return@launch
        }
        if (restored == null) {
            _state.value = AccountState.SignedOut()
        } else {
            lock.withLock { open(restored) }
        }
    }

    /** The app came to the foreground (sync) or went to the background (stop). */
    fun setForeground(value: Boolean): Job {
        foreground = value
        return scope.launch {
            lock.withLock { updateSync() }
        }
    }

    /** The room open on screen while the app is in the foreground; from any thread. */
    fun foregroundRoom(): String? = if (foreground) openRoom else null

    /** The room screen shows [roomId]: its notification goes. */
    fun roomOpened(roomId: String) {
        openRoom = roomId
        notifications.clearRoom(roomId)
    }

    /** The room screen left [roomId]. */
    fun roomClosed(roomId: String) {
        if (openRoom == roomId) openRoom = null
    }

    /** [roomId] was marked read: its notification goes. */
    fun roomRead(roomId: String) {
        notifications.clearRoom(roomId)
    }

    /** A touch or key press in the app: we are not idle. */
    fun input() {
        presence?.input()
    }

    /** Turn "Share my presence" on or off; the session's presence follows at once. */
    fun setSharePresence(on: Boolean) {
        if (on == sharePresence.on.value) return
        sharePresence.set(on)
        presence?.sharingChanged(on)
    }

    /** Open [roomId] (a notification was tapped); signed in, the navigation does. */
    fun requestRoom(roomId: String) {
        _roomRequest.value = roomId
    }

    /** The navigation opened the requested room. */
    fun roomRequestHandled() {
        _roomRequest.value = null
    }

    /** Keep the Notes to self at the top, or not: the account's choice, for all its devices. */
    fun setNotesAtTop(on: Boolean): Job = scope.launch {
        val current = session ?: return@launch
        try {
            current.setNotesAtTop(on)
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
            it.join(roomId, emptyList())
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
            it.room(roomId).use { room -> room.leave() }
            null
        } catch (e: MtrxException) {
            "Could not leave $name: ${describe(e)}"
        }
    }

    // [change] returns its error, if any.
    private fun membership(onDone: (Boolean) -> Unit, change: suspend (Session) -> String?): Job = scope.launch {
        val current = session ?: return@launch
        val error = change(current)
        if (error != null) _messages.tryEmit(error)
        onDone(error == null)
    }

    /** Throws [MtrxException]. */
    suspend fun supportsOauth(homeserver: String): Boolean = mtrx.supportsOauth(homeserver)

    /** Sign in with a password. Throws [MtrxException]. */
    suspend fun login(homeserver: String, user: String, password: String) {
        val signedIn = mtrx.login(homeserver, user, password)
        lock.withLock { open(signedIn) }
    }

    /**
     * Start a browser sign-in and return the page to open; the redirect goes
     * to [finishBrowserLogin]. Throws [MtrxException].
     */
    suspend fun startBrowserLogin(homeserver: String): String {
        val login = mtrx.startOauthLogin(homeserver)
        pendingLogin.getAndSet(login)?.close()
        _browserLogin.value = BrowserLogin.Waiting
        return login.url()
    }

    /** Give up the browser sign-in, or forget its error. */
    fun cancelBrowserLogin() {
        pendingLogin.getAndSet(null)?.close()
        _browserLogin.value = BrowserLogin.Idle
    }

    /** The browser returned to [redirectUrl]; ignored without a sign-in waiting. */
    fun finishBrowserLogin(redirectUrl: String) {
        val login = pendingLogin.getAndSet(null) ?: return
        _browserLogin.value = BrowserLogin.Finishing
        scope.launch {
            try {
                val signedIn = login.finish(redirectUrl)
                lock.withLock { open(signedIn) }
                _browserLogin.value = BrowserLogin.Idle
            } catch (e: MtrxException) {
                _browserLogin.value = describe(e)?.let { BrowserLogin.Failed(it) } ?: BrowserLogin.Idle
            } finally {
                login.close()
            }
        }
    }

    /** Sign out on the server and delete the account's data on this device. */
    fun signOut(): Job = scope.launch {
        lock.withLock {
            val current = session ?: return@withLock
            presence?.signOut()
            stopSync()
            // No more pushes to this device: on the server, then at the distributor.
            push.closing(current)
            try {
                current.logout()
            } catch (e: MtrxException) {
                _messages.tryEmit("Could not sign out: ${describe(e)}")
                updateSync()
                push.opened(current)
                return@withLock
            }
            close()
        }
    }

    /**
     * After [ConnectionState.SIGNED_OUT] (the server no longer accepts the
     * session): delete its data, to sign in again.
     */
    fun forget(): Job = scope.launch {
        lock.withLock {
            val current = session ?: return@withLock
            // The server no longer accepts the session: nothing to set.
            presence?.close()
            stopSync()
            push.closing(current)
            try {
                current.forget()
            } catch (e: MtrxException) {
                _messages.tryEmit("Could not delete the session's data: ${describe(e)}")
            }
            close()
        }
    }

    /** Get our devices into [devices] (for the encryption settings). */
    fun loadDevices(): Job = scope.launch {
        val current = session ?: return@launch
        val list = try {
            current.devices()
        } catch (e: MtrxException) {
            return@launch
        }
        if (session === current) _devices.value = list
    }

    /**
     * Unlock encryption with the recovery key; [recovery] follows its steps.
     * Ignored while one runs. [encryption] follows as the library reports it.
     */
    fun recover(recoveryKey: String) {
        val current = session ?: return
        if (_recovery.value is Recovery.Running) return
        _recovery.value = Recovery.Running(step = null)
        recoveryJob = scope.launch {
            try {
                current.recover(recoveryKey, object : RecoveryListener {
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
        val current = session ?: return
        replaceVerification(VerificationModel.request(scope) {
            current.requestSelfVerification()
        })
    }

    /** Accept [verificationRequest]; then [verification] has it. */
    fun acceptVerificationRequest() {
        val request = _verificationRequest.value ?: return
        _verificationRequest.value = null
        replaceVerification(VerificationModel.accept(scope, request))
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
    private fun onVerificationRequest(current: Session, request: Verification) {
        val busy = _verification.value?.ended == false
        // Verifying other people is not in this app yet.
        if (session !== current || request.otherUser() != current.userId() || busy) {
            decline(request)
            return
        }
        // A newer request replaces one not answered yet.
        _verificationRequest.value?.let { decline(it) }
        _verificationRequest.value = request
    }

    private fun decline(request: Verification) {
        scope.launch {
            try {
                request.cancel()
            } catch (e: MtrxException) {
                // Already ended.
            } finally {
                request.close()
            }
        }
    }

    // Once sync has started (before it, the state is not known).
    private fun followEncryption(current: Session): TaskHandle =
        current.subscribeEncryptionState(object : EncryptionStateListener {
            // On a library thread: the state now and after every change.
            override fun onUpdate(state: EncryptionState) {
                scope.launch { if (session === current) _encryption.value = state }
            }
        })

    // Under the lock.
    private suspend fun open(signedIn: Session) {
        check(session == null) { "already signed in" }
        session = signedIn
        _connection.value = signedIn.connection()
        val handles = mutableListOf<TaskHandle>()
        handles += signedIn.subscribeConnection(object : ConnectionListener {
            override fun onUpdate(state: ConnectionState) {
                _connection.value = state
            }
        })
        try {
            handles += signedIn.subscribeRoomLists(object : RoomListsListener {
                override fun onUpdate(lists: RoomLists) {
                    val previous = _rooms.value
                    _rooms.value = lists
                    roomsRead(previous, lists).forEach(notifications::clearRoom)
                }
            })
        } catch (e: MtrxException) {
            _messages.tryEmit("No room list: ${describe(e)}")
        }
        handles += signedIn.subscribeVerificationRequests(object : VerificationRequestListener {
            // On a library thread.
            override fun onRequest(verification: Verification) {
                scope.launch { onVerificationRequest(signedIn, verification) }
            }
        })
        subscriptions = handles
        val sessionPresence = PresenceModel(signedIn, signedIn.userId(), rooms, sharePresence, scope, log)
        presence = sessionPresence
        _state.value = AccountState.SignedIn(signedIn, Images(signedIn), sessionPresence)
        updateSync()
        push.opened(signedIn)
    }

    // Under the lock.
    private suspend fun updateSync() {
        val current = session
        if (current != null && foreground && !syncing) {
            // Not sharing, offline before sync can say otherwise.
            presence?.foreground()
            current.startSync()
            syncing = true
            if (encryptionSubscription == null) encryptionSubscription = followEncryption(current)
        } else if ((current == null || !foreground) && syncing) {
            stopSync()
        }
    }

    // Under the lock. Offline first.
    private suspend fun stopSync() {
        if (syncing) {
            presence?.background()
            session?.stopSync()
            syncing = false
        }
    }

    // Under the lock, after logout or forget.
    private suspend fun close() {
        _state.value = AccountState.SignedOut()
        (subscriptions + listOfNotNull(encryptionSubscription)).forEach {
            it.cancel()
            it.close()
        }
        subscriptions = emptyList()
        encryptionSubscription = null
        presence?.close()
        presence = null
        recoveryJob?.cancel()
        recoveryJob = null
        _recovery.value = Recovery.Idle
        replaceVerification(null)
        declineVerificationRequest()
        _encryption.value = null
        _devices.value = null
        session?.close()
        session = null
        _rooms.value = NoRooms
        _connection.value = ConnectionState.ONLINE
        spaceSelection.clear()
        spaceChildren.clear()
        _roomRequest.value = null
        notifications.clearAll()
    }
}
