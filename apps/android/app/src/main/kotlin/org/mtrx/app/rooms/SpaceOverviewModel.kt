package org.mtrx.app.rooms

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.mtrx.DirectoryRoom
import uniffi.mtrx.MtrxException
import uniffi.mtrx.Session

/**
 * The Rooms tab of space [spaceId]'s overview: its rooms and subspaces as
 * its server lists them, with the joined ones it leaves out. The library's
 * last list shows at once while it updates behind; a failed update offers
 * Try again. Joining or asking to join stays on the overview, the row
 * changing as the room list has it, at once (Open, Asked). Methods are
 * called on the main thread.
 */
class SpaceOverviewModel(
    val spaceId: String,
    private val session: Session,
) : ViewModel() {
    private val _rooms = MutableStateFlow(session.listedSpaceChildren(spaceId))
    /** Null until first listed. */
    val rooms: StateFlow<List<DirectoryRoom>?> = _rooms.asStateFlow()

    private val _updating = MutableStateFlow(false)
    val updating: StateFlow<Boolean> = _updating.asStateFlow()

    /** The last update failed: Try again. */
    private val _failed = MutableStateFlow(false)
    val failed: StateFlow<Boolean> = _failed.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    private val joining = Joining(viewModelScope, session) { _error.value = it }

    /** Rooms being joined or asked to join. */
    val pending: StateFlow<Set<String>> = joining.pending

    init {
        refresh()
    }

    fun refresh() {
        if (_updating.value) return
        _updating.value = true
        _failed.value = false
        viewModelScope.launch {
            try {
                _rooms.value = session.spaceChildren(spaceId)
            } catch (e: MtrxException) {
                _failed.value = true
            } finally {
                _updating.value = false
            }
        }
    }

    /** Join [room] (or accept its invite): its row opens, as the room list has it at once. */
    fun join(room: DirectoryRoom) = joining.join(room)

    /** Ask to join [room]: its row becomes Asked, as the room list has it at once. */
    fun knock(room: DirectoryRoom) = joining.knock(room)

    fun clearError() {
        _error.value = null
    }
}
