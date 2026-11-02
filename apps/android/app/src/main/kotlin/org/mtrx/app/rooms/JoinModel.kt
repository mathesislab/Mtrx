package org.mtrx.app.rooms

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.mtrx.app.model.describe
import uniffi.mtrx.Address
import uniffi.mtrx.DirectoryKind
import uniffi.mtrx.DirectoryRoom
import uniffi.mtrx.Membership
import uniffi.mtrx.MtrxException
import uniffi.mtrx.Session

/** What the join screen lists. */
sealed interface JoinResults {
    data object Loading : JoinResults

    /** A page or more of the server's directory; [next] for the following page. */
    data class Listed(val rooms: List<DirectoryRoom>, val next: String?) : JoinResults

    /** The room or space a pasted address or link names. */
    data class Preview(val room: DirectoryRoom) : JoinResults

    data class Failed(val message: String) : JoinResults
}

/**
 * Join a space or room: a server's public directory ([server], ours while
 * null), searched by [query] and filtered by [kind]; a pasted address or
 * link ([parseAddress], the library's) shows its preview instead. A server
 * browsed successfully is remembered (synced to our other devices). Rows
 * asked to join here change in place; those joined open as the room list
 * has them, at once. Methods are called on the main thread.
 */
class JoinModel(
    private val session: Session,
    private val parseAddress: (String) -> Address?,
    private val searchDelayMillis: Long = SEARCH_DELAY_MILLIS,
) : ViewModel() {
    /** Servers browsed before, most recent first (ours is null, first in the picker). */
    val servers: List<String> = try {
        session.directoryServers()
    } catch (e: MtrxException) {
        emptyList()
    }

    private val _server = MutableStateFlow<String?>(null)
    val server: StateFlow<String?> = _server.asStateFlow()

    private val _query = MutableStateFlow("")
    val query: StateFlow<String> = _query.asStateFlow()

    private val _kind = MutableStateFlow(DirectoryKind.ALL)
    val kind: StateFlow<DirectoryKind> = _kind.asStateFlow()

    private val _results = MutableStateFlow<JoinResults>(JoinResults.Loading)
    val results: StateFlow<JoinResults> = _results.asStateFlow()

    private val _pending = MutableStateFlow<Set<String>>(emptySet())
    val pending: StateFlow<Set<String>> = _pending.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    private var search: Job? = null

    init {
        load(delayed = false)
    }

    /** Browse [server]'s directory; null for ours. */
    fun setServer(server: String?) {
        val name = server?.trim()?.removePrefix("https://")?.trimEnd('/')?.ifEmpty { null }
        if (name == _server.value) return
        _server.value = name
        load(delayed = false)
    }

    fun setQuery(query: String) {
        if (query == _query.value) return
        _query.value = query
        load(delayed = true)
    }

    fun setKind(kind: DirectoryKind) {
        if (kind == _kind.value) return
        _kind.value = kind
        load(delayed = false)
    }

    /** Again, after a failure. */
    fun retry() = load(delayed = false)

    /** The next page of the directory, if any. */
    fun more() {
        val listed = _results.value as? JoinResults.Listed ?: return
        val next = listed.next ?: return
        if (search?.isActive == true) return
        val server = _server.value
        search = viewModelScope.launch {
            try {
                val page = session.publicRooms(server, _query.value.trim(), _kind.value, next)
                _results.value = JoinResults.Listed(listed.rooms + page.rooms, page.next)
            } catch (e: MtrxException) {
                _error.value = describe(e)
            }
        }
    }

    private fun load(delayed: Boolean) {
        search?.cancel()
        val server = _server.value
        val query = _query.value.trim()
        val kind = _kind.value
        search = viewModelScope.launch {
            if (delayed) delay(searchDelayMillis)
            _results.value = JoinResults.Loading
            val address = parseAddress(query)
            _results.value = try {
                if (address != null) {
                    JoinResults.Preview(session.preview(address))
                } else {
                    val page = session.publicRooms(server, query, kind, null)
                    if (server != null) remember(server)
                    JoinResults.Listed(page.rooms, page.next)
                }
            } catch (e: MtrxException) {
                JoinResults.Failed(
                    when {
                        address != null -> "Could not find it: ${describe(e)}"
                        server != null -> "$server doesn't share its directory, or can't be reached."
                        else -> "Couldn't list the rooms."
                    },
                )
            }
        }
    }

    private suspend fun remember(server: String) {
        try {
            session.rememberDirectoryServer(server)
        } catch (e: MtrxException) {
            // Only a convenience.
        }
    }

    /** Join [room] (or accept its invite): its row opens, as the room list has it at once. */
    fun join(room: DirectoryRoom) = act(room, after = null) { session.join(room.id, room.via) }

    /** Ask to join [room]: its row becomes Asked. */
    fun knock(room: DirectoryRoom) = act(room, Membership.ASKED) { session.knock(room.id, room.via) }

    private fun act(room: DirectoryRoom, after: Membership?, call: suspend () -> Unit) {
        if (room.id in _pending.value) return
        _pending.value += room.id
        viewModelScope.launch {
            try {
                call()
                if (after == null) return@launch
                _results.value = when (val shown = _results.value) {
                    is JoinResults.Listed -> shown.copy(rooms = withMembership(shown.rooms, room.id, after))
                    is JoinResults.Preview -> if (shown.room.id == room.id) JoinResults.Preview(shown.room.copy(membership = after)) else shown
                    else -> shown
                }
            } catch (e: MtrxException) {
                _error.value = describe(e)
            } finally {
                _pending.value -= room.id
            }
        }
    }

    fun clearError() {
        _error.value = null
    }

    companion object {
        /** Typing waits this long before searching. */
        const val SEARCH_DELAY_MILLIS = 400L
    }
}
