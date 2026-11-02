package org.mtrx.app.rooms

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.mtrx.app.model.describe
import uniffi.mtrx.DirectoryRoom
import uniffi.mtrx.Membership
import uniffi.mtrx.MtrxException
import uniffi.mtrx.Session

/**
 * The Rooms tab of space [spaceId]'s overview: its rooms and subspaces as
 * its server lists them. The last list ([cache]) shows at once while it
 * updates behind; a failed update offers Try again. Joining or asking to
 * join stays on the overview, the row changing (Open as the room list has
 * it, at once; Asked). Methods are called on the main thread.
 */
class SpaceOverviewModel(
    val spaceId: String,
    private val session: Session,
    private val cache: SpaceChildrenCache,
) : ViewModel() {
    private val _rooms = MutableStateFlow(cache[spaceId])
    /** Null until first listed. */
    val rooms: StateFlow<List<DirectoryRoom>?> = _rooms.asStateFlow()

    private val _updating = MutableStateFlow(false)
    val updating: StateFlow<Boolean> = _updating.asStateFlow()

    /** The last update failed: Try again. */
    private val _failed = MutableStateFlow(false)
    val failed: StateFlow<Boolean> = _failed.asStateFlow()

    /** Rooms being joined or asked to join. */
    private val _pending = MutableStateFlow<Set<String>>(emptySet())
    val pending: StateFlow<Set<String>> = _pending.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    init {
        refresh()
    }

    fun refresh() {
        if (_updating.value) return
        _updating.value = true
        _failed.value = false
        viewModelScope.launch {
            try {
                val list = session.spaceChildren(spaceId)
                cache[spaceId] = list
                _rooms.value = list
            } catch (e: MtrxException) {
                _failed.value = true
            } finally {
                _updating.value = false
            }
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
                val list = withMembership(_rooms.value.orEmpty(), room.id, after)
                cache[spaceId] = list
                _rooms.value = list
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
}
