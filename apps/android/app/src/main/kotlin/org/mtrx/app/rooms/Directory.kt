package org.mtrx.app.rooms

import uniffi.mtrx.DirectoryRoom
import uniffi.mtrx.JoinAccess
import uniffi.mtrx.Membership
import uniffi.mtrx.Role

// Rooms to join, as a space or a server's directory lists them: what each
// row offers and says.

/** What a listed room's row offers. */
enum class RowAction(val label: String) {
    JOIN("Join"),
    ASK("Ask to join"),
    OPEN("Open"),

    /** Only invited people may join: nothing to do. */
    INVITE_ONLY("Invite only"),

    /** We asked to join and wait. */
    ASKED("Asked"),
}

/**
 * Open where we are a member ([joined] also counts the room list's), Join
 * where we are invited or may join, Ask to join where it takes requests.
 */
fun rowAction(room: DirectoryRoom, joined: Boolean = false): RowAction = when {
    joined || room.membership == Membership.JOINED -> RowAction.OPEN
    room.membership == Membership.ASKED -> RowAction.ASKED
    room.membership == Membership.INVITED -> RowAction.JOIN
    room.access == JoinAccess.KNOCK -> RowAction.ASK
    room.access == JoinAccess.INVITE -> RowAction.INVITE_ONLY
    else -> RowAction.JOIN
}

/** "N members" (a space: "Space · N rooms"), then the topic's first line. */
fun rowDetails(room: DirectoryRoom): String {
    val count = if (room.isSpace) {
        "Space · " + if (room.children == 1uL) "1 room" else "${room.children} rooms"
    } else {
        if (room.members == 1uL) "1 member" else "${room.members} members"
    }
    val topic = room.topic?.lineSequence()?.map { it.trim() }?.firstOrNull { it.isNotEmpty() }
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
        listOfNotNull(room.name, room.alias, room.topic).any { it.contains(text, ignoreCase = true) }
    }
}

/** [rooms] with [roomId]'s membership now [membership]. */
fun withMembership(rooms: List<DirectoryRoom>, roomId: String, membership: Membership): List<DirectoryRoom> =
    rooms.map { if (it.id == roomId) it.copy(membership = membership) else it }
