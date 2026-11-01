package org.mtrx.app.model

import java.net.URI
import java.net.URISyntaxException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import uniffi.mtrx.MtrxException
import uniffi.mtrx.RoomListsInterface
import uniffi.mtrx.Session

/** Clears the message notifications. Callable from any thread. */
interface RoomNotifications {
    fun clearRoom(roomId: String)

    fun clearAll()
}

/**
 * The rooms whose unread notifications dropped to 0 from [previous] to
 * [current] (read here or on another device): their notifications go.
 */
fun roomsRead(previous: RoomListsInterface, current: RoomListsInterface): List<String> {
    val before = previous.rooms().associate { it.id to it.unreadNotifications }
    return current.rooms()
        .filter { it.unreadNotifications == 0uL && (before[it.id] ?: 0uL) > 0uL }
        .map { it.id }
}

/** An app that delivers UnifiedPush messages (a distributor, such as ntfy). */
data class Distributor(val packageName: String, val label: String)

/** Push notifications as Settings shows them. */
sealed interface PushState {
    /** Turned off in Settings (or not signed in). */
    data object Off : PushState

    /** No UnifiedPush app is installed. */
    data object NoDistributor : PushState

    /** Several are installed and none is chosen. */
    data class Choose(val distributors: List<Distributor>) : PushState

    /** Asked [distributor] for an endpoint. */
    data class Registering(val distributor: Distributor) : PushState

    /** The server pushes through [distributor]. */
    data class Registered(val distributor: Distributor) : PushState

    data class Failed(val reason: String) : PushState
}

/** The UnifiedPush connector, as the app uses it (one registration, the default instance). */
interface Distributors {
    /** The installed distributors' package names. */
    fun installed(): List<String>

    /** The one chosen before, if still installed. */
    fun saved(): String?

    /** The system's default distributor, if there is one. */
    fun default(): String?

    fun save(packageName: String)

    /** Ask the saved distributor for an endpoint: [PushRegistration.onNewEndpoint] follows. */
    fun register()

    fun unregister()

    fun label(packageName: String): String
}

/** What push registration keeps across launches. */
interface PushStore {
    /** The Settings switch; on by default. */
    suspend fun enabled(): Boolean

    suspend fun setEnabled(value: Boolean)

    /** The endpoint the server pushes to (the pusher's pushkey). */
    suspend fun endpoint(): String?

    suspend fun setEndpoint(value: String?)

    /** Whether the notification permission was asked after a sign-in. */
    suspend fun permissionAsked(): Boolean

    suspend fun setPermissionAsked()
}

/** The push gateway for an endpoint. */
interface GatewayDiscovery {
    suspend fun gatewayFor(endpoint: String): String
}

/** This device, as its pusher names it. */
data class PusherDevice(val name: String, val lang: String)

const val APP_ID = "org.mtrx.app"
const val APP_NAME = "Mtrx"
const val NOTIFY_PATH = "/_matrix/push/v1/notify"

/** UnifiedPush's own Matrix gateway, for push servers that are not one. */
const val FALLBACK_GATEWAY = "https://matrix.gateway.unifiedpush.org$NOTIFY_PATH"

/** `<scheme>://<host>[:port]` of [url]; null if it has none. */
fun originOf(url: String): String? = try {
    val uri = URI(url)
    val scheme = uri.scheme
    val host = uri.host
    if (scheme == null || host == null) {
        null
    } else {
        val port = if (uri.port == -1) "" else ":${uri.port}"
        "$scheme://$host$port"
    }
} catch (e: URISyntaxException) {
    null
}

/** The notify URL at the origin of [endpoint] (the push server as gateway). */
fun originGateway(endpoint: String): String? = originOf(endpoint)?.let { it + NOTIFY_PATH }

/**
 * Registers this device for push notifications (docs/mobile.md, section
 * 10): picks the distributor, registers with it while signed in and turned
 * on, and keeps the account's pusher on the endpoint it gives. The
 * [Account] tells it when a session opens and before it closes; Settings
 * turns it on or off and chooses the distributor; the push service passes
 * the connector's callbacks. Calls run one at a time in [scope].
 */
