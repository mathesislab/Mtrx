package org.mtrx.app.model

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mtrx.app.FakeRoomLists
import org.mtrx.app.FakeDistributors
import org.mtrx.app.FakeDistributors.Companion.NTFY
import org.mtrx.app.FakeDistributors.Companion.OTHER
import org.mtrx.app.FakePushStore
import org.mtrx.app.FakeSession
import org.mtrx.app.pushRegistration
import org.mtrx.app.summary
import uniffi.mtrx.MtrxException
import uniffi.mtrx.PusherInfo

@OptIn(ExperimentalCoroutinesApi::class)
class PushRegistrationTest {
    private val endpoint = "https://ntfy.sh/upAbC?up=1"
    private val ntfy = Distributor(NTFY, "ntfy")

    private fun TestScope.registration(
        distributors: FakeDistributors = FakeDistributors(),
        store: FakePushStore = FakePushStore(),
    ) = pushRegistration(CoroutineScope(UnconfinedTestDispatcher(testScheduler)), distributors, store)

    @Test
    fun theOnlyDistributorIsSavedAndTheEndpointBecomesThePusher() = runTest {
        val distributors = FakeDistributors()
        val store = FakePushStore()
        val push = registration(distributors, store)
        val session = FakeSession()
        push.opened(session)
        assertEquals(NTFY, distributors.saved)
        assertEquals(1, distributors.registrations)
        assertEquals(PushState.Registering(ntfy), push.state.value)

        push.onNewEndpoint(endpoint)
        assertEquals(
            listOf(
                PusherInfo(
                    pushkey = endpoint,
                    appId = "org.mtrx.app",
                    appName = "Mtrx",
                    deviceName = "Pixel 9",
                    lang = "en-GB",
                    gatewayUrl = "https://ntfy.sh/_matrix/push/v1/notify",
                    eventIdOnly = true,
                ),
            ),
            session.pushers(),
        )
        assertEquals(endpoint, store.endpoint)
        assertEquals(PushState.Registered(ntfy), push.state.value)
    }

    @Test
    fun theGatewayIsAtTheEndpointsOrigin() {
        assertEquals(
            "https://push.example.org:8443/_matrix/push/v1/notify",
            originGateway("https://push.example.org:8443/up/abc?up=1"),
        )
        assertEquals("http://10.0.2.2:9/_matrix/push/v1/notify", originGateway("http://10.0.2.2:9/upTEST?up=1"))
        assertNull(originGateway("not a url"))
    }

    @Test
    fun eachStartSetsThePusherAgainAndRegisters() = runTest {
        val distributors = FakeDistributors(saved = NTFY)
        val store = FakePushStore(endpoint = endpoint)
        val session = FakeSession()
        registration(distributors, store).apply { opened(session) }.also {
            assertEquals(PushState.Registered(ntfy), it.state.value)
        }
        assertEquals(listOf("set $endpoint"), session.pusherCalls)
        assertEquals(1, distributors.registrations)
        // The distributor answers with the same endpoint: set again, nothing deleted.
        val push = registration(distributors, store)
        push.opened(session)
        push.onNewEndpoint(endpoint)
        assertEquals(listOf("set $endpoint", "set $endpoint", "set $endpoint"), session.pusherCalls)
    }

    @Test
    fun aChangedEndpointReplacesThePusher() = runTest {
        val session = FakeSession()
        val push = registration(FakeDistributors(saved = NTFY), FakePushStore(endpoint = endpoint))
        push.opened(session)
        push.onNewEndpoint("https://ntfy.sh/upNEW?up=1")
        assertEquals(
            listOf("set $endpoint", "delete $endpoint", "set https://ntfy.sh/upNEW?up=1"),
            session.pusherCalls,
        )
        assertEquals(listOf("https://ntfy.sh/upNEW?up=1"), session.pushers().map { it.pushkey })
    }

    @Test
    fun anEndpointBeforeTheSessionIsSetWhenItOpens() = runTest {
        val store = FakePushStore()
        val push = registration(FakeDistributors(saved = NTFY), store)
        push.onNewEndpoint(endpoint)
        assertEquals(endpoint, store.endpoint)
        val session = FakeSession()
        push.opened(session)
        assertEquals(listOf("set $endpoint"), session.pusherCalls)
    }

    @Test
    fun withoutADistributorNothingRegistersUntilOneIsInstalled() = runTest {
        val distributors = FakeDistributors(installed = emptyList())
        val push = registration(distributors)
        push.opened(FakeSession())
        assertEquals(PushState.NoDistributor, push.state.value)
        assertEquals(0, distributors.registrations)

        distributors.installed = listOf(NTFY)
        push.refresh()
        assertEquals(PushState.Registering(ntfy), push.state.value)
        assertEquals(1, distributors.registrations)
    }

