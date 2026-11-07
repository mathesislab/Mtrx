package org.mtrx.app.model

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mtrx.app.FakeDistributors
import org.mtrx.app.FakeMtrx
import org.mtrx.app.FakeNetwork
import org.mtrx.app.FakeRoomLists
import org.mtrx.app.FakeNotifier
import org.mtrx.app.FakePushStore
import org.mtrx.app.FakeSession
import org.mtrx.app.pushRegistration
import org.mtrx.app.signedIn
import org.mtrx.app.summary
import org.mtrx.app.testAccount
import uniffi.mtrx.MtrxException

@OptIn(ExperimentalCoroutinesApi::class)
class AccountTest {
    @Test
    fun withoutASavedSessionItIsSignedOut() = runTest {
        val account = testAccount(FakeMtrx(restored = null), CoroutineScope(UnconfinedTestDispatcher(testScheduler)))
        account.start()
        assertEquals(AccountState.SignedOut(), account.state.value)
    }

    @Test
    fun restoredSessionSyncsInTheForegroundOnly() = runTest {
        val session = FakeSession()
        val account = testAccount(FakeMtrx(restored = session), CoroutineScope(UnconfinedTestDispatcher(testScheduler)))
        account.start()
        assertTrue(account.state.value is AccountState.SignedIn)
        assertEquals(0, session.syncStarts)

        account.setForeground(true)
        account.setForeground(true)
        assertEquals(1, session.syncStarts)
        account.setForeground(false)
        assertEquals(1, session.syncStops)
    }

    @Test
    fun theNetworkBackReconnectsWhileInTheForeground() = runTest {
        val session = FakeSession()
        val network = FakeNetwork()
        val account = testAccount(
            FakeMtrx(restored = session),
            CoroutineScope(UnconfinedTestDispatcher(testScheduler)),
            network = network,
        )
        account.start()
        account.setForeground(true)
        network.back!!.invoke()
        assertEquals(1, session.reconnects)

        account.setForeground(false)
        assertNull(network.back)
        account.signOut()
        account.setForeground(true)
        // Signed out: nothing to reconnect.
        network.back!!.invoke()
        assertEquals(1, session.reconnects)
    }

    @Test
    fun roomListUpdatesBecomeState() = runTest {
        val session = FakeSession()
        val account = testAccount(FakeMtrx(restored = session), CoroutineScope(UnconfinedTestDispatcher(testScheduler)))
        account.start()
        val rooms = account.signedIn.rooms
        assertFalse(rooms.value.loaded())
        session.roomListListener!!.onUpdate(FakeRoomLists())
        assertTrue(rooms.value.loaded())
    }

    @Test
    fun signingOutStopsSyncAndSubscriptions() = runTest {
        val session = FakeSession()
        val account = testAccount(FakeMtrx(restored = session), CoroutineScope(UnconfinedTestDispatcher(testScheduler)))
        account.start()
        account.setForeground(true)
        account.signOut()
        assertTrue(session.loggedOut)
        assertEquals(1, session.syncStops)
        assertTrue(session.handles.all { it.cancelled })
        assertEquals(AccountState.SignedOut(), account.state.value)
        // Back in the foreground, nothing syncs.
        account.setForeground(false)
        account.setForeground(true)
        assertEquals(1, session.syncStarts)
    }

    @Test
    fun failedSignOutKeepsTheSessionSyncing() = runTest {
        val session = FakeSession().apply { logoutError = MtrxException.Other("offline") }
        val account = testAccount(FakeMtrx(restored = session), CoroutineScope(UnconfinedTestDispatcher(testScheduler)))
        account.start()
        account.setForeground(true)
        account.signOut()
        assertTrue(account.state.value is AccountState.SignedIn)
        assertEquals(2, session.syncStarts)
        assertFalse(session.handles.any { it.cancelled })
    }

    @Test
    fun passwordSignIn() = runTest {
        val session = FakeSession()
        val mtrx = FakeMtrx(restored = null).apply { loginSession = session }
        val account = testAccount(mtrx, CoroutineScope(UnconfinedTestDispatcher(testScheduler)))
        account.start()
        account.setForeground(true)
        account.login("example.org", "me", "secret")
        assertTrue(account.state.value is AccountState.SignedIn)
        assertEquals(1, session.syncStarts)
    }

    private val endpoint = "https://ntfy.sh/upAbC?up=1"

    @Test
    fun signingInRegistersForPushes() = runTest {
        val session = FakeSession()
        val mtrx = FakeMtrx(restored = null).apply { loginSession = session }
        val scope = CoroutineScope(UnconfinedTestDispatcher(testScheduler))
        val distributors = FakeDistributors()
        val push = pushRegistration(scope, distributors)
        val account = testAccount(mtrx, scope, push = push)
        account.start()
        assertEquals(0, distributors.registrations)
        account.login("example.org", "me", "secret")
        assertEquals(1, distributors.registrations)
        push.onNewEndpoint(endpoint)
        assertEquals(listOf(endpoint), session.pushers().map { it.pushkey })
    }

