package org.mtrx.app.push

import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.json.JSONException
import org.json.JSONObject
import org.mtrx.app.model.RoomNotifications
import uniffi.mtrx.MtrxException
import uniffi.mtrx.MtrxInterface
import uniffi.mtrx.Notification

/** What a push from the Matrix push gateway says. */
sealed interface Push {
    /** A new event to notify about. */
    data class Event(val roomId: String, val eventId: String) : Push

    /** Only the unread count (e.g. read on another device); null if not given. */
    data class Counts(val unread: Long?) : Push
}

/**
 * The gateway's body, the Matrix push notification as UTF-8 JSON
 * (`{"notification": {"room_id", "event_id", "counts": {"unread"}, …}}`);
 * null if it is not one.
 */
fun parsePush(bytes: ByteArray): Push? = try {
    val notification = JSONObject(String(bytes, Charsets.UTF_8)).getJSONObject("notification")
    val eventId = notification.optString("event_id", "")
    val roomId = notification.optString("room_id", "")
    if (eventId.isNotEmpty() && roomId.isNotEmpty()) {
        Push.Event(roomId, eventId)
    } else {
        val counts = notification.optJSONObject("counts")
        Push.Counts(if (counts != null && counts.has("unread")) counts.getLong("unread") else null)
    }
} catch (e: JSONException) {
    null
}

/** Posts the message notifications. Callable from any thread. */
interface MessageNotifier : RoomNotifications {
    /** The room's notification, with [content] as its newest message. */
    suspend fun post(content: Notification)

    /** "New message" for the room, when the event could not be read. */
    fun postFallback(roomId: String)
}

/**
 * Handles each push (docs/mobile.md, section 10): reads what it is about
 * with [Mtrx.notification][MtrxInterface.notification], from this process
 * and without syncing, and posts it, unless it is for [foregroundRoom].
 * A plain class, so the end-to-end test calls it as the push service does.
 */
class PushHandler(
    private val mtrx: MtrxInterface,
    private val notifier: MessageNotifier,
    /** The room open on screen while the app is in the foreground. */
    private val foregroundRoom: () -> String?,
    /** The push found no session: stop the pushes. */
    private val onSignedOut: () -> Unit,
    private val timeout: Duration = 20.seconds,
) {
    suspend fun handle(bytes: ByteArray) {
        when (val push = parsePush(bytes)) {
            null -> Unit
            is Push.Counts -> if (push.unread == 0L) notifier.clearAll()
            is Push.Event -> show(push.roomId, push.eventId)
        }
    }

    private suspend fun show(roomId: String, eventId: String) {
        val content = try {
            withTimeout(timeout) { mtrx.notification(roomId, eventId) }
        } catch (e: MtrxException.NotLoggedIn) {
            onSignedOut()
            return
        } catch (e: MtrxException) {
            fallback(roomId)
            return
        } catch (e: TimeoutCancellationException) {
            fallback(roomId)
            return
        } catch (e: CancellationException) {
            // The service stopped first: say there is something, so the push is not lost.
            withContext(NonCancellable) { fallback(roomId) }
            throw e
        }
        if (content == null) return
        if (foregroundRoom() == content.roomId) return
        notifier.post(content)
    }

    private fun fallback(roomId: String) {
        if (foregroundRoom() != roomId) notifier.postFallback(roomId)
    }
}