class PushRegistration(
    private val distributors: Distributors,
    private val store: PushStore,
    private val gateway: GatewayDiscovery,
    private val scope: CoroutineScope,
    private val device: () -> PusherDevice,
) {
    private val _state = MutableStateFlow<PushState>(PushState.Off)
    val state: StateFlow<PushState> = _state.asStateFlow()

    /** The Settings switch. */
    private val _enabled = MutableStateFlow(true)
    val enabled: StateFlow<Boolean> = _enabled.asStateFlow()

    private val lock = Mutex()
    private var session: Session? = null

    // The session's pusher is on the saved endpoint.
    private var pusherSet = false

    /** A session opened (restored or signed in): set its pusher again and register. */
    suspend fun opened(current: Session) = lock.withLock {
        session = current
        _enabled.value = store.enabled()
        if (!_enabled.value) {
            _state.value = PushState.Off
            return@withLock
        }
        store.endpoint()?.let { pusherSet = setPusher(current, it) }
        connect()
    }

    /**
     * The session is about to sign out (or be forgotten): stop the pushes to
     * this device, on the server and at the distributor.
     */
    suspend fun closing(current: Session) = lock.withLock {
        store.endpoint()?.let { deletePusher(current, it) }
        distributors.unregister()
        store.setEndpoint(null)
        session = null
        pusherSet = false
        _state.value = PushState.Off
    }

    /** A push found the session gone: stop the pushes at the distributor. */
    fun signedOut(): Job = scope.launch {
        lock.withLock {
            distributors.unregister()
            store.setEndpoint(null)
            session = null
            pusherSet = false
            _state.value = PushState.Off
        }
    }

    /** The Settings switch. */
    fun setEnabled(value: Boolean): Job = scope.launch {
        lock.withLock {
            store.setEnabled(value)
            _enabled.value = value
            if (value) {
                connect()
            } else {
                val current = session
                store.endpoint()?.let { if (current != null) deletePusher(current, it) }
                distributors.unregister()
                store.setEndpoint(null)
                pusherSet = false
                _state.value = PushState.Off
            }
        }
    }

    /** The user chose [packageName] among several distributors. */
    fun choose(packageName: String): Job = scope.launch {
        lock.withLock {
            distributors.save(packageName)
            connect()
        }
    }

    /** Look again at the installed distributors (Settings shows), if none was usable. */
    fun refresh(): Job = scope.launch {
        lock.withLock {
            val state = _state.value
            if (state is PushState.NoDistributor || state is PushState.Choose) connect()
        }
    }

    /** The connector's new endpoint: the server pushes there from now on. */
    fun onNewEndpoint(endpoint: String): Job = scope.launch {
        lock.withLock {
            // Turned off meanwhile: unregistered already.
            if (!_enabled.value) return@withLock
            val previous = store.endpoint()
            store.setEndpoint(endpoint)
            // Without a session yet, opening one sets the pusher.
            val current = session ?: return@withLock
            if (previous != null && previous != endpoint) deletePusher(current, previous)
            pusherSet = setPusher(current, endpoint)
            if (pusherSet) {
                _state.value = distributors.saved()?.let { PushState.Registered(distributor(it)) }
                    ?: PushState.Failed("the distributor is not installed")
            }
        }
    }

    /** The connector could not register; retried at the next start. */
    fun onRegistrationFailed(reason: String): Job = scope.launch {
        lock.withLock {
            if (_enabled.value && session != null) _state.value = PushState.Failed(reason)
        }
    }

    /** The distributor unregistered this app: no more pushes until the next start. */
    fun onUnregistered(): Job = scope.launch {
        lock.withLock {
            val current = session
            store.endpoint()?.let { if (current != null) deletePusher(current, it) }
            store.setEndpoint(null)
            pusherSet = false
            if (_enabled.value && current != null) {
                _state.value = PushState.Failed("the distributor unregistered Mtrx")
            }
        }
    }

    /** Whether to ask for the notification permission after a sign-in; asked once. */
    suspend fun askPermissionOnce(): Boolean = lock.withLock {
        if (store.permissionAsked()) {
            false
        } else {
            store.setPermissionAsked()
            true
        }
    }

    // Under the lock: pick the distributor and register with it, when signed in and on.
    private suspend fun connect() {
        if (session == null || !_enabled.value) {
            _state.value = PushState.Off
            return
        }
        val chosen = distributors.saved() ?: run {
            val installed = distributors.installed()
            when {
                installed.isEmpty() -> {
                    _state.value = PushState.NoDistributor
                    return
                }
                installed.size == 1 -> installed.first()
                else -> distributors.default() ?: run {
                    _state.value = PushState.Choose(installed.map { distributor(it) })
                    return
                }
            }.also { distributors.save(it) }
        }
        distributors.register()
        _state.value = if (pusherSet) PushState.Registered(distributor(chosen)) else PushState.Registering(distributor(chosen))
    }

    private fun distributor(packageName: String) = Distributor(packageName, distributors.label(packageName))

    // Under the lock. False (and the state says why) if the server refused.
    private suspend fun setPusher(current: Session, endpoint: String): Boolean {
        val here = device()
        return try {
            current.setPusher(
                gatewayUrl = gateway.gatewayFor(endpoint),
                pushkey = endpoint,
                appId = APP_ID,
                appName = APP_NAME,
                deviceName = here.name,
                lang = here.lang,
            )
            true
        } catch (e: MtrxException) {
            _state.value = PushState.Failed(describe(e) ?: "the server refused the pusher")
            false
        } catch (e: IllegalStateException) {
            // The session was closed meanwhile.
            false
        }
    }

    // Under the lock. Succeeds if there is none; a failure (offline, signed out) is not retried.
    private suspend fun deletePusher(current: Session, endpoint: String) {
        try {
            current.deletePusher(endpoint)
        } catch (e: MtrxException) {
            // The server drops a signed-out device's pushers itself.
        } catch (e: IllegalStateException) {
            // The session was closed meanwhile.
        }
    }
}