    @Test
    fun aRestoredSessionSetsItsPusherAgain() = runTest {
        val session = FakeSession()
        val scope = CoroutineScope(UnconfinedTestDispatcher(testScheduler))
        val push = pushRegistration(scope, FakeDistributors(saved = FakeDistributors.NTFY), FakePushStore(endpoint = endpoint))
        testAccount(FakeMtrx(restored = session), scope, push = push).start()
        assertEquals(listOf("set $endpoint"), session.pusherCalls)
    }

    @Test
    fun signingOutDeletesThePusherAndUnregistersBeforeLoggingOut() = runTest {
        val session = FakeSession()
        val scope = CoroutineScope(UnconfinedTestDispatcher(testScheduler))
        val distributors = FakeDistributors(saved = FakeDistributors.NTFY)
        val store = FakePushStore(endpoint = endpoint)
        val notifications = FakeNotifier()
        val account = testAccount(
            FakeMtrx(restored = session),
            scope,
            push = pushRegistration(scope, distributors, store),
            notifications = notifications,
        )
        account.start()
        val calls = mutableListOf<String>()
        distributors.onUnregister = {
            calls += session.pusherCalls.last()
            calls += "unregister"
        }
        session.onLogout = { calls += "logout" }
        account.signOut()
        assertEquals(listOf("delete $endpoint", "unregister", "logout"), calls)
        assertNull(store.endpoint)
        assertEquals(1, notifications.clearedAll)
    }

    @Test
    fun aFailedSignOutRegistersAgain() = runTest {
        val session = FakeSession().apply { logoutError = MtrxException.Other("offline") }
        val scope = CoroutineScope(UnconfinedTestDispatcher(testScheduler))
        val distributors = FakeDistributors(saved = FakeDistributors.NTFY)
        val account = testAccount(FakeMtrx(restored = session), scope, push = pushRegistration(scope, distributors))
        account.start()
        account.signOut()
        assertEquals(1, distributors.unregistrations)
        assertEquals(2, distributors.registrations)
    }

    @Test
    fun forgettingUnregistersToo() = runTest {
        val session = FakeSession()
        val scope = CoroutineScope(UnconfinedTestDispatcher(testScheduler))
        val distributors = FakeDistributors(saved = FakeDistributors.NTFY)
        val store = FakePushStore(endpoint = endpoint)
        val account = testAccount(FakeMtrx(restored = session), scope, push = pushRegistration(scope, distributors, store))
        account.start()
        account.forget()
        assertTrue(session.forgotten)
        assertEquals(1, distributors.unregistrations)
        assertNull(store.endpoint)
    }

    @Test
    fun pushesSkipOnlyTheRoomOnScreenInTheForeground() = runTest {
        val account = testAccount(FakeMtrx(restored = FakeSession()), CoroutineScope(UnconfinedTestDispatcher(testScheduler)))
        account.start()
        account.roomOpened("!a:x")
        // In the background nothing is on screen.
        assertNull(account.foregroundRoom())
        account.setForeground(true)
        assertEquals("!a:x", account.foregroundRoom())
        // Another room's screen leaving does not matter.
        account.roomClosed("!b:x")
        assertEquals("!a:x", account.foregroundRoom())
        account.roomClosed("!a:x")
        assertNull(account.foregroundRoom())
    }

    @Test
    fun openingOrReadingARoomClearsItsNotifications() = runTest {
        val notifications = FakeNotifier()
        val account = testAccount(
            FakeMtrx(restored = FakeSession()),
            CoroutineScope(UnconfinedTestDispatcher(testScheduler)),
            notifications = notifications,
        )
        account.start()
        account.roomOpened("!a:x")
        account.roomRead("!b:x")
        assertEquals(listOf("!a:x", "!b:x"), notifications.cleared)
    }

    @Test
    fun roomsReadElsewhereLoseTheirNotifications() = runTest {
        val session = FakeSession()
        val notifications = FakeNotifier()
        val account = testAccount(
            FakeMtrx(restored = session),
            CoroutineScope(UnconfinedTestDispatcher(testScheduler)),
            notifications = notifications,
        )
        account.start()
        val listener = session.roomListListener!!
        listener.onUpdate(FakeRoomLists(rooms = listOf(summary("!a:x", unread = 2), summary("!b:x", unread = 1))))
        assertTrue(notifications.cleared.isEmpty())
        listener.onUpdate(FakeRoomLists(rooms = listOf(summary("!a:x"), summary("!b:x", unread = 1))))
        assertEquals(listOf("!a:x"), notifications.cleared)
    }

    @Test
    fun aTappedNotificationsRoomIsRequestedUntilShownOrSignedOut() = runTest {
        val account = testAccount(FakeMtrx(restored = FakeSession()), CoroutineScope(UnconfinedTestDispatcher(testScheduler)))
        account.start()
        account.requestRoom("!a:x")
        assertEquals("!a:x", account.roomRequest.value)
        account.roomRequestHandled()
        assertNull(account.roomRequest.value)
        account.requestRoom("!b:x")
        account.signOut()
        assertNull(account.roomRequest.value)
    }
}
