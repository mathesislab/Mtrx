package org.mtrx.app

import android.Manifest
import android.app.NotificationManager
import android.service.notification.StatusBarNotification
import androidx.core.app.NotificationCompat
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicReference
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.mtrx.app.model.AccountState
import uniffi.mtrx.EventContent
import uniffi.mtrx.Mtrx
import uniffi.mtrx.RoomLists
import uniffi.mtrx.RoomListsListener
import uniffi.mtrx.SecretBackend
import uniffi.mtrx.Session
import uniffi.mtrx.TimelineItem
import uniffi.mtrx.TimelineItemContent
import uniffi.mtrx.TimelineListener

/**
 * Push notifications end to end (docs/mobile.md, section 10), against the
 * Synapse that CI starts on the host (testserver/run.sh; the emulator
 * reaches it at 10.0.2.2). The test plays distributor and gateway: Bob
 * signs in through the app's account and registers a pusher the way the
 * app does; Alice, in a second `Mtrx`, mentions him in an encrypted room;
 * the gateway's payload goes to the app's push handler, which the push
 * service calls; the notification it posts is checked.
 */
@RunWith(AndroidJUnit4::class)
class PushEndToEndTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val app = instrumentation.targetContext.applicationContext as MtrxApplication
    private val notifications = app.getSystemService(NotificationManager::class.java)

    @Test
    fun aMentionInAnEncryptedRoomIsNotified() = runBlocking<Unit> {
        instrumentation.uiAutomation.grantRuntimePermission(app.packageName, Manifest.permission.POST_NOTIFICATIONS)
        val suffix = UUID.randomUUID().toString().take(8)
        val bob = "bob-$suffix"
        val alice = "alice-$suffix"
        register(bob)
        register(alice)

        // Bob in the app, syncing as in the foreground.
        val account = app.account
        waitUntil("the account to load") { account.state.value != AccountState.Loading }
        if (account.state.value is AccountState.SignedIn) {
            withContext(Dispatchers.Main) { account.signOut().join() }
        }
        withContext(Dispatchers.Main) {
            account.login(HOMESERVER, bob, PASSWORD)
            account.setForeground(true).join()
        }
        val bobSession = (account.state.value as AccountState.SignedIn).session

        // Alice in a Mtrx of her own.
        val dir = File(app.cacheDir, "push-test-$suffix")
        val aliceMtrx = Mtrx(File(dir, "data").path, File(dir, "cache").path, MemorySecrets())
        val aliceSession = aliceMtrx.login(HOMESERVER, alice, PASSWORD)
        aliceSession.startSync()
        val aliceRooms = AtomicReference<RoomLists?>(null)
        val aliceRoomList = aliceSession.subscribeRoomLists(object : RoomListsListener {
            override fun onUpdate(lists: RoomLists) = aliceRooms.set(lists)
        })

        // The room, encrypted, made over HTTP (the bindings make no rooms).
        val aliceToken = loginToken(alice)
        val roomId = post(
            "/_matrix/client/v3/createRoom",
            aliceToken,
            JSONObject()
                .put("preset", "private_chat")
                .put("name", "Push test")
                .put("invite", JSONArray().put("@$bob:localhost"))
                .put(
                    "initial_state",
                    JSONArray().put(
                        JSONObject()
                            .put("type", "m.room.encryption")
                            .put("state_key", "")
                            .put("content", JSONObject().put("algorithm", "m.megolm.v1.aes-sha2")),
                    ),
                ),
        ).getString("room_id")
        post("/_matrix/client/v3/join/${URLEncoder.encode(roomId, "UTF-8")}", loginToken(bob), JSONObject())
        waitUntil("Alice to see the room encrypted with Bob in it") {
            aliceRooms.get()?.rooms()?.any { it.id == roomId && it.encrypted && it.joinedMembers == 2uL } == true
        }
        waitUntil("Bob's app to see the room") {
            account.rooms.value.rooms.any { it.id == roomId && !it.invited && it.encrypted }
        }

        // The pusher, as the app sets it for a new endpoint.
        app.push.onNewEndpoint(ENDPOINT).join()
        val pusher = bobSession.pushers().firstOrNull { it.pushkey == ENDPOINT }
        assertNotNull("the pusher is listed", pusher)
        assertTrue("the pusher asks for event IDs only", pusher!!.eventIdOnly)
        assertEquals("org.mtrx.app", pusher.appId)
        assertEquals("http://10.0.2.2:9/_matrix/push/v1/notify", pusher.gatewayUrl)

        // Alice mentions Bob.
        val text = "@$bob:localhost push $suffix"
        val eventId = send(aliceSession, roomId, text, "push $suffix")
        app.pushHandler.handle(gatewayPush(roomId, eventId))
        val posted = waitFor("the notification") { roomNotification(roomId) }
        val messages = NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(posted.notification)!!.messages
        assertEquals(text, messages.last().text.toString())
        assertEquals(alice, messages.last().person?.name.toString())

        // Opening the room clears it.
        withContext(Dispatchers.Main) { account.roomOpened(roomId) }
        waitUntil("the notification to go") { roomNotification(roomId) == null }
        withContext(Dispatchers.Main) { account.roomClosed(roomId) }

        // Bob's own message posts nothing. (Sending reads the room, so his
        // syncing app would clear a notification anyway: checked after the
        // clearing, with the room closed again.)
        val own = "own $suffix"
        val ownEventId = send(bobSession, roomId, own, own)
        app.pushHandler.handle(gatewayPush(roomId, ownEventId))
        delay(3_000)
        assertNull("no notification for Bob's own message", roomNotification(roomId))

        aliceRoomList.cancel()
        aliceRoomList.close()
        aliceSession.stopSync()
        aliceSession.close()
        aliceMtrx.close()
    }

    private fun roomNotification(roomId: String): StatusBarNotification? =
        notifications.activeNotifications.firstOrNull { it.tag == roomId }

    /** The gateway's body for [eventId], as the distributor delivers it. */
    private fun gatewayPush(roomId: String, eventId: String): ByteArray = JSONObject()
        .put(
            "notification",
            JSONObject()
                .put("event_id", eventId)
                .put("room_id", roomId)
                .put("counts", JSONObject().put("unread", 1))
                .put("prio", "high")
                .put(
                    "devices",
                    JSONArray().put(
                        JSONObject()
                            .put("app_id", "org.mtrx.app")
                            .put("pushkey", ENDPOINT)
                            .put("data", JSONObject().put("format", "event_id_only")),
                    ),
                ),
        )
        .toString()
        .toByteArray()

    /** Send [text] in [roomId] and return its event ID, found by [shown] in the sender's timeline. */
    private suspend fun send(session: Session, roomId: String, text: String, shown: String): String {
        val timeline = session.timeline(roomId)
        val latest = AtomicReference<List<TimelineItem>>(emptyList())
        val subscription = timeline.subscribe(object : TimelineListener {
            override fun onUpdate(items: List<TimelineItem>) {
                latest.set(items)
            }
        })
        timeline.sendText(text)
        val eventId = waitFor("the sent event's ID") {
            latest.get().firstNotNullOfOrNull { item ->
                val event = (item.content as? TimelineItemContent.Event)?.event ?: return@firstNotNullOfOrNull null
                val message = (event.content as? EventContent.Message)?.message ?: return@firstNotNullOfOrNull null
                if (event.own && message.body.contains(shown)) event.eventId else null
            }
        }
        subscription.cancel()
        subscription.close()
        timeline.close()
        return eventId
    }

    private suspend fun <T : Any> waitFor(what: String, timeoutMs: Long = 60_000, check: suspend () -> T?): T {
        val until = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < until) {
            check()?.let { return it }
            delay(250)
        }
        throw AssertionError("Timed out waiting for $what")
    }

    private suspend fun waitUntil(what: String, check: () -> Boolean) {
        waitFor(what) { if (check()) Unit else null }
    }

    /** A user, through Synapse's shared-secret registration (as testserver/add_user.sh). */
    private suspend fun register(name: String) {
        val nonce = request("GET", "/_synapse/admin/v1/register", null, null).getString("nonce")
        val mac = Mac.getInstance("HmacSHA1").apply {
            init(SecretKeySpec(SHARED_SECRET.toByteArray(), "HmacSHA1"))
        }.doFinal("$nonce\u0000$name\u0000$PASSWORD\u0000notadmin".toByteArray())
            .joinToString("") { "%02x".format(it) }
        post(
            "/_synapse/admin/v1/register",
            null,
            JSONObject()
                .put("nonce", nonce)
                .put("username", name)
                .put("password", PASSWORD)
                .put("admin", false)
                .put("mac", mac),
        )
    }

    /** An access token of a new device of [name]'s, for the client-server API. */
    private suspend fun loginToken(name: String): String = post(
        "/_matrix/client/v3/login",
        null,
        JSONObject()
            .put("type", "m.login.password")
            .put("identifier", JSONObject().put("type", "m.id.user").put("user", name))
            .put("password", PASSWORD),
    ).getString("access_token")

    private suspend fun post(path: String, token: String?, body: JSONObject): JSONObject =
        request("POST", path, token, body)

    private suspend fun request(method: String, path: String, token: String?, body: JSONObject?): JSONObject =
        withContext(Dispatchers.IO) {
            val connection = URL(HOMESERVER + path).openConnection() as HttpURLConnection
            try {
                connection.requestMethod = method
                token?.let { connection.setRequestProperty("Authorization", "Bearer $it") }
                if (body != null) {
                    connection.doOutput = true
                    connection.setRequestProperty("Content-Type", "application/json")
                    connection.outputStream.use { it.write(body.toString().toByteArray()) }
                }
                val code = connection.responseCode
                val stream = if (code in 200..299) connection.inputStream else connection.errorStream
                val text = stream?.use { it.readBytes().toString(Charsets.UTF_8) } ?: ""
                if (code !in 200..299) throw AssertionError("$method $path: $code $text")
                JSONObject(text)
            } finally {
                connection.disconnect()
            }
        }

    /** Secrets in memory, for Alice's `Mtrx`. */
    private class MemorySecrets : SecretBackend {
        private val secrets = ConcurrentHashMap<String, String>()

        override fun load(key: String): String? = secrets[key]

        override fun save(key: String, value: String) {
            secrets[key] = value
        }

        override fun delete(key: String) {
            secrets.remove(key)
        }
    }

    private companion object {
        const val HOMESERVER = "http://10.0.2.2:8008"
        const val PASSWORD = "mtrx-test-password"
        const val SHARED_SECRET = "mtrx-test-shared-secret"

        /** A UnifiedPush endpoint that no one serves: no transport in CI. */
        const val ENDPOINT = "http://10.0.2.2:9/upTEST?up=1"
    }
}
