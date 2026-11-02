package org.mtrx.app.model

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mtrx.app.FakeMtrx
import org.mtrx.app.FakeSession
import org.mtrx.app.FakeSharePresenceStore
import org.mtrx.app.FakeRoomLists
import org.mtrx.app.SpacesFixture
import org.mtrx.app.makePresence
import org.mtrx.app.settings.sharePresenceFootnote
import org.mtrx.app.summary
import org.mtrx.app.testAccount
import uniffi.mtrx.MtrxException
import uniffi.mtrx.PresenceState
import uniffi.mtrx.RoomKind
import uniffi.mtrx.RoomLists

@OptIn(ExperimentalCoroutinesApi::class)
class PresenceTest {
    @Test
    fun theSettingsFootnoteSaysWhenTheServerDoesNotSharePresence() {
        assertEquals("Others see when you are online or idle.", sharePresenceFootnote(available = true))
        assertEquals("This server does not share presence.", sharePresenceFootnote(available = false))
    }

    // An account restored on [session], in the foreground.
    private fun TestScope.signedIn(
        session: FakeSession,
        store: FakeSharePresenceStore = FakeSharePresenceStore(),
        foreground: Boolean = true,
    ): Account {
        val account = testAccount(FakeMtrx(restored = session), CoroutineScope(UnconfinedTestDispatcher(testScheduler)), sharePresence = store)
        account.start()
        if (foreground) account.setForeground(true)
        return account
    }

    private val Account.sessionPresence: PresenceModel get() = (state.value as AccountState.SignedIn).presence

    private fun TestScope.settle() = advanceTimeBy(PresenceModel.FOLLOW_DEBOUNCE_MS + 1)

    private fun FakeSession.rooms(lists: RoomLists) = roomListListener!!.onUpdate(lists)

    private fun withDan(lists: RoomLists) =
        FakeRoomLists(rooms = lists.rooms() + summary("!with-dan:x", RoomKind.DIRECT, directUserId = "@dan:x"))

    @Test
    fun theDirectChatsPeopleAreReadOnceTheRoomListIsLoaded() = runTest {
        val session = FakeSession()
        signedIn(session)
        session.rooms(SpacesFixture.lists(loaded = false))
        settle()
        assertTrue(session.presenceSubscriptions.isEmpty())

        session.rooms(SpacesFixture.lists())
        settle()
        // Not the Notes to self; the invite to a direct chat too.
        assertEquals(listOf(listOf("@bob:x", "@carol:x")), session.presenceSubscriptions.map { it.first })
    }

    @Test
    fun theSubscriptionIsReplacedOnlyWhenThePeopleChange() = runTest {
        val session = FakeSession()
        signedIn(session)
        val fixture = SpacesFixture.lists()
        session.rooms(fixture)
        settle()
        val first = session.presenceSubscriptions.single().second

        // Other changes (an unread count, a room) keep it.
        session.rooms(FakeRoomLists(rooms = fixture.rooms().map { if (it.id == SpacesFixture.WITH_BOB) it.copy(unreadMessages = 9uL) else it }))
        session.rooms(FakeRoomLists(rooms = fixture.rooms() + summary("!new-room:x")))
        settle()
        assertEquals(1, session.presenceSubscriptions.size)

        // A new direct chat: subscribed anew, then the old one ends.
        session.rooms(withDan(fixture))
        settle()
        assertEquals(listOf("@bob:x", "@carol:x", "@dan:x"), session.presenceSubscriptions.last().first)
        assertTrue(first.cancelled)
        assertFalse(session.presenceSubscriptions.last().second.cancelled)
    }

    @Test
    fun changesInABurstSubscribeOnceForWhereTheySettle() = runTest {
        val session = FakeSession()
        signedIn(session)
        session.rooms(SpacesFixture.lists())
        settle()
        session.rooms(withDan(SpacesFixture.lists()))
        advanceTimeBy(PresenceModel.FOLLOW_DEBOUNCE_MS / 2)
        session.rooms(SpacesFixture.lists())
        settle()
        // Back where it was: nothing to replace.
        assertEquals(1, session.presenceSubscriptions.size)

        session.rooms(withDan(SpacesFixture.lists()))
        advanceTimeBy(PresenceModel.FOLLOW_DEBOUNCE_MS / 2)
        assertEquals(1, session.presenceSubscriptions.size)
        settle()
        assertEquals(2, session.presenceSubscriptions.size)
    }

