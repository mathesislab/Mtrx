package org.mtrx.app.rooms

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.mtrx.app.model.describe
import uniffi.mtrx.History
import uniffi.mtrx.JoinRule
import uniffi.mtrx.Member
import uniffi.mtrx.MtrxException
import uniffi.mtrx.NotifyMode
import uniffi.mtrx.Permissions
import uniffi.mtrx.Room
import uniffi.mtrx.RoomSettings
import uniffi.mtrx.RoomSettingsListener
import uniffi.mtrx.Session
import uniffi.mtrx.TaskHandle
import uniffi.mtrx.Topic

/**
 * Room info of [roomId] (a room or a space): its settings and members, and
 * the changes to them, through its [Room]. Nothing is read until [load]
 * (Room info shows; the room screen only saves the topic): from then on the
 * settings follow the room, and a change to them shows at once (the
 * library's); the members are read at each [load] and again after each
 * change to them. Changes run in a scope of their own, so leaving the
 * screen (which clears this model) does not cancel them; the room is closed
 * once they end. [composeTopic] makes the topic to save from its text and
 * links (the library's). Methods are called on the main thread.
 */
class RoomInfoModel(
    val roomId: String,
    private val session: Session,
    private val composeTopic: (Topic) -> String,
) : ViewModel(), RoomInfoActions {
    /** The room, or why it is not known; closed with the model. */
    private val room: Result<Room> = runCatching { session.room(roomId) }

    private val _settings = MutableStateFlow<RoomSettings?>(null)
    val settings: StateFlow<RoomSettings?> = _settings.asStateFlow()

    private val _members = MutableStateFlow<List<Member>?>(null)
    val members: StateFlow<List<Member>?> = _members.asStateFlow()

    /** A change to the members under way, as "Banning…". */
    private val _busy = MutableStateFlow<String?>(null)
    val busy: StateFlow<String?> = _busy.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    /** Follows the settings into [settings] from the first [load]; cancelled with the model. */
    private var subscription: TaskHandle? = null

    // Not cancelled with viewModelScope: completed when the model is cleared,
    // it ends once the changes under way are done.
    private val changesJob = SupervisorJob()
    private val changes = CoroutineScope(changesJob + Dispatchers.Main.immediate)

    /** Room info shows: follow the settings (from the first call on) and read the members. */
    fun load() {
        if (subscription == null) {
            try {
                subscription = room.getOrThrow().subscribeSettings(object : RoomSettingsListener {
                    // On a library thread: the settings once read, then after every change.
                    override fun onUpdate(settings: RoomSettings) {
                        _settings.value = settings
                    }
                })
            } catch (e: MtrxException) {
                _error.value = describe(e)
            }
        }
        viewModelScope.launch {
            try {
                _members.value = room.getOrThrow().members()
            } catch (e: MtrxException) {
                _error.value = describe(e)
            }
        }
    }

    override fun rename(name: String) {
        if (name.isBlank()) return
        change { it.setName(name.trim()) }
    }

    override fun setTopic(topic: Topic) = change { it.setTopic(composeTopic(topic)) }

    override fun setNotify(mode: NotifyMode) = change { it.setNotify(mode) }

    override fun setJoinRule(rule: JoinRule) = change { it.setJoinRule(rule) }

    override fun setListed(listed: Boolean) = change { it.setListed(listed) }

    override fun setSuggested(spaceId: String, suggested: Boolean) =
        change { session.room(spaceId).use { space -> space.setSuggested(roomId, suggested) } }

    override fun setHistory(history: History) = change { it.setHistory(history) }

    override fun setPermissions(permissions: Permissions) = change { it.setPermissions(permissions) }

    override fun encrypt() = change { it.encrypt() }

    /** The address `#name:ourserver`; blank removes it. */
    override fun setAddress(name: String) = change { it.setAddress(name.trim().removePrefix("#").ifEmpty { null }) }

    override fun addToSpace(spaceId: String) = change { session.room(spaceId).use { space -> space.addChild(roomId) } }

    override fun removeFromSpace(spaceId: String) =
        change { session.room(spaceId).use { space -> space.removeChild(roomId) } }

    override fun unban(userId: String) = change { it.unban(userId) }

    override fun setLevel(userId: String, level: Long) =
        changeMembers("Changing the level…") { it.setLevel(userId, level) }

    /** Remove [userId], or also ban them; they see [reason]. */
    override fun remove(userId: String, reason: String, ban: Boolean) =
        changeMembers(if (ban) "Banning…" else "Removing…") { it.remove(userId, ban, reason.trim().ifEmpty { null }) }

    /** Invite [userId] (`@name:server`). */
    override fun invite(userId: String) = changeMembers("Sending the invites…") { it.invite(userId.trim()) }

    /** Make a change, which shows at once (in [settings], or the room list's name and topic); a failure shows as [error]. */
    private fun change(call: suspend (Room) -> Unit) {
        changes.launch {
            try {
                call(room.getOrThrow())
            } catch (e: MtrxException) {
                _error.value = describe(e)
            }
        }
    }

    /** Make a change to the members, shown as [doing] meanwhile, then read them again. */
    private fun changeMembers(doing: String, call: suspend (Room) -> Unit) {
        changes.launch {
            _busy.value = doing
            try {
                val opened = room.getOrThrow()
                call(opened)
                _members.value = opened.members()
            } catch (e: MtrxException) {
                _error.value = describe(e)
            } finally {
                if (_busy.value == doing) _busy.value = null
            }
        }
    }

    override fun clearError() {
        _error.value = null
    }

    override fun onCleared() {
        subscription?.let {
            it.cancel()
            it.close()
        }
        // Changes under way keep the room until they end.
        changesJob.invokeOnCompletion { room.getOrNull()?.close() }
        changesJob.complete()
    }
}
