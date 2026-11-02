package org.mtrx.app.rooms

import java.time.Instant
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
import org.mtrx.app.FakeMtrx
import org.mtrx.app.FakeRoomLists
import org.mtrx.app.FakeSession
import org.mtrx.app.SpacesFixture
import org.mtrx.app.SpacesFixture.DEEP
import org.mtrx.app.SpacesFixture.LAB
import org.mtrx.app.SpacesFixture.LOOSE
import org.mtrx.app.SpacesFixture.PARTY
import org.mtrx.app.SpacesFixture.TEAM
import org.mtrx.app.SpacesFixture.WITH_BOB
import org.mtrx.app.SpacesFixture.room
import org.mtrx.app.listSection
import org.mtrx.app.testAccount
import org.mtrx.app.timeline.roomHeader
import org.mtrx.app.ui.HomeNav
import org.mtrx.app.ui.HomeTab
import uniffi.mtrx.Draft

@OptIn(ExperimentalCoroutinesApi::class)
class RoomsTabTest {
    private fun TestScope.scope() = CoroutineScope(UnconfinedTestDispatcher(testScheduler))

    private val lists = SpacesFixture.lists()

    @Test
    fun theOpenRoomShowsWhileListed() {
        assertEquals(LAB, shownRoom(lists, LAB))
        assertNull(shownRoom(lists, null))
        assertNull(shownRoom(lists, "!gone:x"))
        // Not on the Rooms tab.
        assertNull(shownRoom(lists, WITH_BOB))
        assertNull(shownRoom(lists, TEAM))
        // Before the list is loaded.
        assertEquals(LAB, shownRoom(NoRooms, LAB))
        // Just joined, not listed yet: coming, as the library says.
        assertEquals("!new:x", shownRoom(SpacesFixture.lists(joining = listOf("!new:x")), "!new:x"))
    }

    // #59: on iOS an open room showed another's messages, then another's
    // header, as the list re-sorted. Here the open room is its ID, and its
    // screen (header, timeline model) is keyed by it.
    @Test
    fun theOpenRoomStaysAsTheListReSorts() {
        val nav = HomeNav()
        nav.open(LAB, room(LAB))
        fun latest(id: String, at: Long): FakeRoomLists {
            val rooms = listOf(room(id).copy(lastActivity = Instant.ofEpochSecond(at))) +
                SpacesFixture.rooms.filter { it.id != id }
            return FakeRoomLists(rooms = rooms, firstSpaces = mapOf(LAB to room(TEAM)))
        }
        // A message arrives in Deep, then ours is sent in Lab.
        for (list in listOf(latest(DEEP, 300), latest(LAB, 400))) {
            val shown = shownRoom(list, nav.room)
            assertEquals(LAB, shown)
            val header = roomHeader(LAB, shown?.let(list::room), FakeSession.USER_ID, space = list.firstSpaceOf(LAB))
            assertEquals("lab", header.name)
            assertEquals(TEAM, header.space?.id)
        }
        assertEquals(HomeTab.Rooms, nav.tab)
    }

    /** Each group with its space's summary, for its picture; none for Home. */
    @Test
    fun theRoomsTabsGroupsHaveTheirSpaces() {
        val groups = listOf(
            listSection(HOME_GROUP, HOME_NAME, listOf(room(PARTY), room(LOOSE))),
            listSection(TEAM, "team", listOf(room(LAB)), folded = true, space = TEAM),
        )
        val shown = roomGroups(FakeRoomLists(rooms = SpacesFixture.rooms, groups = groups))
        assertEquals(listOf(null, TEAM), shown.map { it.space?.id })
        assertEquals(listOf(HOME_GROUP, TEAM), shown.map { it.key })
        assertEquals(listOf(false, true), shown.map { it.folded })
        assertEquals(listOf(0L, 1L), shown.map { it.unread })
    }

    @Test
    fun directChatsOpenOnChatsAndRoomsOnRooms() {
        val nav = HomeNav()
        nav.open(WITH_BOB, room(WITH_BOB))
        assertEquals(HomeTab.Chats, nav.tab)
        assertEquals(WITH_BOB, nav.chat)
        assertNull(nav.room)

        nav.open(LAB, room(LAB))
        assertEquals(HomeTab.Rooms, nav.tab)
        assertEquals(LAB, nav.room)
        // An invite opens there too (its invite screen).
        nav.open(PARTY, room(PARTY))
        assertEquals(PARTY, nav.room)
        // A room not listed yet, just joined.
        nav.open("!new:x", null)
        assertEquals(HomeTab.Rooms, nav.tab)
        assertEquals("!new:x", nav.room)
    }

    @Test
    fun theRoomsTabOpensOnItsListAndTappingItAgainGoesBackThere() {
        val nav = HomeNav(tab = HomeTab.Spaces)
        nav.tapTab(HomeTab.Rooms)
        assertNull(nav.room)
        nav.open(LAB, room(LAB))
        nav.tapTab(HomeTab.Chats)
        nav.tapTab(HomeTab.Rooms)
        assertEquals(LAB, nav.room)
        nav.tapTab(HomeTab.Rooms)
        assertNull(nav.room)
    }

    @Test
    fun spacesAreNotOpenedOnRoomsButInvitesToThemAre() {
        assertFalse(opensOnRoomsTab(room(TEAM)))
        assertTrue(opensOnRoomsTab(room(LAB)))
    }

    @Test
    fun aLeftRoomKeepsItsDraft() = runTest {
        val session = FakeSession()
        val account = testAccount(FakeMtrx(restored = session), scope())
        account.start()
        session.drafts[LAB] = Draft("half a thought", null)
        account.leave(LAB, "lab")
        assertEquals(listOf("leave $LAB"), session.roomCalls)
        assertEquals(Draft("half a thought", null), session.drafts[LAB])
    }

    @Test
    fun signingOutForgetsTheDrafts() = runTest {
        val session = FakeSession()
        val account = testAccount(FakeMtrx(restored = session), scope())
        account.start()
        session.roomListListener!!.onUpdate(SpacesFixture.lists(drafts = listOf(LAB)))
        assertEquals(listOf(LAB), account.rooms.value.drafts())
        account.signOut()
        // The drafts were in the session's store, gone with it.
        assertTrue(session.loggedOut)
        assertEquals(emptyList<String>(), account.rooms.value.drafts())
    }
}
