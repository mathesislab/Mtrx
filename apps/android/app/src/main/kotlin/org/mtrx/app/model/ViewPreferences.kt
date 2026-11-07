package org.mtrx.app.model

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject
import org.mtrx.app.rooms.SectionPrefs
import uniffi.mtrx.SpaceKey

/** What the app shows, kept across launches (as the desktop's `view.json`). */
val Context.viewPreferences: DataStore<Preferences> by preferencesDataStore(name = "view")

/** The selected space in [data]. */
fun selectedSpaceStore(data: DataStore<Preferences>): PreferenceStore<SpaceKey?> =
    DataStoreText<SpaceKey?>(data, stringPreferencesKey("selected_space"), encode = { it?.saved }, decode = ::spaceKeyOf)

/** The order of the spaces' rooms in [data]. */
fun roomSortStore(data: DataStore<Preferences>): PreferenceStore<RoomSort> =
    DataStoreText<RoomSort>(
        data,
        stringPreferencesKey("room_sort"),
        encode = { it.name },
        decode = { saved -> RoomSort.entries.firstOrNull { it.name == saved } },
    )

/** "Share my presence" in [data]. */
fun sharePresenceStore(data: DataStore<Preferences>): PreferenceStore<Boolean> =
    DataStoreFlag(data, booleanPreferencesKey("share_presence"))

/**
 * How this phone arranges the spaces' sections, in [data]: a JSON object
 * by space, each with its sections' order, those hidden and those folded,
 * by key, and whether its group on the Rooms tab is folded.
 */
fun sectionStore(data: DataStore<Preferences>): PreferenceStore<Map<String, SectionPrefs>> =
    DataStoreText<Map<String, SectionPrefs>>(data, stringPreferencesKey("space_sections"), ::encodeSections, ::decodeSections)

private fun encodeSections(prefs: Map<String, SectionPrefs>): String = JSONObject().apply {
    for ((space, p) in prefs) {
        put(
            space,
            JSONObject()
                .put("order", JSONArray(p.order))
                .put("hidden", JSONArray(p.hidden.toList()))
                .put("folded", JSONArray(p.folded.toList()))
                .put("group_folded", p.groupFolded),
        )
    }
}.toString()

/** The saved arrangements; null if they cannot be read. */
private fun decodeSections(text: String): Map<String, SectionPrefs>? = try {
    val json = JSONObject(text)
    fun JSONObject.strings(name: String): List<String> {
        val array = optJSONArray(name) ?: return emptyList()
        return (0 until array.length()).map { array.getString(it) }
    }
    json.keys().asSequence().associateWith { space ->
        val p = json.getJSONObject(space)
        SectionPrefs(
            p.strings("order"),
            p.strings("hidden").toSet(),
            p.strings("folded").toSet(),
            groupFolded = p.optBoolean("group_folded"),
        )
    }
} catch (e: JSONException) {
    null
}
