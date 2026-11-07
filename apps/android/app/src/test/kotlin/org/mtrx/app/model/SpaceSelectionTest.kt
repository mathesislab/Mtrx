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
import org.mtrx.app.FakeSortStore
import org.mtrx.app.FakeSpaceStore
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

    @Test
    fun withoutASavedEntryNothingIsSelected() = runTest {
        val selection = SpaceSelection(FakeSpaceStore(), MutableStateFlow<RoomListsInterface>(SpacesFixture.lists()), scope())
        assertNull(selection.selected.value)
    }

    @Test
    fun theSelectedSpaceIsSavedAndRestoredAtTheNextLaunch() = runTest {
        val store = FakeSpaceStore()
        val first = SpaceSelection(store, MutableStateFlow<RoomListsInterface>(SpacesFixture.lists()), scope())
        first.select(SpaceKey.Space(TEAM))
        assertEquals(SpaceKey.Space(TEAM), first.selected.value)
        assertEquals(SpaceKey.Space(TEAM), store.saved)

        // The next launch starts without a list, then gets the cached one.
        val rooms = MutableStateFlow<RoomListsInterface>(NoRooms)
        val next = SpaceSelection(store, rooms, scope())
        assertNull(next.selected.value)
        rooms.value = SpacesFixture.lists(loaded = false)
        assertEquals(SpaceKey.Space(TEAM), next.selected.value)
        rooms.value = SpacesFixture.lists()
        assertEquals(SpaceKey.Space(TEAM), next.selected.value)
    }

    @Test
    fun aSavedSpaceThatIsGoneIsForgottenOnceTheListIsLoaded() = runTest {
        val left = SpaceKey.Space("!left:x")
        val store = FakeSpaceStore(left)
        val rooms = MutableStateFlow<RoomListsInterface>(SpacesFixture.lists(loaded = false))
        val selection = SpaceSelection(store, rooms, scope())
        assertNull(selection.selected.value)
        assertEquals("kept while the list may be a cache", left, store.saved)

        rooms.value = SpacesFixture.lists()
        assertNull(selection.selected.value)
        assertNull(store.saved)
    }

    @Test
    fun aSelectedEntryThatDisappearsIsDeselectedAndForgotten() = runTest {
        val store = FakeSpaceStore()
        val rooms = MutableStateFlow<RoomListsInterface>(SpacesFixture.lists())
        val selection = SpaceSelection(store, rooms, scope())
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
        val store = FakeSpaceStore(SpaceKey.Space(TEAM))
        val session = FakeSession()
        val account = testAccount(FakeMtrx(restored = session), scope(), store, FakeSortStore())
        account.start()
        session.roomListListener!!.onUpdate(SpacesFixture.lists())
        val selection = account.signedIn.spaceSelection
        assertEquals("restored", SpaceKey.Space(TEAM), selection.selected.value)

        account.signOut()
        assertNull(selection.selected.value)
        assertNull(store.saved)
    }
}
