package org.mtrx.app.model

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import uniffi.mtrx.MtrxException
import uniffi.mtrx.Presence
import uniffi.mtrx.PresenceActivity
import uniffi.mtrx.PresenceListener
import uniffi.mtrx.RoomKind
import uniffi.mtrx.RoomListsInterface
import uniffi.mtrx.RoomSummary
import uniffi.mtrx.Session
import uniffi.mtrx.TaskHandle

/** The person of a direct chat whose presence shows: not ours (the Notes to self). */
fun directPerson(room: RoomSummary, ownUserId: String): String? =
    room.takeIf { it.kind == RoomKind.DIRECT }?.directUser?.userId?.takeIf { it != ownUserId }

/** The people of the direct chats in [lists], whose presence is read. */
fun directPeople(lists: RoomListsInterface, ownUserId: String): Set<String> =
    lists.rooms().mapNotNullTo(mutableSetOf()) { directPerson(it, ownUserId) }

/** Where "Share my presence" is kept across launches. */
interface SharePresenceStore {
    /** The saved value; null if none, or it cannot be read. */
    suspend fun load(): Boolean?

    suspend fun save(on: Boolean)
}

/**
 * "Share my presence": a device preference, on by default, kept at
 * sign-out, saved in [store]. Used on [scope]'s (main) thread.
 */
class SharePresence(private val store: SharePresenceStore, private val scope: CoroutineScope) {
    private val _on = MutableStateFlow(true)
    val on: StateFlow<Boolean> = _on.asStateFlow()

    /** Set before the saved value was read: that one is stale. */
    private var changed = false

    // Saves run in order and each saves the latest value.
    private val saving = Mutex()

    private val loading: Deferred<Unit> = scope.async {
        val saved = store.load()
        if (!changed && saved != null) _on.value = saved
    }

    /** The value, once the saved one is read. */
    suspend fun current(): Boolean {
        loading.await()
        return _on.value
    }

    fun set(on: Boolean) {
        changed = true
        _on.value = on
        scope.launch { saving.withLock { store.save(_on.value) } }
    }
}

/**
 * A session's presence: the people of the direct chats in [rooms] (their
 * presences in [presences], read while the app is in the foreground), and
 * ours, which the library sets from what the app does (as it is told
 * here) while [sharing] is on. Once the server does not share presence
 * ([available] false), all of it is hidden and nothing more is read or
 * told. The [SignedInSession] calls [foreground] and [background] around
 * sync and [close] when it closes; the [Account] calls [signOut] before
 * logging out and [sharingChanged] for the switch; touches and keys call
 * [input]. Used on [parent]'s (main) thread; listener callbacks hop there.
 * What goes wrong goes to [log].
 */