    @Test
    fun presencesArriveInTheMapAndTheSheetReadsAFresherOne() = runTest {
        val session = FakeSession()
        val account = signedIn(session)
        session.rooms(SpacesFixture.lists())
        settle()
        val bob = makePresence(PresenceState.ONLINE)
        session.presenceListener!!.onUpdate(mapOf("@bob:x" to bob))
        assertEquals(mapOf("@bob:x" to bob), account.sessionPresence.presences.value)

        // The person sheet opens: fresher, and for someone not subscribed to.
        val idle = makePresence(PresenceState.UNAVAILABLE)
        session.presences["@bob:x"] = idle
        account.sessionPresence.refresh("@bob:x")
        assertEquals(idle, account.sessionPresence.presences.value["@bob:x"])

        // A failure for one user keeps what is known, and hides nothing.
        session.presenceError = MtrxException.Other("You are not allowed to see their presence")
        account.sessionPresence.refresh("@bob:x")
        assertEquals(idle, account.sessionPresence.presences.value["@bob:x"])
        assertTrue(account.sessionPresence.available.value)
    }

    @Test
    fun readingStopsInTheBackgroundAndAtSignOut() = runTest {
        val session = FakeSession()
        val account = signedIn(session)
        session.rooms(SpacesFixture.lists())
        settle()
        val first = session.presenceSubscriptions.single().second

        account.setForeground(false)
        assertTrue(first.cancelled)
        // The last map stays.
        account.setForeground(true)
        assertEquals(2, session.presenceSubscriptions.size)
        assertEquals(listOf("@bob:x", "@carol:x"), session.presenceSubscriptions.last().first)

        account.signOut()
        assertTrue(session.presenceSubscriptions.all { it.second.cancelled })
        // Nothing more after sign-out.
        session.rooms(withDan(SpacesFixture.lists()))
        settle()
        assertEquals(2, session.presenceSubscriptions.size)
    }

    @Test
    fun theLibraryHearsOfTheForegroundAndOfTheBackgroundBeforeSyncStops() = runTest {
        val session = FakeSession()
        val account = signedIn(session, foreground = false)
        assertTrue(session.calls.isEmpty())

        // "Share my presence" goes first, once.
        account.setForeground(true)
        assertEquals(listOf("sharing true", "presence foreground"), session.presenceCalls)
        assertTrue("startSync" in session.calls)
        account.setForeground(false)
        assertEquals(listOf("presence background", "stopSync"), session.calls.takeLast(2))
        account.setForeground(true)
        assertEquals(listOf("presence background", "presence foreground"), session.presenceCalls.takeLast(2))
        assertEquals("startSync", session.calls.last { it.endsWith("Sync") })
    }

    @Test
    fun sharingSyncDoesNotWaitForOurPresence() = runTest {
        val session = FakeSession().apply { ownPresenceGate = CompletableDeferred() }
        signedIn(session)
        // Sync started while "Share my presence" still waits.
        assertEquals(setOf("sharing true", "startSync"), session.calls.toSet())
        session.ownPresenceGate!!.complete(Unit)
        assertEquals("presence foreground", session.calls.last())
    }

    @Test
    fun notSharingSyncWaitsUntilWeAreOffline() = runTest {
        val session = FakeSession().apply { ownPresenceGate = CompletableDeferred() }
        signedIn(session, FakeSharePresenceStore(saved = false))
        assertEquals(listOf("sharing false"), session.calls)
        session.ownPresenceGate!!.complete(Unit)
        assertEquals(listOf("sharing false", "presence foreground", "startSync"), session.calls)
    }

    @Test
    fun notSharingIsPassedBeforeTheFirstSync() = runTest {
        val session = FakeSession()
        val account = signedIn(session, FakeSharePresenceStore(saved = false))
        assertFalse(account.sharePresence.on.value)
        assertEquals(listOf("sharing false", "presence foreground", "startSync"), session.calls)
        // Others' presence is still read.
        session.rooms(SpacesFixture.lists())
        settle()
        assertEquals(1, session.presenceSubscriptions.size)
    }

    @Test
    fun signingOutGoesOfflineFirst() = runTest {
        val session = FakeSession()
        session.onLogout = { session.calls += "logout" }
        val account = signedIn(session)
        account.signOut()
        assertEquals(listOf("presence sign_out", "presence background", "stopSync", "logout"), session.calls.takeLast(4))
    }

    @Test
    fun aFailedSignOutComesBackToTheForeground() = runTest {
        val session = FakeSession().apply { logoutError = MtrxException.Other("offline") }
        val account = signedIn(session)
        account.signOut()
        assertEquals("presence foreground", session.presenceCalls.last())
    }

