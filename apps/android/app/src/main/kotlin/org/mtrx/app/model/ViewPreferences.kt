package org.mtrx.app.model

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import java.io.IOException
import kotlinx.coroutines.flow.first
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject
import org.mtrx.app.rooms.SectionPrefs
import org.mtrx.app.rooms.SectionStore
import uniffi.mtrx.SpaceKey

/** What the app shows, kept across launches (as the desktop's `view.json`). */
val Context.viewPreferences: DataStore<Preferences> by preferencesDataStore(name = "view")

/** The selected space in [preferences]. */
class DataStoreSelectedSpace(private val preferences: DataStore<Preferences>) : SelectedSpaceStore {
    override suspend fun load(): SpaceKey? = try {
        preferences.data.first()[SELECTED_SPACE]?.let { spaceKeyOf(it) }
    } catch (e: IOException) {
        null
    }

    override suspend fun save(entry: SpaceKey?) {
        try {
            preferences.edit {
                it.remove(SELECTED_SPACE)
                if (entry != null) it[SELECTED_SPACE] = entry.saved
            }
        } catch (e: IOException) {
            // Not remembered; the app works without it.
        }
    }

    private companion object {
        val SELECTED_SPACE = stringPreferencesKey("selected_space")
    }
}

/** The order of the spaces' rooms in [preferences]. */
class DataStoreRoomSort(private val preferences: DataStore<Preferences>) : RoomSortStore {
    override suspend fun load(): RoomSort? = try {
        preferences.data.first()[ROOM_SORT]?.let { saved -> RoomSort.entries.firstOrNull { it.name == saved } }
    } catch (e: IOException) {
        null
    }

    override suspend fun save(sort: RoomSort) {
        try {
            preferences.edit { it[ROOM_SORT] = sort.name }
        } catch (e: IOException) {
            // Not remembered; the app works without it.
        }
    }

    private companion object {
        val ROOM_SORT = stringPreferencesKey("room_sort")
    }
}

/** "Share my presence" in [preferences]. */
class DataStoreSharePresence(private val preferences: DataStore<Preferences>) : SharePresenceStore {
    override suspend fun load(): Boolean? = try {
        preferences.data.first()[SHARE_PRESENCE]
    } catch (e: IOException) {
        null
    }

    override suspend fun save(on: Boolean) {
        try {
            preferences.edit { it[SHARE_PRESENCE] = on }
        } catch (e: IOException) {
            // Not remembered; the default (on) comes back at the next launch.
        }
    }

    private companion object {
        val SHARE_PRESENCE = booleanPreferencesKey("share_presence")
    }
}

/**
 * How this phone arranges the spaces' sections, in [preferences]: a JSON
 * object by space, each with its sections' order, those hidden and those
 * folded, by key, and whether its group on the Rooms tab is folded.
 */
class DataStoreSections(private val preferences: DataStore<Preferences>) : SectionStore {
    override suspend fun load(): Map<String, SectionPrefs>? = try {
        preferences.data.first()[SECTIONS]?.let(::decode)
    } catch (e: IOException) {
        null
    } catch (e: JSONException) {
        null
    }

    override suspend fun save(prefs: Map<String, SectionPrefs>) {
        try {
            preferences.edit { it[SECTIONS] = encode(prefs) }
        } catch (e: IOException) {
            // Not remembered; the app works without it.
        }
    }

    private companion object {
        val SECTIONS = stringPreferencesKey("space_sections")

        fun encode(prefs: Map<String, SectionPrefs>): String = JSONObject().apply {
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

        fun decode(text: String): Map<String, SectionPrefs> {
            val json = JSONObject(text)
            fun JSONObject.strings(name: String): List<String> {
                val array = optJSONArray(name) ?: return emptyList()
                return (0 until array.length()).map { array.getString(it) }
            }
            return json.keys().asSequence().associateWith { space ->
                val p = json.getJSONObject(space)
                SectionPrefs(
                    p.strings("order"),
                    p.strings("hidden").toSet(),
                    p.strings("folded").toSet(),
                    groupFolded = p.optBoolean("group_folded"),
                )
            }
        }
    }
}
