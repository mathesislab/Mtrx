package org.mtrx.app.timeline

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.mtrx.app.model.describe
import uniffi.mtrx.MtrxException
import uniffi.mtrx.RoomSummary
import uniffi.mtrx.Session
import uniffi.mtrx.User

/** What the person's info sheet shows of them. */
data class PersonCard(val userId: String, val name: String, val avatarUrl: String?)

/**
 * The person [userId] of the direct chat [header]: the profile's name and
 * avatar where set, otherwise the chat's (also while the profile loads, or
 * when it could not).
 */
fun personCard(userId: String, header: RoomHeader, profile: User?): PersonCard = PersonCard(
    userId = userId,
    name = profile?.name?.takeIf { it.isNotBlank() } ?: header.name,
    avatarUrl = profile?.avatarUrl ?: header.avatarUrl,
)

/**
 * The info sheet about a direct chat's person [userId]: their profile, and
 * inviting them to one of our spaces. The profile loads in [scope] (the
 * room's); invites run in [actions], which leaving the room does not
 * cancel. Methods are called on the main thread; [close] when the sheet is.
 */
class PersonInfoModel(
    val userId: String,
    private val session: Session,
    scope: CoroutineScope,
    private val actions: CoroutineScope,
) {
    /** Null while loading, and after a failure (the chat's name and avatar stay). */
    private val _profile = MutableStateFlow<User?>(null)
    val profile: StateFlow<User?> = _profile.asStateFlow()

    /** The list of spaces to invite to is open, instead of the person. */
    private val _choosingSpace = MutableStateFlow(false)
    val choosingSpace: StateFlow<Boolean> = _choosingSpace.asStateFlow()

    /** The space an invite is being sent to (its row spins); one at a time. */
    private val _inviting = MutableStateFlow<String?>(null)
    val inviting: StateFlow<String?> = _inviting.asStateFlow()

    /** An invite's outcome, to show briefly. */
    private val _message = MutableStateFlow<String?>(null)
    val message: StateFlow<String?> = _message.asStateFlow()

    private val loading: Job = scope.launch {
        try {
            _profile.value = session.userProfile(userId)
        } catch (e: MtrxException) {
            // The chat's name and avatar stay, without a message.
        }
    }

    fun chooseSpace() {
        _choosingSpace.value = true
    }

    /** Back from the spaces to the person. */
    fun closeSpaces() {
        _choosingSpace.value = false
    }

    /**
     * Invite the person to [space]: on success "Invited to …" and back to the
     * person; on failure the reason, the list staying. Ignored while one runs.
     */
    fun invite(space: RoomSummary) {
        if (_inviting.value != null) return
        _inviting.value = space.id
        actions.launch {
            try {
                session.room(space.id).use { it.invite(userId) }
                _message.value = "Invited to ${space.name}."
                _choosingSpace.value = false
            } catch (e: MtrxException) {
                _message.value = describe(e)
            } finally {
                _inviting.value = null
            }
        }
    }

    fun clearMessage() {
        _message.value = null
    }

    /** The sheet closed: stop loading the profile (an invite under way still completes). */
    fun close() {
        loading.cancel()
    }
}