    @Test
    fun severalDistributorsWithoutADefaultAreOffered() = runTest {
        val distributors = FakeDistributors(installed = listOf(NTFY, OTHER))
        val push = registration(distributors)
        push.opened(FakeSession())
        assertEquals(PushState.Choose(listOf(ntfy, Distributor(OTHER, "push"))), push.state.value)
        assertEquals(0, distributors.registrations)

        push.choose(OTHER)
        assertEquals(OTHER, distributors.saved)
        assertEquals(1, distributors.registrations)
        assertEquals(PushState.Registering(Distributor(OTHER, "push")), push.state.value)
    }

    @Test
    fun theSystemsDefaultDistributorIsTaken() = runTest {
        val distributors = FakeDistributors(installed = listOf(NTFY, OTHER), default = OTHER)
        registration(distributors).opened(FakeSession())
        assertEquals(OTHER, distributors.saved)
        assertEquals(1, distributors.registrations)
    }

    @Test
    fun closingDeletesThePusherThenUnregisters() = runTest {
        val distributors = FakeDistributors(saved = NTFY)
        val store = FakePushStore(endpoint = endpoint)
        val session = FakeSession()
        val calls = mutableListOf<String>()
        distributors.onUnregister = { calls += session.pusherCalls.last() + ", unregister" }
        val push = registration(distributors, store)
        push.opened(session)
        push.closing(session)
        assertEquals(listOf("delete $endpoint, unregister"), calls)
        assertTrue(session.pushers().isEmpty())
        assertNull(store.endpoint)
        assertEquals(PushState.Off, push.state.value)
        // Kept for the next sign-in.
        assertTrue(store.enabled)
    }

    @Test
    fun turningOffStopsThePushesAndIsRemembered() = runTest {
        val distributors = FakeDistributors(saved = NTFY)
        val store = FakePushStore(endpoint = endpoint)
        val session = FakeSession()
        val push = registration(distributors, store)
        push.opened(session)
        push.setEnabled(false)
        assertFalse(push.enabled.value)
        assertEquals(PushState.Off, push.state.value)
        assertTrue(session.pushers().isEmpty())
        assertEquals(1, distributors.unregistrations)
        assertNull(store.endpoint)

        // The next start leaves it off.
        val next = registration(distributors, store)
        val nextSession = FakeSession()
        next.opened(nextSession)
        assertEquals(PushState.Off, next.state.value)
        assertTrue(nextSession.pusherCalls.isEmpty())
        assertEquals(1, distributors.registrations)

        // On again: registers.
        next.setEnabled(true)
        assertTrue(store.enabled)
        assertEquals(2, distributors.registrations)
        next.onNewEndpoint(endpoint)
        assertEquals(listOf("set $endpoint"), nextSession.pusherCalls)
    }

    @Test
    fun anEndpointAfterTurningOffIsIgnored() = runTest {
        val store = FakePushStore()
        val session = FakeSession()
        val push = registration(FakeDistributors(), store)
        push.opened(session)
        push.setEnabled(false)
        push.onNewEndpoint(endpoint)
        assertNull(store.endpoint)
        assertTrue(session.pusherCalls.isEmpty())
    }

    @Test
    fun failuresShowInSettings() = runTest {
        val session = FakeSession()
        val push = registration()
        push.opened(session)
        push.onRegistrationFailed("no network")
        assertEquals(PushState.Failed("no network"), push.state.value)

        session.pusherError = MtrxException.Other("forbidden")
        push.onNewEndpoint(endpoint)
        assertEquals(PushState.Failed("forbidden"), push.state.value)
    }

    @Test
    fun unregisteredByTheDistributorDeletesThePusher() = runTest {
        val store = FakePushStore(endpoint = endpoint)
        val session = FakeSession()
        val push = registration(FakeDistributors(saved = NTFY), store)
        push.opened(session)
        push.onUnregistered()
        assertTrue(session.pushers().isEmpty())
        assertNull(store.endpoint)
        assertTrue(push.state.value is PushState.Failed)
    }

    @Test
    fun aPushWithoutSessionUnregisters() = runTest {
        val distributors = FakeDistributors(saved = NTFY)
        val store = FakePushStore(endpoint = endpoint)
        val push = registration(distributors, store)
        push.signedOut()
        assertEquals(1, distributors.unregistrations)
        assertNull(store.endpoint)
    }

    @Test
    fun thePermissionIsAskedOnce() = runTest {
        val push = registration()
        assertTrue(push.askPermissionOnce())
        assertFalse(push.askPermissionOnce())
    }

    @Test
    fun roomsReadAreThoseWhoseCountDroppedToZero() {
        val before = FakeRoomLists(rooms = listOf(summary("!a:x", unread = 2), summary("!b:x", unread = 1), summary("!c:x")))
        val after = FakeRoomLists(
            rooms = listOf(summary("!a:x"), summary("!b:x", unread = 3), summary("!c:x"), summary("!new:x")),
        )
        assertEquals(listOf("!a:x"), roomsRead(before, after))
    }
}
