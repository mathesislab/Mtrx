package org.mtrx.app.push

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.Person
import androidx.core.graphics.drawable.IconCompat
import kotlinx.coroutines.withTimeoutOrNull
import org.mtrx.app.MainActivity
import org.mtrx.app.R
import uniffi.mtrx.Notification
import uniffi.mtrx.User

/** How a room's notification shows a message. */
data class NotificationLook(
    /** The conversation's title: the room's name, none for a direct chat (the sender's). */
    val title: String?,
    /** The push rules ask for no sound. */
    val silent: Boolean,
    /** The message's time, ms. */
    val at: Long,
)

fun lookOf(content: Notification): NotificationLook = NotificationLook(
    title = if (content.direct) null else content.roomName,
    silent = !content.noisy,
    at = content.timestamp.toEpochMilli(),
)

/** Whether message notifications can show: the permission, the app's switch in Android and the channel. */
fun notificationsAllowed(context: Context): Boolean {
    if (!NotificationManagerCompat.from(context).areNotificationsEnabled()) return false
    val channel = context.getSystemService(NotificationManager::class.java)
        .getNotificationChannel(MessageNotifications.CHANNEL) ?: return true
    return channel.importance != NotificationManager.IMPORTANCE_NONE
}

/**
 * The message notifications: one per room (tag = the room ID, id 0) in
 * the channel "Messages", with the room's latest messages in a
 * `MessagingStyle`, grouped with a summary from two rooms on. Tapping one
 * opens its room ([MainActivity.EXTRA_ROOM_ID]). The messages so far are
 * kept in memory, per process. [avatar] gives the sender's avatar when the
 * app's session is open (null otherwise).
 */
class MessageNotifications(
    private val context: Context,
    private val avatar: suspend (mxcUri: String) -> Bitmap?,
) : MessageNotifier {
    private val manager = context.getSystemService(NotificationManager::class.java)

    // The messages shown per room, newest last; guarded by itself.
    private val shown = mutableMapOf<String, List<Shown>>()

    private data class Shown(val sender: User, val body: String, val at: Long, val icon: Bitmap?)

    /** Create the channel; at start. */
    fun createChannel() {
        val channel = NotificationChannel(CHANNEL, "Messages", NotificationManager.IMPORTANCE_HIGH)
        manager.createNotificationChannel(channel)
    }

    override suspend fun post(content: Notification) {
        if (!notificationsAllowed(context)) return
        val icon = content.sender.avatarUrl?.let { withTimeoutOrNull(AVATAR_WAIT_MS) { avatar(it) } }
        val message = Shown(content.sender, content.body, content.timestamp.toEpochMilli(), icon)
        val messages = synchronized(shown) {
            val list = (shown[content.roomId].orEmpty() + message).takeLast(KEPT)
            shown[content.roomId] = list
            list
        }
        val look = lookOf(content)
        val me = Person.Builder().setName("Me").build()
        val style = NotificationCompat.MessagingStyle(me)
            .setGroupConversation(look.title != null)
        look.title?.let { style.setConversationTitle(it) }
        for (each in messages) {
            val person = Person.Builder()
                .setKey(each.sender.userId)
                .setName(each.sender.displayName)
                .apply { each.icon?.let { setIcon(IconCompat.createWithBitmap(it)) } }
                .build()
            style.addMessage(each.body, each.at, person)
        }
        val notification = builder(content.roomId)
            .setStyle(style)
            .setWhen(look.at)
            .setShowWhen(true)
            .setSilent(look.silent)
            .build()
        show(content.roomId, notification)
    }

    override fun postFallback(roomId: String) {
        if (!notificationsAllowed(context)) return
        val notification = builder(roomId)
            .setContentTitle("Mtrx")
            .setContentText("New message")
            .build()
        show(roomId, notification)
    }

    override fun clearRoom(roomId: String) {
        synchronized(shown) { shown.remove(roomId) }
        manager.cancel(roomId, ROOM_ID)
        updateSummary()
    }

    override fun clearAll() {
        synchronized(shown) { shown.clear() }
        manager.cancelAll()
    }

    private fun builder(roomId: String): NotificationCompat.Builder =
        NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setGroup(GROUP)
            .setGroupAlertBehavior(NotificationCompat.GROUP_ALERT_CHILDREN)
            .setContentIntent(openRoom(roomId))

    private fun openRoom(roomId: String): PendingIntent {
        // The data makes each room's intent distinct, so each keeps its extra.
        val intent = Intent(context, MainActivity::class.java)
            .setData(Uri.fromParts("mtrx-room", roomId, null))
            .putExtra(MainActivity.EXTRA_ROOM_ID, roomId)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        return PendingIntent.getActivity(
            context,
            0,
            intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    // The permission is checked in notificationsAllowed().
    @Suppress("MissingPermission")
    private fun show(roomId: String, notification: android.app.Notification) {
        try {
            manager.notify(roomId, ROOM_ID, notification)
        } catch (e: SecurityException) {
            // The permission was revoked meanwhile.
            return
        }
        updateSummary()
    }

    /** The group's summary while two rooms or more have a notification. */
    @Suppress("MissingPermission")
    private fun updateSummary() {
        val rooms = manager.activeNotifications.filter {
            it.notification.group == GROUP && it.id == ROOM_ID && it.tag != null
        }
        if (rooms.size < 2) {
            manager.cancel(SUMMARY_ID)
            return
        }
        val inbox = NotificationCompat.InboxStyle()
        rooms.forEach { sbn ->
            val title = sbn.notification.extras.getCharSequence(NotificationCompat.EXTRA_CONVERSATION_TITLE)
                ?: sbn.notification.extras.getCharSequence(NotificationCompat.EXTRA_TITLE)
            title?.let { inbox.addLine(it) }
        }
        val summary = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setContentTitle("Mtrx")
            .setContentText("${rooms.size} rooms")
            .setStyle(inbox.setSummaryText("${rooms.size} rooms"))
            .setGroup(GROUP)
            .setGroupSummary(true)
            .setGroupAlertBehavior(NotificationCompat.GROUP_ALERT_CHILDREN)
            .setAutoCancel(true)
            .setSilent(true)
            .build()
        try {
            manager.notify(SUMMARY_ID, summary)
        } catch (e: SecurityException) {
            // The permission was revoked meanwhile.
        }
    }

    companion object {
        const val CHANNEL = "messages"
        const val GROUP = "messages"

        /** Each room's notification: its ID as the tag, and this id. */
        const val ROOM_ID = 0
        const val SUMMARY_ID = 1

        /** Messages kept per room's notification. */
        const val KEPT = 6

        /** How long a sender's avatar may hold up the notification. */
        const val AVATAR_WAIT_MS = 3_000L
    }
}
