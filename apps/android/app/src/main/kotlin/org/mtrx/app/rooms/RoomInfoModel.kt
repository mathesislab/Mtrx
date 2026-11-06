package org.mtrx.app.rooms

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
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
import uniffi.mtrx.Session
import uniffi.mtrx.Topic

/**
 * Room info of [roomId] (a room or a space): its settings and members, and
 * the changes to them, through its [Room]. Each change reads the settings
 * (or members) again after it, as the library returns once sync brought it
 * back. [composeTopic] makes the topic to save from its text and links (the
 * library's). Methods are called on the main thread.
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

    /** A change under way, as "Saving…". */
    private val _busy = MutableStateFlow<String?>(null)
    val busy: StateFlow<String?> = _busy.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    init {
        loadSettings()
        loadMembers()
    }

    fun loadSettings() {
        viewModelScope.launch {
            try {
                _settings.value = room.getOrThrow().settings()
            } catch (e: MtrxException) {
                _error.value = describe(e)
            }
        }
    }

    fun loadMembers() {
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
        change("Renaming…") { it.setName(name.trim()) }
    }

    override fun setTopic(topic: Topic) = change("Saving…") { it.setTopic(composeTopic(topic)) }

    override fun setNotify(mode: NotifyMode) = change("Saving…") { it.setNotify(mode) }

    override fun setJoinRule(rule: JoinRule) = change("Saving…") { it.setJoinRule(rule) }

    override fun setListed(listed: Boolean) = change("Saving…") { it.setListed(listed) }

    override fun setSuggested(spaceId: String, suggested: Boolean) =
        change("Saving…") { session.room(spaceId).use { space -> space.setSuggested(roomId, suggested) } }

    override fun setHistory(history: History) = change("Saving…") { it.setHistory(history) }

    override fun setPermissions(permissions: Permissions) = change("Saving…") { it.setPermissions(permissions) }

    override fun encrypt() = change("Turning encryption on…") { it.encrypt() }

    /** The address `#name:ourserver`; blank removes it. */
    override fun setAddress(name: String) =
        change("Saving…") { it.setAddress(name.trim().removePrefix("#").ifEmpty { null }) }

    override fun addToSpace(spaceId: String) =
        change("Saving…") { session.room(spaceId).use { space -> space.addChild(roomId) } }

    override fun removeFromSpace(spaceId: String) =
        change("Saving…") { session.room(spaceId).use { space -> space.removeChild(roomId) } }

    override fun unban(userId: String) = change("Saving…") { it.unban(userId) }

    override fun setLevel(userId: String, level: Long) =
        change("Changing the level…", members = true) { it.setLevel(userId, level) }

    /** Remove [userId], or also ban them; they see [reason]. */
    override fun remove(userId: String, reason: String, ban: Boolean) =
        change(if (ban) "Banning…" else "Removing…", members = true) {
            it.remove(userId, ban, reason.trim().ifEmpty { null })
        }

    /** Invite [userId] (`@name:server`). */
    override fun invite(userId: String) = change("Sending the invites…", members = true) { it.invite(userId.trim()) }

    private fun change(doing: String, members: Boolean = false, call: suspend (Room) -> Unit) {
        viewModelScope.launch {
            _busy.value = doing
            try {
                val opened = room.getOrThrow()
                call(opened)
                _settings.value = opened.settings()
                if (members) _members.value = opened.members()
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
        room.getOrNull()?.close()
    }
}
