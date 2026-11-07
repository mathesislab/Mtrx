package org.mtrx.app.timeline

import org.mtrx.app.model.directPerson
import org.mtrx.app.rooms.NO_RIGHTS
import org.mtrx.app.rooms.NO_TOPIC
import uniffi.mtrx.Rights
import uniffi.mtrx.RoomSummary
import uniffi.mtrx.Topic

/**
 * What the timeline's top bar shows of the room: its name, and for a direct
 * chat with someone else ([person], their user ID) the room's avatar,
 * coloured by [seed] without one; its [topic] (text and links), the event
 * IDs of its [pinned] messages, a room's [space] (at the right), and our
 * [rights] in it.
 */
data class RoomHeader(
    val name: String,
    val avatarUrl: String?,
    val seed: String,
    val person: String?,
    val topic: Topic = NO_TOPIC,
    val pinned: List<String> = emptyList(),
    val space: RoomSummary? = null,
    val rights: Rights = NO_RIGHTS,
)

/**
 * The header of room [roomId], from its summary; the ID while it is not in
 * the room list. Our own direct chat (the Notes to self) has no person.
 * The topic comes parsed by the library; [space] is the room's
 * (`roomSpace`).
 */
fun roomHeader(
    roomId: String,
    room: RoomSummary?,
    ownUserId: String,
    space: RoomSummary? = null,
): RoomHeader = RoomHeader(
    name = room?.name ?: roomId,
    avatarUrl = room?.avatarUrl,
    seed = room?.directUser?.userId ?: roomId,
    person = room?.let { directPerson(it, ownUserId) },
    topic = room?.topic ?: NO_TOPIC,
    pinned = room?.pinned.orEmpty(),
    space = space,
    rights = room?.rights ?: NO_RIGHTS,
)
