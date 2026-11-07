package org.mtrx.app.model

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import java.io.IOException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Where one value is kept across launches. */
interface PreferenceStore<T> {
    /** The saved value; null if none, or it cannot be read. */
    suspend fun load(): T?

    suspend fun save(value: T)
}

/**
 * A device preference, saved in [store]: [value] at once (the [default]
 * until the saved one is read), saved in order on change. A change before
 * the saved value is read wins over it. Used on [scope]'s (main) thread.
 */
class Preference<T>(private val store: PreferenceStore<T>, default: T, private val scope: CoroutineScope) {
    private val _value = MutableStateFlow(default)
    val value: StateFlow<T> = _value.asStateFlow()

    /** Set before the saved value was read: that one is stale. */
    private var changed = false

    // Saves run in order and each saves the latest value.
    private val saving = Mutex()

    private val loading: Deferred<Unit> = scope.async {
        val saved = store.load()
        if (!changed && saved != null) _value.value = saved
    }

    /** The value, once the saved one is read. */
    suspend fun current(): T {
        loading.await()
        return _value.value
    }

    fun set(value: T) {
        changed = true
        _value.value = value
        scope.launch { saving.withLock { store.save(_value.value) } }
    }

    fun update(change: (T) -> T) = set(change(_value.value))
}

/**
 * One key of [data], as text; [encode] null removes it, [decode] null
 * reads as none. A value that cannot be read or saved is not remembered;
 * the app works without it.
 */
class DataStoreText<T>(
    private val data: DataStore<Preferences>,
    private val key: Preferences.Key<String>,
    private val encode: (T) -> String?,
    private val decode: (String) -> T?,
) : PreferenceStore<T> {
    override suspend fun load(): T? = try {
        data.data.first()[key]?.let(decode)
    } catch (e: IOException) {
        null
    }

    override suspend fun save(value: T) {
        try {
            val text = encode(value)
            data.edit { prefs ->
                if (text == null) {
                    prefs.remove(key)
                } else {
                    prefs[key] = text
                }
            }
        } catch (e: IOException) {
            // Not remembered.
        }
    }
}

/** One switch of [data]; one that cannot be read or saved is not remembered. */
class DataStoreFlag(private val data: DataStore<Preferences>, private val key: Preferences.Key<Boolean>) :
    PreferenceStore<Boolean> {
    override suspend fun load(): Boolean? = try {
        data.data.first()[key]
    } catch (e: IOException) {
        null
    }

    override suspend fun save(value: Boolean) {
        try {
            data.edit { it[key] = value }
        } catch (e: IOException) {
            // Not remembered.
        }
    }
}

/** In memory, for tests and previews: one instance across "launches". */
class MemoryStore<T>(var saved: T? = null) : PreferenceStore<T> {
    override suspend fun load(): T? = saved

    override suspend fun save(value: T) {
        saved = value
    }
}
