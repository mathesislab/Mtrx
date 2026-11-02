package org.mtrx.app.model

import java.time.Duration
import java.time.Instant
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
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
import uniffi.mtrx.PresenceListener
import uniffi.mtrx.PresenceState
import uniffi.mtrx.RoomKind
import uniffi.mtrx.RoomListsInterface
import uniffi.mtrx.RoomSummary
import uniffi.mtrx.Session
import uniffi.mtrx.TaskHandle

/** The reason of the `Other` error from a server that does not share presence. */
const val PRESENCE_UNAVAILABLE = "This server does not share presence"

/** The dot on an avatar: green online, amber idle (unavailable). */
enum class PresenceDot(val label: String) {
    Online("Online"),
    Idle("Idle"),
}

/** The dot for [presence]; none offline or unknown. */
fun presenceDot(presence: Presence?): PresenceDot? = when (presence?.state) {
    PresenceState.ONLINE -> PresenceDot.Online
    PresenceState.UNAVAILABLE -> PresenceDot.Idle
    PresenceState.OFFLINE, null -> null
}

/**
 * The line under a person's name: the status message, else "Active now" or
 * "Idle", else when they were last active ([ago] says how long before
 * [now], e.g. "2 hr. ago"; under a minute "just now"); null for nothing to
 * say.
 */
fun presenceLine(presence: Presence?, now: Instant, ago: (Instant, Instant) -> String): String? {
    if (presence == null) return null
    presence.statusMessage?.takeIf { it.isNotBlank() }?.let { return it }
    return when (presence.state) {
        PresenceState.ONLINE -> "Active now"
        PresenceState.UNAVAILABLE -> "Idle"
        PresenceState.OFFLINE -> presence.lastActive?.let { last ->
            if (Duration.between(last, now) < Duration.ofMinutes(1)) "Last active just now" else "Last active ${ago(last, now)}"
        }
    }
}

/** The person of a direct chat whose presence shows: not ours (the Notes to self). */
fun directPerson(room: RoomSummary, ownUserId: String): String? =
    room.takeIf { it.kind == RoomKind.DIRECT }?.directUserId?.takeIf { it != ownUserId }

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
 * ours, set from the app's lifecycle while [sharing] is on. Once the server
 * does not share presence ([available] false), all of it is hidden and
 * nothing more is read or set. The `Account` calls [foreground] and
 * [background] around sync, [sharingChanged] for the switch, [input] for
 * touches and keys, and [close] at sign-out. Used on [parent]'s (main)
 * thread; listener callbacks hop there. [clock] counts milliseconds;
 * what goes wrong goes to [log].
 */
@OptIn(FlowPreview::class)
class PresenceModel(
    private val session: Session,
    private val ownUserId: String,
    rooms: StateFlow<RoomListsInterface>,
    private val sharing: SharePresence,
    parent: CoroutineScope,
    private val clock: () -> Long,
    private val log: Logger,
    private val idleAfter: Long = IDLE_AFTER_MS,
) {
    private val scope = CoroutineScope(parent.coroutineContext + SupervisorJob(parent.coroutineContext[Job]))

    /** The last presences read, by user ID; a user missing shows nothing. */
    private val _presences = MutableStateFlow<Map<String, Presence>>(emptyMap())
    val presences: StateFlow<Map<String, Presence>> = _presences.asStateFlow()

    /** The server shares presence (until it says it does not). */
    private val _available = MutableStateFlow(true)
    val available: StateFlow<Boolean> = _available.asStateFlow()

    /** Ours, as last set here. */
    var own: PresenceState? = null
        private set

    private var foreground = false
    private var started = false
    private var closed = false

    /** The users to read; null before the first loaded room list. */
    private var users: Set<String>? = null
    private var subscription: TaskHandle? = null

    // Updates of an earlier subscription are dropped.
    private var generation = 0

    /** The last setPresence call: each waits for the one before. */
    private var lastCall: Job? = null
    private var lastInput = clock()

    /** Waits for [idleAfter] without input, while online. */
    private var idleTimer: Job? = null

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
     * sync starts: read others' presence again and, sharing ours, be
     * online. Not sharing, the session's first call returns after setting
     * us offline, so that sync does not report us online either.
     */
    suspend fun foreground() {
        if (closed) return
        foreground = true
        val first = !started
        started = true
        if (!_available.value) return
        if (users != null) subscribe()
        if (sharing.current()) {
            beActive()
        } else if (first) {
            setOwn(PresenceState.OFFLINE).join()
        }
    }

    /**
     * The app goes to the background, before sync stops: stop reading
     * others' presence and, sharing ours, return once we are offline.
     */
    suspend fun background() {
        if (closed) return
        foreground = false
        stopReading()
        stopIdleTimer()
        if (_available.value && sharing.current()) setOwn(PresenceState.OFFLINE).join()
    }

    /** "Share my presence" changed: offline at once, or online if in the foreground. */
    fun sharingChanged(on: Boolean) {
        if (closed || !_available.value) return
        if (on) {
            if (foreground) beActive()
        } else {
            stopIdleTimer()
            setOwn(PresenceState.OFFLINE)
        }
    }

    /** A touch or key press: online again if idle. Cheap otherwise. */
    fun input() {
        lastInput = clock()
        if (!foreground || closed || idleTimer?.isActive == true || !_available.value || !sharing.on.value) return
        beActive()
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
        stopIdleTimer()
    }

    private fun failed(error: MtrxException, doing: String) {
        if (error is MtrxException.Other && error.reason == PRESENCE_UNAVAILABLE) {
            hide()
        } else {
            log.warn(TAG, "$doing failed: ${describe(error)}")
        }
    }

    /** Online, until idle. */
    private fun beActive() {
        lastInput = clock()
        setOwn(PresenceState.ONLINE)
        startIdleTimer()
    }

    private fun stopIdleTimer() {
        idleTimer?.cancel()
        idleTimer = null
    }

    /** Unavailable once [idleAfter] has passed since the last input. */
    private fun startIdleTimer() {
        stopIdleTimer()
        // Hidden by the call just made.
        if (!_available.value) return
        idleTimer = scope.launch {
            while (true) {
                val left = lastInput + idleAfter - clock()
                if (left <= 0) break
                delay(left)
            }
            setOwn(PresenceState.UNAVAILABLE)
        }
    }

    /** Set ours, after the previous call (unless that hid presence); failures are logged (or hide presence). */
    private fun setOwn(state: PresenceState): Job {
        own = state
        val previous = lastCall
        return scope.launch {
            previous?.join()
            if (!_available.value) return@launch
            try {
                session.setPresence(state, null)
            } catch (e: MtrxException) {
                failed(e, "Setting our presence")
            }
        }.also { lastCall = it }
    }

    companion object {
        /** Without input for this long in the foreground, we are idle. */
        const val IDLE_AFTER_MS = 5 * 60 * 1000L

        /** The room list's direct chats must settle this long before subscribing anew. */
        const val FOLLOW_DEBOUNCE_MS = 1000L

        private const val TAG = "Presence"
    }
}
