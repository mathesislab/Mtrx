package org.mtrx.app.model

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
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

/** Where the selected space is kept across launches. */
interface SelectedSpaceStore {
    /** The saved entry; null if none, or it cannot be read. */
    suspend fun load(): SpaceKey?

    /** Save [entry], or delete the saved one (null). */
    suspend fun save(entry: SpaceKey?)
}

/**
 * The entry selected on the Spaces tab, saved in [store] across launches.
 * [selected] has it while the room list shows it. A saved entry waits for a
 * list that has it; the first lists may be a cache, so it is forgotten only
 * when a loaded list does not (the space was left, or became nested in
 * another), as is a selected one that disappears. Used on [scope]'s (main)
 * thread.
 */
class SpaceSelection(
    private val store: SelectedSpaceStore,
    private val rooms: StateFlow<RoomListsInterface>,
    private val scope: CoroutineScope,
) {
    private val _selected = MutableStateFlow<SpaceKey?>(null)
    val selected: StateFlow<SpaceKey?> = _selected.asStateFlow()

    /** The selection, also while the room list does not show it (yet). */
    private var chosen: SpaceKey? = null

    /** Selected or cleared before the saved entry was read: that one is stale. */
    private var changed = false

    // Saves run in order and each saves the latest choice.
    private val saving = Mutex()

    init {
        scope.launch {
            val saved = store.load()
            if (!changed) chosen = saved
            rooms.collect { update(it) }
        }
    }

    /** Select [entry] (tapped on the spaces page) and save it. */
    fun select(entry: SpaceKey) {
        chosen = entry
        changed = true
        update(rooms.value)
        scope.launch { save() }
    }

    /** At sign-out: nothing selected, and nothing saved. */
    suspend fun clear() {
        chosen = null
        changed = true
        _selected.value = null
        save()
    }

    private fun update(lists: RoomListsInterface) {
        val entry = chosen
        _selected.value = when {
            entry == null -> null
            lists.space(entry) != null -> entry
            lists.loaded() -> {
                chosen = null
                scope.launch { save() }
                null
            }
            else -> null
        }
    }

    private suspend fun save() {
        saving.withLock { store.save(chosen) }
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

/** Where the order of the spaces' rooms is kept across launches. */
interface RoomSortStore {
    /** The saved order; null if none, or it cannot be read. */
    suspend fun load(): RoomSort?

    suspend fun save(sort: RoomSort)
}

/**
 * The order chosen for every space's rooms page, saved in [store] across
 * launches (and sign-outs: it is the device's, not the account's). Used on
 * [scope]'s (main) thread.
 */
class RoomSortChoice(private val store: RoomSortStore, private val scope: CoroutineScope) {
    private val _sort = MutableStateFlow(RoomSort.DEFAULT)
    val sort: StateFlow<RoomSort> = _sort.asStateFlow()

    /** Chosen before the saved order was read: that one is stale. */
    private var changed = false

    // Saves run in order and each saves the latest choice.
    private val saving = Mutex()

    init {
        scope.launch {
            val saved = store.load()
            if (!changed && saved != null) _sort.value = saved
        }
    }

    fun choose(sort: RoomSort) {
        changed = true
        _sort.value = sort
        scope.launch { saving.withLock { store.save(_sort.value) } }
    }
}
