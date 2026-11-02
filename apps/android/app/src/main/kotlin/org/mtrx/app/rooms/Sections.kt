package org.mtrx.app.rooms

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.mtrx.app.model.HOME_SAVED
import uniffi.mtrx.SectionLayout

// How this phone shows a space's sections (the library's `ListSection`s),
// and the Rooms tab's groups, folded or not.

/**
 * How this phone arranges one space's sections, by `ListSection.key`:
 * their [order], those [hidden] and those [folded]. Keys not in [order]
 * are new to the phone. [groupFolded]: the space's group on the Rooms tab
 * is folded.
 */
data class SectionPrefs(
    val order: List<String> = emptyList(),
    val hidden: Set<String> = emptySet(),
    val folded: Set<String> = emptySet(),
    val groupFolded: Boolean = false,
) {
    /** As the library takes it. */
    val library: SectionLayout get() = SectionLayout(order, hidden.sorted(), folded.sorted())
}

/** The key of Home's group on the Rooms tab, as the library gives it. */
const val HOME_GROUP = "home"

/**
 * The keys of the Rooms tab's groups folded in [prefs] (saved by
 * `SpaceKey.saved`), as the library names the groups.
 */
fun foldedGroups(prefs: Map<String, SectionPrefs>): List<String> =
    prefs.filterValues { it.groupFolded }.keys.map { if (it == HOME_SAVED) HOME_GROUP else it }

/** Where the sections' arrangement is kept, per space (`SpaceKey.saved`), on this phone. */
interface SectionStore {
    /** The saved arrangements; null if none, or they cannot be read. */
    suspend fun load(): Map<String, SectionPrefs>?

    suspend fun save(prefs: Map<String, SectionPrefs>)
}

/**
 * How this phone arranges each space's sections (shown, folded, order)
 * and whether its group on the Rooms tab is folded, by `SpaceKey.saved`
 * (Home's too), saved in [store]. Not the account's: the desktop and
 * other devices keep theirs. Used on [scope]'s (main) thread. The changes
 * take the keys of the space's sections now ([keys], in the phone's
 * order), so that a reorder keeps sections new to the phone in place.
 */
class SectionLayouts(private val store: SectionStore, private val scope: CoroutineScope) {
    private val _prefs = MutableStateFlow<Map<String, SectionPrefs>>(emptyMap())
    val prefs: StateFlow<Map<String, SectionPrefs>> = _prefs.asStateFlow()

    /** Changed before the saved ones were read: those are stale. */
    private var changed = false

    // Saves run in order and each saves the latest.
    private val saving = Mutex()

    init {
        scope.launch {
            val saved = store.load()
            if (!changed && saved != null) _prefs.value = saved
        }
    }

    /** Fold [key], or unfold it. */
    fun toggleFold(space: String, key: String) = change(space) {
        it.copy(folded = if (key in it.folded) it.folded - key else it.folded + key)
    }

    /** Fold the space's group on the Rooms tab (by the group's key), or unfold it. */
    fun toggleGroupFold(group: String) =
        change(if (group == HOME_GROUP) HOME_SAVED else group) { it.copy(groupFolded = !it.groupFolded) }

    fun setShown(space: String, key: String, shown: Boolean) = change(space) {
        it.copy(hidden = if (shown) it.hidden - key else it.hidden + key)
    }

    fun showAll(space: String) = change(space) { it.copy(hidden = emptySet()) }

    fun hideAll(space: String, keys: List<String>) = change(space) { it.copy(hidden = keys.toSet()) }

    /** Move the section at [from] to [to], in the phone's order of [keys]. */
    fun move(space: String, keys: List<String>, from: Int, to: Int) = change(space) {
        val order = keys.toMutableList()
        if (from !in order.indices || to !in order.indices || from == to) return@change it
        order.add(to, order.removeAt(from))
        it.copy(order = order)
    }

    private fun change(space: String, update: (SectionPrefs) -> SectionPrefs) {
        changed = true
        val current = _prefs.value[space] ?: SectionPrefs()
        _prefs.value = _prefs.value + (space to update(current))
        scope.launch { saving.withLock { store.save(_prefs.value) } }
    }
}
