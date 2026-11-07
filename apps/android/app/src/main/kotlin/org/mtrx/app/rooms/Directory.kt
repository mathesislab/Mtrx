package org.mtrx.app.rooms

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.mtrx.app.model.describe
import uniffi.mtrx.DirectoryRoom
import uniffi.mtrx.MtrxException
import uniffi.mtrx.Role
import uniffi.mtrx.Session

// Rooms to join, as a space or a server's directory lists them: what each
// row says, and joining them. What a row offers is the library's
// joinButton.

/** "N members" (a space: "Space · N rooms"), then the topic's first line. */
fun rowDetails(room: DirectoryRoom): String {
    val count = if (room.isSpace) {
        "Space · " + if (room.children == 1uL) "1 room" else "${room.children} rooms"
    } else {
        if (room.members == 1uL) "1 member" else "${room.members} members"
    }
    val topic = room.topic.firstLine
    return listOfNotNull(count, topic).joinToString(" · ")
}

/** Our role, shown on joined rows: Admin or Mod; nothing for a member. */
fun roleTag(room: DirectoryRoom): String? = when (room.role) {
    Role.ADMIN -> "Admin"
    Role.MODERATOR -> "Mod"
    else -> null
}

/** The rooms whose name, address or topic has [filter], upper and lower case alike. */
fun filtered(rooms: List<DirectoryRoom>, filter: String): List<DirectoryRoom> {
    val text = filter.trim()
    if (text.isEmpty()) return rooms
    return rooms.filter { room ->
        listOfNotNull(room.name, room.alias, room.topic.text).any { it.contains(text, ignoreCase = true) }
    }
}

/**
 * Joining or asking to join listed rooms: [pending] while on its way, a
 * failure to [onError]. The rows then show Open or Asked as the room lists
 * have them, at once (the library's). Methods are called on the main thread.
 */
class Joining(
    private val scope: CoroutineScope,
    private val session: Session,
    private val onError: (String?) -> Unit,
) {
    private val _pending = MutableStateFlow<Set<String>>(emptySet())
    val pending: StateFlow<Set<String>> = _pending.asStateFlow()

    /** Join [room] (or accept its invite). */
    fun join(room: DirectoryRoom) = act(room) { session.join(room.id, room.via) }

    /** Ask to join [room]. */
    fun knock(room: DirectoryRoom) = act(room) { session.knock(room.id, room.via) }

    private fun act(room: DirectoryRoom, call: suspend () -> Unit) {
        if (room.id in _pending.value) return
        _pending.value += room.id
        scope.launch {
            try {
                call()
            } catch (e: MtrxException) {
                onError(describe(e))
            } finally {
                _pending.value -= room.id
            }
        }
    }
}
