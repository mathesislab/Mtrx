package org.mtrx.app.model

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.mtrx.RoomListsInterface
import uniffi.mtrx.SpaceKey

/** The key of Home (the rooms outside spaces) as saved since it was "Other rooms"; room IDs start with "!". */
const val HOME_SAVED = "other-rooms"

/** An entry of the spaces page as saved in the preferences. */
val SpaceKey.saved: String
    get() = when (this) {
        is SpaceKey.Space -> id
        SpaceKey.Home, SpaceKey.Outside -> HOME_SAVED
    }

/** The entry of the spaces page saved as [saved]. */
fun spaceKeyOf(saved: String): SpaceKey = if (saved == HOME_SAVED) SpaceKey.Outside else SpaceKey.Space(saved)

/**
 * The entry selected on the Spaces tab, kept in [saved] across launches.
 * [selected] has it while the room list shows it. A saved entry waits for a
 * list that has it; the first lists may be a cache, so it is forgotten only
 * when a loaded list does not (the space was left, or became nested in
 * another), as is a selected one that disappears. Used on [scope]'s (main)
 * thread.
 */
class SpaceSelection(
    private val saved: Preference<SpaceKey?>,
    private val rooms: StateFlow<RoomListsInterface>,
    scope: CoroutineScope,
) {
    private val _selected = MutableStateFlow<SpaceKey?>(null)
    val selected: StateFlow<SpaceKey?> = _selected.asStateFlow()

    init {
        scope.launch {
            saved.current()
            rooms.collect { update(it) }
        }
    }

    /** Select [entry] (tapped on the spaces page) and save it. */
    fun select(entry: SpaceKey) {
        saved.set(entry)
        update(rooms.value)
    }

    private fun update(lists: RoomListsInterface) {
        val entry = saved.value.value
        _selected.value = when {
            entry == null -> null
            lists.space(entry) != null -> entry
            lists.loaded() -> {
                saved.set(null)
                null
            }
            else -> null
        }
    }
}

/** The order of a space's rooms and direct messages (and of Home's). */
enum class RoomSort(val label: String) {
    /** The room list's: most recently active first. */
    DEFAULT("Default"),
    A_TO_Z("A to Z"),
    Z_TO_A("Z to A"),

    /** By last activity, rooms without any last. */
    RECENT("Most recent"),
    ;

    /** As the library takes it. */
    val library: uniffi.mtrx.RoomSort
        get() = when (this) {
            DEFAULT -> uniffi.mtrx.RoomSort.LIST_ORDER
            A_TO_Z -> uniffi.mtrx.RoomSort.A_TO_Z
            Z_TO_A -> uniffi.mtrx.RoomSort.Z_TO_A
            RECENT -> uniffi.mtrx.RoomSort.MOST_RECENT
        }
}
