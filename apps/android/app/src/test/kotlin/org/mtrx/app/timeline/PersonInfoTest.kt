package org.mtrx.app.timeline

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mtrx.app.FakeSession
import org.mtrx.app.SpacesFixture
import org.mtrx.app.summary
import uniffi.mtrx.MtrxException
import uniffi.mtrx.RoomKind
import uniffi.mtrx.UserProfile

@OptIn(ExperimentalCoroutinesApi::class)
class PersonInfoTest {
    private val bob = "@bob:x"
    private val session = FakeSession()
    private val chat = summary(SpacesFixture.WITH_BOB, RoomKind.DIRECT, directUserId = bob, name = "Bob (chat)")
        .copy(avatarUrl = "mxc://x/chat")
    private val header = roomHeader(chat.id, chat, FakeSession.USER_ID)
    private val team = summary(SpacesFixture.TEAM, RoomKind.SPACE, name = "Team")

    // The room's scope (the profile) and the actions' (invites), apart.
    private val roomJob = Job()

    private fun TestScope.model(): PersonInfoModel {
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        return PersonInfoModel(bob, session, CoroutineScope(dispatcher + roomJob), CoroutineScope(dispatcher))
    }

    @Test
    fun headerNamesThePersonOfADirectChatOnly() {
        fun person(id: String) = roomHeader(id, SpacesFixture.room(id), FakeSession.USER_ID).person
        assertEquals(bob, header.person)
        assertNull("the Notes to self", person(SpacesFixture.NOTES))
        assertNull("a room", person(SpacesFixture.LAB))
        assertEquals(RoomHeader("!gone:x", null, "!gone:x", null), roomHeader("!gone:x", null, FakeSession.USER_ID))
    }

    @Test
    fun profileReplacesTheChatsNameAndAvatarOnceLoaded() = runTest {
        val gate = CompletableDeferred<Unit>()
        session.profileGate = gate
        session.profile = UserProfile(userId = bob, name = "Bob", avatarUrl = "mxc://x/bob")
        val model = model()
        assertEquals(PersonCard(bob, "Bob (chat)", "mxc://x/chat"), personCard(bob, header, model.profile.value))

        gate.complete(Unit)
        assertEquals(PersonCard(bob, "Bob", "mxc://x/bob"), personCard(bob, header, model.profile.value))
    }

    @Test
    fun profileFillsInOnlyWhatIsSet() {
        val noName = UserProfile(userId = bob, name = null, avatarUrl = "mxc://x/bob")
        assertEquals(PersonCard(bob, "Bob (chat)", "mxc://x/bob"), personCard(bob, header, noName))
        val noAvatar = UserProfile(userId = bob, name = "Bob", avatarUrl = null)
        assertEquals(PersonCard(bob, "Bob", "mxc://x/chat"), personCard(bob, header, noAvatar))
    }

    @Test
    fun failedProfileKeepsTheChatsWithoutAMessage() = runTest {
        session.profileError = MtrxException.Other("M_NOT_FOUND")
        val model = model()
        assertNull(model.profile.value)
        assertEquals(PersonCard(bob, "Bob (chat)", "mxc://x/chat"), personCard(bob, header, model.profile.value))
        assertNull(model.message.value)
    }

    @Test
    fun inviteSucceedsAndClosesTheListEvenAfterTheRoomIsLeft() = runTest {
        val gate = CompletableDeferred<Unit>()
        session.inviteGate = gate
        val model = model()
        model.chooseSpace()
        assertTrue(model.choosingSpace.value)

        model.invite(team)
        assertEquals(team.id, model.inviting.value)
        // One at a time.
        model.invite(summary(SpacesFixture.OTHER, RoomKind.SPACE))
        // Leaving the room does not cancel it.
        model.close()
        roomJob.cancel()
        gate.complete(Unit)

        assertEquals(listOf(team.id to bob), session.invites)
        assertNull(model.inviting.value)
        assertEquals("Invited to Team.", model.message.value)
        assertFalse(model.choosingSpace.value)
        model.clearMessage()
        assertNull(model.message.value)
    }

    @Test
    fun failedInviteGivesTheReasonAndKeepsTheList() = runTest {
        session.inviteError = MtrxException.Other("bob is already in the room.")
        val model = model()
        model.chooseSpace()
        model.invite(team)
        assertTrue(session.invites.isEmpty())
        assertNull(model.inviting.value)
        assertEquals("bob is already in the room.", model.message.value)
        assertTrue(model.choosingSpace.value)

        // Another try goes.
        session.inviteError = null
        model.invite(team)
        assertEquals(listOf(team.id to bob), session.invites)
    }

    @Test
    fun closingStopsLoadingTheProfile() = runTest {
        val gate = CompletableDeferred<Unit>()
        session.profileGate = gate
        session.profile = UserProfile(userId = bob, name = "Bob", avatarUrl = null)
        val model = model()
        model.close()
        gate.complete(Unit)
        assertNull(model.profile.value)
    }
}
