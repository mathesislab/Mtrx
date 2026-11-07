package org.mtrx.app.rooms

import kotlinx.coroutines.flow.StateFlow
import org.mtrx.app.model.HOME_SAVED
import org.mtrx.app.model.Preference
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

/**
 * How this phone arranges each space's sections (shown, folded, order)
 * and whether its group on the Rooms tab is folded, by `SpaceKey.saved`
 * (Home's too), kept in [saved]. Not the account's: the desktop and other
 * devices keep theirs. The changes take the keys of the space's sections
 * now ([keys], in the phone's order), so that a reorder keeps sections new
 * to the phone in place.
 */
class SectionLayouts(private val saved: Preference<Map<String, SectionPrefs>>) {
    val prefs: StateFlow<Map<String, SectionPrefs>> = saved.value

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

    private fun change(space: String, update: (SectionPrefs) -> SectionPrefs) = saved.update { prefs ->
        prefs + (space to update(prefs[space] ?: SectionPrefs()))
    }
}