@OptIn(FlowPreview::class)
class PresenceModel(
    private val session: Session,
    private val ownUserId: String,
    rooms: StateFlow<RoomListsInterface>,
    private val sharing: SharePresence,
    parent: CoroutineScope,
    private val log: Logger,
) {
    private val scope = CoroutineScope(parent.coroutineContext + SupervisorJob(parent.coroutineContext[Job]))

    /** The last presences read, by user ID; a user missing shows nothing. */
    private val _presences = MutableStateFlow<Map<String, Presence>>(emptyMap())
    val presences: StateFlow<Map<String, Presence>> = _presences.asStateFlow()

    /** The server shares presence (until it says it does not). */
    private val _available = MutableStateFlow(true)
    val available: StateFlow<Boolean> = _available.asStateFlow()

    private var foreground = false
    private var started = false
    private var closed = false

    /** The users to read; null before the first loaded room list. */
    private var users: Set<String>? = null
    private var subscription: TaskHandle? = null

    // Updates of an earlier subscription are dropped.
    private var generation = 0

    /** The last call telling the library what the app does. */
    private var lastCall: Job? = null

    init {
        scope.launch {
            rooms.filter { it.loaded() }
                .map { directPeople(it, ownUserId) }
                .distinctUntilChanged()
                // Room lists come in bursts at first: subscribe for where they settle.
                .debounce(FOLLOW_DEBOUNCE_MS)
                .collect { follow(it) }
        }
    }

    /**
     * The app comes to the foreground (also at start and sign-in), before
     * sync starts: read others' presence again, and tell the library (the
     * session's first time after "Share my presence"). Not sharing, returns
     * once we are offline, as sync's requests would say online.
     */
    suspend fun foreground() {
        if (closed) return
        foreground = true
        val first = !started
        started = true
        if (!_available.value) return
        if (users != null) subscribe()
        val told = tell {
            if (first) session.setPresenceSharing(sharing.current())
            session.presenceActivity(PresenceActivity.FOREGROUND)
        }
        if (!sharing.current()) told.join()
    }

    /**
     * The app goes to the background, before sync stops: stop reading
     * others' presence, and return once we are offline.
     */
    suspend fun background() {
        if (closed) return
        foreground = false
        stopReading()
        if (_available.value) tell { session.presenceActivity(PresenceActivity.BACKGROUND) }.join()
    }

    /** Signing out, before logging out: return once we are offline. */
    suspend fun signOut() {
        if (closed || !_available.value) return
        tell { session.presenceActivity(PresenceActivity.SIGN_OUT) }.join()
    }

    /** "Share my presence" changed: offline at once, or online if in the foreground. */
    fun sharingChanged(on: Boolean) {
        if (closed || !_available.value) return
        tell { session.setPresenceSharing(on) }
    }

    /** A touch or key press: online again if idle. */
    fun input() {
        if (!foreground || closed || !_available.value) return
        tell { session.presenceActivity(PresenceActivity.INPUT) }
    }

    /** Read [userId]'s presence now (their info sheet opens); a failure keeps what is known. */
    suspend fun refresh(userId: String) {
        if (closed || !_available.value) return
        try {
            val presence = session.presence(userId)
            if (!closed && _available.value) _presences.value += userId to presence
        } catch (e: MtrxException) {
            failed(e, "Reading $userId's presence")
        }
    }

    /** The session ends: no more reading or setting. */
    fun close() {
        if (closed) return
        closed = true
        foreground = false
        stopReading()
        scope.cancel()
    }

    private fun follow(people: Set<String>) {
        if (closed || !_available.value || people == users) return
        users = people
        if (foreground) subscribe()
    }

    /** Subscribe for [users] anew, then end the subscription before. */
    private fun subscribe() {
        val previous = subscription
        subscription = null
        val mine = ++generation
        val people = users.orEmpty()
        if (people.isEmpty()) {
            _presences.value = emptyMap()
        } else {
            try {
                subscription = session.subscribePresence(people.sorted(), object : PresenceListener {
                    // On a library thread.
                    override fun onUpdate(presences: Map<String, Presence>) {
                        scope.launch { if (mine == generation && _available.value) _presences.value = presences }
                    }

                    override fun onUnavailable() {
                        scope.launch { hide() }
                    }
                })
            } catch (e: MtrxException) {
                failed(e, "Subscribing to presence")
            }
        }
        previous?.let {
            it.cancel()
            it.close()
        }
    }

    private fun stopReading() {
        generation++
        subscription?.let {
            it.cancel()
            it.close()
        }
        subscription = null
    }

    /** The server does not share presence: hide it for the rest of the session. */
    private fun hide() {
        if (!_available.value) return
        log.info(TAG, "The server does not share presence")
        _available.value = false
        _presences.value = emptyMap()
        users = null
        stopReading()
    }

    private fun failed(error: MtrxException, doing: String) {
        if (error is MtrxException.PresenceUnavailable) {
            hide()
        } else {
            log.warn(TAG, "$doing failed: ${describe(error)}")
        }
    }

    /**
     * Tell the library with [call], after the call before (the library
     * counts each as of when it gets it), unless that hid presence;
     * failures are logged (or hide presence).
     */
    private fun tell(call: suspend () -> Unit): Job {
        val previous = lastCall
        return scope.launch {
            previous?.join()
            if (!_available.value) return@launch
            try {
                call()
            } catch (e: MtrxException) {
                failed(e, "Setting our presence")
            }
        }.also { lastCall = it }
    }

    companion object {
        /** The room list's direct chats must settle this long before subscribing anew. */
        const val FOLLOW_DEBOUNCE_MS = 1000L

        private const val TAG = "Presence"
    }
}
