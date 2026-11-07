package org.mtrx.app.timeline

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import java.time.LocalDate
import org.mtrx.app.model.Images
import org.mtrx.app.model.describe
import org.mtrx.app.rooms.RoomGroup
import org.mtrx.app.rooms.roomGroups
import org.mtrx.app.ui.RoomGroupsList
import uniffi.mtrx.MtrxException
import uniffi.mtrx.RoomListsInterface
import uniffi.mtrx.Session

/** The rooms shared with a person, by ID, or why they could not be read. */
sealed interface SharedRooms {
    data class Loaded(val ids: List<String>) : SharedRooms

    data class Failed(val reason: String?) : SharedRooms
}

/** Read the rooms [session] shares with [userId]. */
suspend fun loadSharedRooms(session: Session, userId: String): SharedRooms = try {
    SharedRooms.Loaded(session.sharedRooms(userId))
} catch (e: MtrxException) {
    SharedRooms.Failed(describe(e))
}

/**
 * A direct chat's panel of the rooms shared with its person ([userId],
 * [name]), read from [session] each time it shows (as iOS), grouped as
 * [rooms] lists them; why they could not be read in their place.
 */
@Composable
fun SharedRoomsPanel(
    userId: String,
    name: String,
    session: Session,
    rooms: RoomListsInterface,
    images: Images,
    onOpenRoom: (String) -> Unit,
    onOpenSpace: (String) -> Unit,
) {
    var shared by remember(userId) { mutableStateOf<SharedRooms?>(null) }
    LaunchedEffect(userId) { shared = loadSharedRooms(session, userId) }
    val today = remember { LocalDate.now() }
    when (val read = shared) {
        is SharedRooms.Failed ->
            SharedRoomsList(name, emptyList(), images, today, onOpenRoom, onOpenSpace, empty = read.reason ?: "")
        is SharedRooms.Loaded -> {
            val groups = remember(read, rooms) { roomGroups(rooms, among = read.ids) }
            SharedRoomsList(name, groups, images, today, onOpenRoom, onOpenSpace)
        }
        null -> SharedRoomsList(name, null, images, today, onOpenRoom, onOpenSpace)
    }
}

/**
 * The panel of the rooms shared with a direct chat's person ([name]), under
 * the header over the timeline and the composer: by space as on the Rooms
 * tab ([groups]; null while loading; [empty] when there are none). A room
 * opens with [onOpenRoom], a space's heading with [onOpenSpace] (their IDs).
 */
@Composable
fun SharedRoomsList(
    name: String,
    groups: List<RoomGroup>?,
    images: Images,
    today: LocalDate,
    onOpenRoom: (String) -> Unit,
    onOpenSpace: (String) -> Unit,
    empty: String = "No rooms shared with $name",
) {
    Surface(Modifier.fillMaxSize(), tonalElevation = 1.dp) {
        RoomGroupsList(
            groups.orEmpty(),
            loaded = groups != null,
            images,
            today,
            onOpenRoom = { onOpenRoom(it.id) },
            onOpenSpace = onOpenSpace,
            empty = empty,
        )
    }
}
