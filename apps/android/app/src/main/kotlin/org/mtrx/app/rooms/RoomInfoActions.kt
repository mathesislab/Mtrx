package org.mtrx.app.rooms

import uniffi.mtrx.History
import uniffi.mtrx.JoinRule
import uniffi.mtrx.NotifyMode
import uniffi.mtrx.Permissions

/**
 * The changes Room info makes ([RoomInfoModel]); the screenshot tests give
 * one that does nothing. People by user ID, spaces by room ID.
 */
interface RoomInfoActions {
    fun rename(name: String)

    /** The topic as typed in the editor; the library composes it. */
    fun setTopic(draft: TopicDraft)

    fun setNotify(mode: NotifyMode)

    fun setJoinRule(rule: JoinRule)

    fun setListed(listed: Boolean)

    fun setSuggested(spaceId: String, suggested: Boolean)

    fun setHistory(history: History)

    fun setPermissions(permissions: Permissions)

    fun encrypt()

    fun setAddress(name: String)

    fun addToSpace(spaceId: String)

    fun removeFromSpace(spaceId: String)

    fun unban(userId: String)

    fun setLevel(userId: String, level: Long)

    fun remove(userId: String, reason: String, ban: Boolean)

    fun invite(userId: String)

    fun clearError()
}