    @Test
    fun forgettingTheSessionTellsNothing() = runTest {
        val session = FakeSession()
        val account = signedIn(session)
        account.forget()
        assertEquals(listOf("sharing true", "presence foreground"), session.presenceCalls)
        assertTrue(session.forgotten)
    }

    @Test
    fun inputInTheForegroundIsPassedOn() = runTest {
        val session = FakeSession()
        val account = signedIn(session)
        account.input()
        account.input()
        assertEquals(listOf("presence input", "presence input"), session.presenceCalls.takeLast(2))

        account.setForeground(false)
        account.input()
        assertEquals("presence background", session.presenceCalls.last())
    }

    @Test
    fun theSwitchIsPassedOnAndKept() = runTest {
        val session = FakeSession()
        val store = FakeSharePresenceStore()
        val account = signedIn(session, store)
        assertTrue(account.sharePresence.on.value)

        account.setSharePresence(false)
        assertEquals(false, store.saved)
        assertEquals("sharing false", session.presenceCalls.last())
        account.setSharePresence(true)
        assertEquals(true, store.saved)
        assertEquals("sharing true", session.presenceCalls.last())
        // Unchanged: nothing to say.
        val told = session.presenceCalls.size
        account.setSharePresence(true)
        assertEquals(told, session.presenceCalls.size)

        // A device preference: kept at sign-out.
        account.signOut()
        assertEquals(true, store.saved)
        assertTrue(account.sharePresence.on.value)
    }

    @Test
    fun failuresToSetOursAreIgnored() = runTest {
        val session = FakeSession().apply { ownPresenceError = MtrxException.Other("The server is unreachable") }
        val account = signedIn(session)
        account.setForeground(false)
        assertEquals(listOf("presence background", "stopSync"), session.calls.takeLast(2))
        assertTrue(account.sessionPresence.available.value)
    }

    @Test
    fun onUnavailablePresenceIsHiddenForTheSession() = runTest {
        val session = FakeSession()
        val account = signedIn(session)
        session.rooms(SpacesFixture.lists())
        settle()
        session.presenceListener!!.onUpdate(mapOf("@bob:x" to makePresence(PresenceState.ONLINE)))

        session.presenceListener!!.onUnavailable()
        assertFalse(account.sessionPresence.available.value)
        assertTrue(account.sessionPresence.presences.value.isEmpty())
        assertTrue(session.presenceSubscriptions.single().second.cancelled)

        // Nothing read or set after.
        session.presenceListener!!.onUpdate(mapOf("@bob:x" to makePresence(PresenceState.ONLINE)))
        assertTrue(account.sessionPresence.presences.value.isEmpty())
        session.rooms(withDan(SpacesFixture.lists()))
        settle()
        account.sessionPresence.refresh("@bob:x")
        account.setForeground(false)
        account.setForeground(true)
        account.setSharePresence(false)
        account.input()
        assertEquals(1, session.presenceSubscriptions.size)
        assertTrue(session.presenceReads.isEmpty())
        assertEquals(listOf("sharing true", "presence foreground"), session.presenceCalls)
    }

    @Test
    fun theServersErrorHidesPresenceToo() = runTest {
        val session = FakeSession()
        val account = signedIn(session)
        session.rooms(SpacesFixture.lists())
        settle()
        session.presenceError = MtrxException.PresenceUnavailable()
        account.sessionPresence.refresh("@bob:x")
        assertFalse(account.sessionPresence.available.value)
        assertTrue(session.presenceSubscriptions.single().second.cancelled)

        // Setting ours failing the same way hides it as well.
        val other = FakeSession().apply { ownPresenceError = MtrxException.PresenceUnavailable() }
        val otherAccount = signedIn(other)
        assertFalse(otherAccount.sessionPresence.available.value)
        otherAccount.setForeground(false)
        assertEquals(listOf("sharing true"), other.presenceCalls)
    }

    @Test
    fun aNewSessionSharesPresenceAgain() = runTest {
        val session = FakeSession()
        val mtrx = FakeMtrx(restored = session)
        val account = testAccount(mtrx, CoroutineScope(UnconfinedTestDispatcher(testScheduler)))
        account.start()
        account.setForeground(true)
        session.rooms(SpacesFixture.lists())
        settle()
        session.presenceListener!!.onUnavailable()
        assertFalse(account.sessionPresence.available.value)

        account.signOut()
        val next = FakeSession()
        mtrx.loginSession = next
        account.login("example.org", "me", "secret")
        assertTrue(account.sessionPresence.available.value)
        assertEquals(listOf("sharing true", "presence foreground"), next.presenceCalls)
    }
}
