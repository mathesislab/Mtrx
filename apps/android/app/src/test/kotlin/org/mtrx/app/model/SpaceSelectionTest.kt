package org.mtrx.app.model

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.mtrx.app.FakeMtrx
import org.mtrx.app.FakeSession
import org.mtrx.app.signedIn
import org.mtrx.app.testAccount
import org.mtrx.app.SpacesFixture
import org.mtrx.app.SpacesFixture.TEAM
import org.mtrx.app.FakeRoomLists
import org.mtrx.app.rooms.NoRooms
import uniffi.mtrx.RoomListsInterface
import uniffi.mtrx.SpaceKey

@OptIn(ExperimentalCoroutinesApi::class)
class SpaceSelectionTest {
    private fun TestScope.scope() = CoroutineScope(UnconfinedTestDispatcher(testScheduler))

    private fun TestScope.spaceSelection(store: MemoryStore<SpaceKey?>, rooms: MutableStateFlow<RoomListsInterface>) =
        SpaceSelection(Preference(store, null, scope()), rooms, scope())

    @Test
    fun withoutASavedEntryNothingIsSelected() = runTest {
        val selection = spaceSelection(MemoryStore(), MutableStateFlow<RoomListsInterface>(SpacesFixture.lists()))
        assertNull(selection.selected.value)
    }

    @Test
    fun theSelectedSpaceIsSavedAndRestoredAtTheNextLaunch() = runTest {
        val store = MemoryStore<SpaceKey?>()
        val first = spaceSelection(store, MutableStateFlow<RoomListsInterface>(SpacesFixture.lists()))
        first.select(SpaceKey.Space(TEAM))
        assertEquals(SpaceKey.Space(TEAM), first.selected.value)
        assertEquals(SpaceKey.Space(TEAM), store.saved)

        // The next launch starts without a list, then gets the cached one.
        val rooms = MutableStateFlow<RoomListsInterface>(NoRooms)
        val next = spaceSelection(store, rooms)
        assertNull(next.selected.value)
        rooms.value = SpacesFixture.lists(loaded = false)
        assertEquals(SpaceKey.Space(TEAM), next.selected.value)
        rooms.value = SpacesFixture.lists()
        assertEquals(SpaceKey.Space(TEAM), next.selected.value)
    }

    @Test
    fun aSavedSpaceThatIsGoneIsForgottenOnceTheListIsLoaded() = runTest {
        val left = SpaceKey.Space("!left:x")
        val store = MemoryStore<SpaceKey?>(left)
        val rooms = MutableStateFlow<RoomListsInterface>(SpacesFixture.lists(loaded = false))
        val selection = spaceSelection(store, rooms)
        assertNull(selection.selected.value)
        assertEquals("kept while the list may be a cache", left, store.saved)

        rooms.value = SpacesFixture.lists()
        assertNull(selection.selected.value)
        assertNull(store.saved)
    }

    @Test
    fun aSelectedEntryThatDisappearsIsDeselectedAndForgotten() = runTest {
        val store = MemoryStore<SpaceKey?>()
        val rooms = MutableStateFlow<RoomListsInterface>(SpacesFixture.lists())
        val selection = spaceSelection(store, rooms)
        selection.select(SpaceKey.Outside)
        assertEquals(SpaceKey.Outside, selection.selected.value)
        assertEquals(SpaceKey.Outside, store.saved)

        // Home lists none.
        rooms.value = FakeRoomLists(spaces = SpacesFixture.lists().spaces().filter { it.key != SpaceKey.Outside })
        assertNull(selection.selected.value)
        assertNull(store.saved)
        // Not selected again when it comes back.
        rooms.value = SpacesFixture.lists()
        assertNull(selection.selected.value)
    }

    @Test
    fun signingOutForgetsTheSelectedSpace() = runTest {
        val store = MemoryStore<SpaceKey?>(SpaceKey.Space(TEAM))
        val session = FakeSession()
        val mtrx = FakeMtrx(restored = session)
        val account = testAccount(mtrx, scope(), spaces = store)
        account.start()
        session.roomListListener!!.onUpdate(SpacesFixture.lists())
        assertEquals("restored", SpaceKey.Space(TEAM), account.signedIn.spaceSelection.selected.value)

        account.signOut()
        assertNull(store.saved)
        // The next sign-in starts without it.
        val next = FakeSession()
        mtrx.loginSession = next
        account.login("example.org", "me", "secret")
        next.roomListListener!!.onUpdate(SpacesFixture.lists())
        assertNull(account.signedIn.spaceSelection.selected.value)
    }
}
