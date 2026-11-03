package org.mtrx.app.push

import java.time.Instant
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mtrx.app.FakeMtrx
import org.mtrx.app.FakeNotifier
import org.mtrx.app.user
import uniffi.mtrx.MtrxException
import uniffi.mtrx.Notification

@OptIn(ExperimentalCoroutinesApi::class)
class PushHandlerTest {
    private val room = "!abc:example.org"

    private fun push(eventId: String? = "\$x7", unread: Int = 2): ByteArray {
        val event = eventId?.let { "\"event_id\": \"$it\", " } ?: ""
        return """
            {"notification": {$event"room_id": "$room", "counts": {"unread": $unread}, "prio": "high",
              "devices": [{"app_id": "org.mtrx.app", "pushkey": "https://ntfy.sh/upAbC?up=1",
                           "data": {"format": "event_id_only"}}]}}
        """.trimIndent().toByteArray()
    }

    private fun content(
        noisy: Boolean = true,
        direct: Boolean = false,
    ) = Notification(
        roomId = room,
        eventId = "\$x7",
        roomName = "Lab",
        direct = direct,
        sender = user("@alice:example.org", "Alice"),
        body = "@bob:example.org hi",
        highlight = true,
        noisy = noisy,
        timestamp = Instant.ofEpochMilli(1_700_000_000_000),
    )

    private class Setup {
        val mtrx = FakeMtrx(restored = null)
        val notifier = FakeNotifier()
        var onScreen: String? = null
        var signedOut = 0
        val handler = PushHandler(mtrx, notifier, foregroundRoom = { onScreen }, onSignedOut = { signedOut++ })
    }

    @Test
    fun parsesTheGatewaysNotification() {
        assertEquals(Push.Event(room, "\$x7"), parsePush(push()))
        assertEquals(Push.Counts(0), parsePush(push(eventId = null, unread = 0)))
        assertEquals(
            Push.Counts(null),
            parsePush("""{"notification": {"room_id": "$room", "prio": "low"}}""".toByteArray()),
        )
    }

    @Test
    fun ignoresWhatIsNotAPush() {
        assertNull(parsePush("garbage".toByteArray()))
        assertNull(parsePush("""{"message": "hello"}""".toByteArray()))
        assertNull(parsePush(byteArrayOf()))
        assertNull(parsePush("""{"notification": {"counts": {"unread": "many"}}}""".toByteArray()))
    }

    @Test
    fun postsWhatThePushIsAbout() = runTest {
        val setup = Setup()
        setup.mtrx.notificationContent = content()
        setup.handler.handle(push())
        assertEquals(listOf(room to "\$x7"), setup.mtrx.notificationsRead)
        assertEquals(listOf(content()), setup.notifier.posted)
    }

    @Test
    fun postsNothingForNil() = runTest {
        val setup = Setup()
        setup.handler.handle(push())
        assertTrue(setup.notifier.posted.isEmpty())
        assertTrue(setup.notifier.fallbacks.isEmpty())
    }

    @Test
    fun skipsTheRoomOnScreen() = runTest {
        val setup = Setup()
        setup.mtrx.notificationContent = content()
        setup.onScreen = room
        setup.handler.handle(push())
        assertTrue(setup.notifier.posted.isEmpty())
        // Not for another room.
        setup.onScreen = "!other:example.org"
        setup.handler.handle(push())
        assertEquals(1, setup.notifier.posted.size)
    }

    @Test
    fun anErrorPostsNewMessage() = runTest {
        val setup = Setup()
        setup.mtrx.notificationError = MtrxException.Other("event not found")
        setup.handler.handle(push())
        assertEquals(listOf(room), setup.notifier.fallbacks)
        // Not while the room is on screen.
        setup.onScreen = room
        setup.handler.handle(push())
        assertEquals(listOf(room), setup.notifier.fallbacks)
    }

    @Test
    fun aSlowReadPostsNewMessageAfterTwentySeconds() = runTest {
        val setup = Setup()
        setup.mtrx.notificationGate = CompletableDeferred()
        setup.handler.handle(push())
        assertEquals(listOf(room), setup.notifier.fallbacks)
        assertEquals(20_000L, testScheduler.currentTime)
    }

    @Test
    fun withoutASessionItUnregisters() = runTest {
        val setup = Setup()
        setup.mtrx.notificationError = MtrxException.NotLoggedIn()
        setup.handler.handle(push())
        assertEquals(1, setup.signedOut)
        assertTrue(setup.notifier.fallbacks.isEmpty())
    }

    @Test
    fun aCountOfZeroClearsTheNotifications() = runTest {
        val setup = Setup()
        setup.handler.handle(push(eventId = null, unread = 3))
        assertEquals(0, setup.notifier.clearedAll)
        setup.handler.handle(push(eventId = null, unread = 0))
        assertEquals(1, setup.notifier.clearedAll)
        assertTrue(setup.mtrx.notificationsRead.isEmpty())
    }

    @Test
    fun garbageDoesNothing() = runTest {
        val setup = Setup()
        setup.handler.handle("garbage".toByteArray())
        assertTrue(setup.mtrx.notificationsRead.isEmpty())
        assertEquals(0, setup.notifier.clearedAll)
    }

    @Test
    fun aRoomsMessagesAreTitledWithItsNameUnlessDirect() {
        assertEquals(NotificationLook(title = "Lab", silent = false, at = 1_700_000_000_000), lookOf(content()))
        assertNull(lookOf(content(direct = true)).title)
        assertTrue(lookOf(content(noisy = false)).silent)
    }
}
