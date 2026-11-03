package org.mtrx.app.rooms

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestDispatcher
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.mtrx.app.FakeSession
import org.mtrx.app.directoryRoom
import org.mtrx.app.member
import org.mtrx.app.roomSettings
import uniffi.mtrx.Access
import uniffi.mtrx.Address
import uniffi.mtrx.DirectoryKind
import uniffi.mtrx.DirectoryPage
import uniffi.mtrx.JoinAccess
import uniffi.mtrx.Membership
import uniffi.mtrx.MtrxException
import uniffi.mtrx.Topic

@OptIn(ExperimentalCoroutinesApi::class)
class RoomModelsTest {
    private val session = FakeSession()
    private val room = "!room:x"

    @Before
    fun mainDispatcher() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
    }

    @After
    fun resetMainDispatcher() {
        Dispatchers.resetMain()
    }

    // Room info

    @Test
    fun roomInfoReadsTheSettingsAndMembers() = runTest {
        session.settings = roomSettings(access = Access.PUBLIC)
        session.memberList = listOf(member("@b:x"))
        val model = RoomInfoModel(room, session) { it.text }
        assertEquals(Access.PUBLIC, model.settings.value?.access)
        assertEquals(listOf("@b:x"), model.members.value?.map { it.userId })
    }

    @Test
    fun aChangeReadsTheSettingsAgain() = runTest {
        val model = RoomInfoModel(room, session) { "composed ${it.text}" }
        val reads = session.settingsReads
        session.settings = roomSettings(access = Access.KNOCK)
        model.setAccess(Access.KNOCK)
        model.setTopic(Topic("Ice", emptyList()))
        model.setAddress(" #ice ")
        model.setAddress("")
        assertEquals(listOf("access KNOCK", "topic composed Ice", "address ice", "address null"), session.roomCalls)
        assertEquals(reads + 4, session.settingsReads)
        assertEquals(Access.KNOCK, model.settings.value?.access)
        assertNull(model.busy.value)
    }

    @Test
    fun memberChangesReadTheMembersAgainAndFailuresShow() = runTest {
        val model = RoomInfoModel(room, session) { it.text }
        session.memberList = listOf(member("@b:x", level = 50))
        model.setLevel("@b:x", 50)
        model.remove("@c:x", "  ", ban = true)
        assertEquals(listOf("level @b:x 50", "ban @c:x null"), session.roomCalls)
        assertEquals(50L, model.members.value?.single()?.level)

        session.roomGate = CompletableDeferred()
        model.unban("@c:x")
        assertEquals("Saving…", model.busy.value)
        session.roomError = MtrxException.Other("not allowed")
        session.roomGate!!.complete(Unit)
        assertEquals("not allowed", model.error.value)
        assertNull(model.busy.value)
    }

    // Space overview

    @Test
    fun theLastListShowsAtOnceAndUpdatesBehind() = runTest {
        session.listed["!team:x"] = listOf(directoryRoom("!old:x"))
        session.children["!team:x"] = listOf(directoryRoom("!lab:x"), directoryRoom("!ask:x", access = JoinAccess.KNOCK))
        session.childrenGate = CompletableDeferred()
        val model = SpaceOverviewModel("!team:x", session)
        assertEquals(listOf("!old:x"), model.rooms.value?.map { it.id })
        assertTrue(model.updating.value)
        session.childrenGate!!.complete(Unit)
        assertEquals(listOf("!lab:x", "!ask:x"), model.rooms.value?.map { it.id })
        assertFalse(model.updating.value)
    }

    @Test
    fun nothingShowsUntilFirstListed() = runTest {
        session.childrenGate = CompletableDeferred()
        val model = SpaceOverviewModel("!team:x", session)
        assertNull(model.rooms.value)
        session.childrenGate!!.complete(Unit)
        assertEquals(emptyList<String>(), model.rooms.value?.map { it.id })
    }

    @Test
    fun aFailedUpdateKeepsTheListAndOffersTryAgain() = runTest {
        session.listed["!team:x"] = listOf(directoryRoom("!old:x"))
        session.childrenError = MtrxException.Other("timeout")
        val model = SpaceOverviewModel("!team:x", session)
        assertTrue(model.failed.value)
        assertEquals(listOf("!old:x"), model.rooms.value?.map { it.id })
        session.childrenError = null
        model.refresh()
        assertFalse(model.failed.value)
        assertEquals(emptyList<String>(), model.rooms.value?.map { it.id })
    }

    @Test
    fun joiningStaysOnTheOverviewAndTheRowOpens() = runTest {
        val lab = directoryRoom("!lab:x")
        val ask = directoryRoom("!ask:x", access = JoinAccess.KNOCK)
        session.children["!team:x"] = listOf(lab, ask)
        val model = SpaceOverviewModel("!team:x", session)
        model.join(lab)
        model.knock(ask)
        assertEquals(listOf("join !lab:x", "knock !ask:x"), session.roomCalls)
        // Asked here; joined as the room list has it (at once, the library's).
        assertEquals(listOf(RowAction.JOIN, RowAction.ASKED), model.rooms.value!!.map { rowAction(it) })
        val joined = setOf(lab.id)
        assertEquals(listOf(RowAction.OPEN, RowAction.ASKED), model.rooms.value!!.map { rowAction(it, it.id in joined) })
        assertTrue(model.pending.value.isEmpty())
    }

    // Joining from a directory

    @Test
    fun theDirectoryOfOurServerFirstThenAnotherRemembered() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        Dispatchers.setMain(dispatcher)
        session.directory[listOf(null, "", DirectoryKind.ALL, null)] =
            DirectoryPage(rooms = listOf(directoryRoom("!ours:x")), next = "2")
        session.directory[listOf(null, "", DirectoryKind.ALL, "2")] =
            DirectoryPage(rooms = listOf(directoryRoom("!more:x")), next = null)
        session.directory[listOf("other.org", "", DirectoryKind.SPACES, null)] =
            DirectoryPage(rooms = listOf(directoryRoom("!theirs:other.org", space = true)), next = null)
        val model = JoinModel(session, parseAddress = { null })
        advanceUntilIdle()
        assertEquals(listOf("!ours:x"), (model.results.value as JoinResults.Listed).rooms.map { it.id })
        model.more()
        advanceUntilIdle()
        assertEquals(listOf("!ours:x", "!more:x"), (model.results.value as JoinResults.Listed).rooms.map { it.id })

        model.setKind(DirectoryKind.SPACES)
        model.setServer(" other.org ")
        advanceUntilIdle()
        assertEquals(listOf("!theirs:other.org"), (model.results.value as JoinResults.Listed).rooms.map { it.id })
        assertEquals(listOf("other.org"), session.browsedServers)
    }

    @Test
    fun typingWaitsAndAnAddressShowsItsPreview() = runTest {
        val dispatcher: TestDispatcher = StandardTestDispatcher(testScheduler)
        Dispatchers.setMain(dispatcher)
        session.previews["#ice:x"] = directoryRoom("!ice:x", access = JoinAccess.KNOCK)
        val model = JoinModel(session, parseAddress = { text -> Address(text, emptyList()).takeIf { text.startsWith("#") } })
        advanceUntilIdle()
        session.roomCalls.clear()
        model.setQuery("#i")
        model.setQuery("#ice:x")
        advanceTimeBy(JoinModel.SEARCH_DELAY_MILLIS + 1)
        advanceUntilIdle()
        assertEquals(listOf("preview #ice:x"), session.roomCalls)
        val preview = model.results.value as JoinResults.Preview
        model.knock(preview.room)
        advanceUntilIdle()
        assertEquals(Membership.ASKED, (model.results.value as JoinResults.Preview).room.membership)
    }

    @Test
    fun aServerWithoutADirectorySaysSo() = runTest {
        session.directoryError = MtrxException.Other("403")
        val model = JoinModel(session, parseAddress = { null })
        assertEquals(JoinResults.Failed("Couldn't list the rooms."), model.results.value)
        model.setServer("closed.org")
        assertTrue((model.results.value as JoinResults.Failed).message.startsWith("closed.org doesn't share its directory"))
        assertTrue(session.browsedServers.isEmpty())
    }

    // New rooms and spaces

    @Test
    fun aNewRoomInASpaceWithCoOwnersWhereOffered() = runTest {
        session.ownersOffered = true
        val model = NewRoomModel(session, space = false, parent = "!team:x")
        assertTrue(model.offersOwners.value)
        model.edit { it.copy(name = " Lab ", topic = " ", person = "@b:x") }
        assertTrue(model.addPerson())
        model.edit { it.copy(person = "not a user") }
        assertFalse(model.addPerson())
        model.edit { it.copy(person = "@c:x") }
        model.setCoOwner("@b:x", true)
        var created: String? = null
        // The typed person is invited too.
        model.create { created = it }
        assertEquals("!new:example.org", created)
        val new = session.created.single()
        assertEquals("Lab", new.name)
        assertNull(new.topic)
        assertTrue(new.encrypted)
        assertFalse(new.moderated)
        assertEquals(listOf("@b:x", "@c:x"), new.invite)
        assertEquals(listOf("@b:x"), new.coOwners)
        assertEquals("!team:x", new.parent)
    }

    @Test
    fun aSpaceIsNotEncryptedAndCoOwnersNeedTheOffer() {
        val form = NewRoomForm(name = "Team", invite = listOf("@b:x"), coOwners = setOf("@b:x"))
        val space = form.toNewRoom(space = true, parent = null, offersOwners = false)
        assertFalse(space.encrypted)
        assertTrue(space.coOwners.isEmpty())
        assertFalse(NewRoomForm().canCreate)
    }

    @Test
    fun aFailedCreationSaysWhat() = runTest {
        session.roomError = MtrxException.Other("no")
        val model = NewRoomModel(session, space = true, parent = null)
        model.edit { it.copy(name = "Team") }
        model.create { }
        assertEquals("Could not create the space “Team”: no", model.error.value)
        assertFalse(model.creating.value)
    }
}
