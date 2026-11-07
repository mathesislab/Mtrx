package org.mtrx.app.model

import java.util.concurrent.atomic.AtomicReference
import org.mtrx.app.rooms.SectionLayouts
import org.mtrx.app.rooms.SectionPrefs
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import uniffi.mtrx.ConnectionState
import uniffi.mtrx.MtrxException
import uniffi.mtrx.MtrxInterface
import uniffi.mtrx.OAuthLogin
import uniffi.mtrx.Session
import uniffi.mtrx.SpaceKey

sealed interface AccountState {
    /** Restoring the saved session. */
    data object Loading : AccountState

    data class SignedOut(val error: String? = null) : AccountState

    data class SignedIn(val session: SignedInSession) : AccountState
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
 * The app's one account: signs in, restores and signs out the session
 * ([SignedInSession], in [state]), and syncs it while the app is in the
 * foreground. Lives as long as the process; [scope] runs on the main
 * thread. Each session keeps the space selected on the Spaces tab in
 * [selectedSpaceStore] until sign-out. The device's preferences outlive
 * the sessions: the order of the spaces' rooms ([roomSortStore]), "Share
 * my presence" ([sharePresenceStore], which each session's presence
 * follows) and how this phone arranges the spaces' sections
 * ([sectionStore]). [push] is told when a session opens and before it
 * closes; [notifications] are cleared as rooms are opened or read; [log]
 * has what goes wrong.
 */
class Account(
    private val mtrx: MtrxInterface,
    private val scope: CoroutineScope,
    selectedSpaceStore: PreferenceStore<SpaceKey?>,
    roomSortStore: PreferenceStore<RoomSort>,
    private val push: PushRegistration,
    private val notifications: RoomNotifications,
    sharePresenceStore: PreferenceStore<Boolean>,
    sectionStore: PreferenceStore<Map<String, SectionPrefs>>,
    private val log: Logger = AndroidLog,
) {
    // Changed under the lock only.
    private val _state = MutableStateFlow<AccountState>(AccountState.Loading)
    val state: StateFlow<AccountState> = _state.asStateFlow()

    /** The order of a space's rooms and direct messages. */
    val roomSort = Preference(roomSortStore, RoomSort.DEFAULT, scope)

    /** "Share my presence": on by default, kept at sign-out. */
    val sharePresence = Preference(sharePresenceStore, true, scope)

    /** How this phone arranges each space's sections. */
    val sections = SectionLayouts(Preference(sectionStore, emptyMap(), scope))

    /** The space selected on the Spaces tab; forgotten at sign-out. */
    private val selectedSpace = Preference(selectedSpaceStore, null, scope)

    private val _browserLogin = MutableStateFlow<BrowserLogin>(BrowserLogin.Idle)
    val browserLogin: StateFlow<BrowserLogin> = _browserLogin.asStateFlow()

    /** A room to open, from a tapped notification; cleared once shown. */
    private val _roomRequest = MutableStateFlow<String?>(null)
    val roomRequest: StateFlow<String?> = _roomRequest.asStateFlow()

    // The room on screen: read by pushes.
    @Volatile private var openRoom: String? = null

    // The session is opened, closed and synced under the lock only.
    private val lock = Mutex()

    // Set at once, followed by sync under the lock; read by pushes too.
    @Volatile private var foreground = false

    private val pendingLogin = AtomicReference<OAuthLogin?>(null)

    /** The open session, if signed in. */
    private val current: SignedInSession?
        get() = (_state.value as? AccountState.SignedIn)?.session

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
        current?.presence?.input()
    }

    /** Turn "Share my presence" on or off; the session's presence follows at once. */
    fun setSharePresence(on: Boolean) {
        if (on == sharePresence.value.value) return
        sharePresence.set(on)
        current?.presence?.sharingChanged(on)
    }

    /** Open [roomId] (a notification was tapped); signed in, the navigation does. */
    fun requestRoom(roomId: String) {
        _roomRequest.value = roomId
    }

    /** The navigation opened the requested room. */
    fun roomRequestHandled() {
        _roomRequest.value = null
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
            val signedIn = current ?: return@withLock
            signedIn.presence.signOut()
            signedIn.stopSync()
            // No more pushes to this device: on the server, then at the distributor.
            push.closing(signedIn.session)
            if (!signedIn.logout()) {
                updateSync()
                push.opened(signedIn.session)
                return@withLock
            }
            close(signedIn)
        }
    }

    /**
     * After [ConnectionState.SIGNED_OUT] (the server no longer accepts the
     * session): delete its data, to sign in again.
     */
    fun forget(): Job = scope.launch {
        lock.withLock {
            val signedIn = current ?: return@withLock
            // The server no longer accepts the session: nothing to set.
            signedIn.presence.close()
            signedIn.stopSync()
            push.closing(signedIn.session)
            signedIn.forget()
            close(signedIn)
        }
    }

    // Under the lock.
    private suspend fun open(session: Session) {
        check(current == null) { "already signed in" }
        val signedIn = SignedInSession.open(session, scope, selectedSpace, sharePresence, notifications, log)
        _state.value = AccountState.SignedIn(signedIn)
        updateSync()
        push.opened(session)
    }

    // Under the lock.
    private suspend fun updateSync() {
        val signedIn = current ?: return
        if (foreground) signedIn.startSync() else signedIn.stopSync()
    }

    // Under the lock, after logout or forget.
    private suspend fun close(signedIn: SignedInSession) {
        _state.value = AccountState.SignedOut()
        signedIn.close()
        selectedSpace.set(null)
        _roomRequest.value = null
        notifications.clearAll()
    }
}
