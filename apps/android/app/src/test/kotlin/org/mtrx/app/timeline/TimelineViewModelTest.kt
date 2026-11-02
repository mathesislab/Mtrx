package org.mtrx.app.timeline

import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
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
import org.mtrx.app.FakeTimeline
import org.mtrx.app.messageEvent
import org.mtrx.app.roomSettings
import org.mtrx.app.textEvent
import uniffi.mtrx.Draft
import uniffi.mtrx.Message
import uniffi.mtrx.MessageKind
import uniffi.mtrx.MtrxException
import uniffi.mtrx.QuoteState
import uniffi.mtrx.ReplyQuote
import uniffi.mtrx.RowContent
import uniffi.mtrx.TimelineRow
import uniffi.mtrx.Topic
import uniffi.mtrx.UserProfile

@OptIn(ExperimentalCoroutinesApi::class)
class TimelineViewModelTest {
    private val timeline = FakeTimeline()
    private val session = FakeSession().apply { roomTimeline = timeline }

    @Before
    fun mainDispatcher() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
    }

    @After
    fun resetMainDispatcher() {
        Dispatchers.resetMain()
    }

    private var reads = 0

    private val room = "!room:example.org"

    private fun viewModel() = TimelineViewModel(room, session, onRead = { reads++ })

    /** [roomId]'s model in [store]: clearing it leaves the room. */
    private fun open(roomId: String, store: ViewModelStore = ViewModelStore()): TimelineViewModel {
        val factory = viewModelFactory { initializer { TimelineViewModel(roomId, session, onRead = {}) } }
        return ViewModelProvider(store, factory)[TimelineViewModel::class.java]
    }

    @Test
    fun theRoomsSharedWithThePersonLoadAsTheirPanelOpens() = runTest {
        session.shared["@bob:x"] = listOf("!lab:x", "!team:x")
        val viewModel = viewModel()
        assertNull(viewModel.sharedRooms.value)
        viewModel.showSharedRooms("@bob:x")
        assertEquals(listOf("!lab:x", "!team:x"), viewModel.sharedRooms.value)
        // Failing: none, and why.
        session.sharedError = MtrxException.Other("offline")
        viewModel.showSharedRooms("@bob:x")
        assertEquals(emptyList<String>(), viewModel.sharedRooms.value)
        assertEquals("offline", viewModel.error.value)
    }

    @Test
    fun personInfoOpensOnceAndCloses() = runTest {
        session.profile = UserProfile(userId = "@bob:x", name = "Bob", avatarUrl = null)
        val viewModel = viewModel()
        viewModel.showPersonInfo("@bob:x")
        val model = viewModel.personInfo.value!!
        assertEquals("@bob:x", model.userId)
        assertEquals("Bob", model.profile.value?.name)
        // Tapped again while open: the same sheet.
        viewModel.showPersonInfo("@bob:x")
        assertTrue(viewModel.personInfo.value === model)
        viewModel.closePersonInfo()
        assertNull(viewModel.personInfo.value)
    }

    @Test
    fun updatesBecomeRows() = runTest {
        val viewModel = viewModel()
        timeline.listener!!.onUpdate(listOf(textEvent("1", "@alice:example.org", "hi")))
        assertEquals(listOf("1"), viewModel.rows.value.map { it.id })
    }

    @Test
    fun sendsTheDraftAndKeepsItOnFailure() = runTest {
        val viewModel = viewModel()
        assertFalse(viewModel.send())
        viewModel.onDraft("hello")
        assertTrue(viewModel.send())
        assertEquals(listOf("hello"), timeline.sent)
        assertEquals("", viewModel.draft.value)

        timeline.sendError = MtrxException.Other("offline")
        viewModel.onDraft("again")
        viewModel.send()
        assertEquals("again", viewModel.draft.value)
        assertEquals("offline", viewModel.error.value)
        // Kept again, as the send cleared it.
        assertEquals(Draft("again", null), session.drafts[room])
    }

    @Test
    fun loadsOlderEventsUntilTheStart() = runTest {
        val viewModel = viewModel()
        viewModel.onNearTop(true)
        // The fake reaches the start with the first page.
        viewModel.onNearTop(true)
        assertEquals(1, timeline.pages)
    }

    @Test
    fun marksTheNewestMessageReadOnce() = runTest {
        val viewModel = viewModel()
        timeline.listener!!.onUpdate(listOf(textEvent("1", "@alice:example.org", "hi")))
        viewModel.markRead()
        viewModel.markRead()
        assertEquals(1, timeline.markedRead)
        // The room's notifications go once it is read.
        assertEquals(1, reads)
    }

    @Test
    fun aFailedMarkReadKeepsTheNotifications() = runTest {
        val viewModel = viewModel()
        timeline.markReadError = MtrxException.Other("offline")
        timeline.listener!!.onUpdate(listOf(textEvent("1", "@alice:example.org", "hi")))
        viewModel.markRead()
        assertEquals(0, reads)
    }

    private val alice = "@alice:example.org"
    private val me = "@me:example.org"

    /** Our message "**bold**", shown as "bold". */
    private val ours = messageEvent(
        "mine",
        me,
        Message(kind = MessageKind.TEXT, body = "bold", edited = false, media = null, rawBody = "**bold**"),
        own = true,
        editable = true,
    )

    private fun show(vararg rows: TimelineRow) = timeline.listener!!.onUpdate(rows.toList())

    @Test
    fun repliesFromTheComposer() = runTest {
        val viewModel = viewModel()
        show(textEvent("1", alice, "shall we?\nat noon"))
        viewModel.startReply("1")
        assertEquals(Composing.Replying("1", "alice", "shall we?", "\$1"), viewModel.composing.value)
        viewModel.onDraft("yes")
        assertTrue(viewModel.send())
        assertEquals(listOf("1" to "yes"), timeline.replies)
        assertEquals(emptyList<String>(), timeline.sent)
        assertNull(viewModel.composing.value)
        assertEquals("", viewModel.draft.value)

        // A refused reply comes back to the composer.
        timeline.actionError = MtrxException.Other("refused")
        viewModel.startReply("1")
        viewModel.onDraft("again")
        viewModel.send()
        assertEquals("refused", viewModel.error.value)
        assertEquals(Composing.Replying("1", "alice", "shall we?", "\$1"), viewModel.composing.value)
        assertEquals("again", viewModel.draft.value)

        viewModel.cancelComposing()
        assertNull(viewModel.composing.value)
        assertEquals("again", viewModel.draft.value)
    }

    @Test
    fun aMessageNotSentYetCannotBeAnswered() = runTest {
        val viewModel = viewModel()
        show(messageEvent("unsent", "@me:example.org", Message(kind = MessageKind.TEXT, body = "soon", edited = false, media = null), own = true, eventId = null))
        viewModel.startReply("unsent")
        assertNull(viewModel.composing.value)
    }

    @Test
    fun editsStartFromTheTextWeWroteAndGiveTheDraftBack() = runTest {
        val viewModel = viewModel()
        show(textEvent("1", alice, "hi"), ours)
        viewModel.onDraft("unsent")
        viewModel.startEdit("mine")
        assertEquals(Composing.Editing("mine", "**bold**", "unsent"), viewModel.composing.value)
        assertEquals("**bold**", viewModel.draft.value)

        // Cancelling gives back the draft.
        viewModel.cancelComposing()
        assertNull(viewModel.composing.value)
        assertEquals("unsent", viewModel.draft.value)

        // One of replying and editing at a time.
        viewModel.startEdit("mine")
        viewModel.startReply("1")
        assertEquals("unsent", viewModel.draft.value)
        assertTrue(viewModel.composing.value is Composing.Replying)
        viewModel.startEdit("mine")
        assertEquals("**bold**", viewModel.draft.value)
        assertEquals(Composing.Editing("mine", "**bold**", "unsent"), viewModel.composing.value)

        viewModel.onDraft("**bolder**")
        assertTrue(viewModel.send())
        assertEquals(listOf("mine" to "**bolder**"), timeline.edits)
        assertNull(viewModel.composing.value)
        assertEquals("unsent", viewModel.draft.value)

        // Unchanged: nothing to send.
        viewModel.startEdit("mine")
        assertTrue(viewModel.send())
        assertEquals(1, timeline.edits.size)
    }

    @Test
    fun actionsTakeTheRowId() = runTest {
        val viewModel = viewModel()
        show(ours)
        viewModel.react("mine", "👍")
        viewModel.delete("mine")
        viewModel.retry("mine")
        viewModel.discard("mine")
        assertEquals(listOf("mine" to "👍"), timeline.reactions)
        assertEquals(listOf("mine"), timeline.deleted)
        assertEquals(listOf("mine"), timeline.retried)
        assertEquals(listOf("mine"), timeline.discarded)

        // A row gone meanwhile.
        timeline.actionError = MtrxException.InvalidInput("no message `mine` in the timeline")
        viewModel.react("mine", "👍")
        assertEquals(TimelineViewModel.GONE, viewModel.error.value)
    }

    @Test
    fun typingGoesBothWays() = runTest {
        val viewModel = viewModel()
        assertEquals(emptyList<String>(), viewModel.typing.value)
        timeline.typingListener!!.onUpdate(listOf("Anna", "Ben"))
        assertEquals(listOf("Anna", "Ben"), viewModel.typing.value)

        viewModel.onDraft("h")
        viewModel.onDraft("hi")
        viewModel.onDraft("")
        viewModel.onDraft("")
        assertEquals(listOf(true, true, false), timeline.typing)
        viewModel.onDraft("hello")
        viewModel.send()
        assertEquals(listOf(true, true, false, true, false), timeline.typing)
    }

    @Test
    fun leavingTheRoomStopsTypingButNotSending() = runTest {
        val store = ViewModelStore()
        val factory = viewModelFactory { initializer { TimelineViewModel("!room:example.org", session, onRead = {}) } }
        val viewModel = ViewModelProvider(store, factory)[TimelineViewModel::class.java]
        val gate = CompletableDeferred<Unit>()
        timeline.gate = gate
        viewModel.onDraft("hello")
        viewModel.onDraft("hello!")
        // Not sent: typing goes on.
        viewModel.onDraft("bye")
        viewModel.send()

        viewModel.onDraft("unsent")
        store.clear()
        assertEquals(Draft("unsent", null), session.drafts[room])
        assertTrue(timeline.typingSubscription.cancelled)
        assertEquals(false, timeline.typing.last())

        gate.complete(Unit)
        assertEquals(listOf("bye"), timeline.sent)
        assertTrue(timeline.closed)
    }

    private val original = textEvent("0", alice, "the original")

    private val reply = messageEvent(
        "1",
        alice,
        Message(
            kind = MessageKind.TEXT,
            body = "the reply",
            edited = false,
            media = null,
            replyTo = ReplyQuote("\$0", QuoteState.LOADED, alice, null, MessageKind.TEXT, "the original"),
        ),
    )

    private val start = TimelineRow("start", RowContent.Start)

    @Test
    fun goesToAnOriginalLoadedAlready() = runTest {
        val viewModel = viewModel()
        show(original, reply)
        viewModel.goToOriginal("\$0")
        assertEquals(emptyList<String>(), timeline.seeks)
        assertEquals("0", viewModel.highlighted.value)
        assertFalse(viewModel.seeking.value)
        // Marked for two seconds.
        advanceTimeBy(TimelineViewModel.HIGHLIGHT_MILLIS + 1)
        assertNull(viewModel.highlighted.value)
    }

    @Test
    fun theLibraryLoadsOlderEventsUntilTheOriginalIsThere() = runTest {
        val viewModel = viewModel()
        show(reply)
        timeline.found["\$0"] = "0"
        timeline.onSeek = { show(original, reply) }
        viewModel.goToOriginal("\$0")
        assertEquals(listOf("\$0"), timeline.seeks)
        assertEquals("0", viewModel.highlighted.value)
        assertFalse(viewModel.seeking.value)
        assertNull(viewModel.error.value)
    }

    @Test
    fun goesToTheOriginalFoundOnceTheRowsHaveIt() = runTest {
        val viewModel = viewModel()
        show(reply)
        timeline.found["\$0"] = "0"
        viewModel.goToOriginal("\$0")
        assertFalse(viewModel.seeking.value)
        assertNull(viewModel.highlighted.value)
        show(start, original, reply)
        assertEquals("0", viewModel.highlighted.value)
    }

    @Test
    fun saysWhenTheOriginalIsNotFound() = runTest {
        val viewModel = viewModel()
        show(start, reply)
        viewModel.goToOriginal("\$0")
        assertEquals("The original message was not found in the room's history", viewModel.error.value)
        assertNull(viewModel.highlighted.value)
        assertFalse(viewModel.seeking.value)
    }

    @Test
    fun theDraftStaysWithItsRoom() = runTest {
        val inA = ViewModelStore()
        open("!a:x", inA).onDraft("for A")
        inA.clear()
        assertEquals(Draft("for A", null), session.drafts["!a:x"])
        val inB = ViewModelStore()
        val b = open("!b:x", inB)
        assertEquals("", b.draft.value)
        b.onDraft("for B")
        inB.clear()
        assertEquals("for A", open("!a:x").draft.value)
        assertEquals("for B", open("!b:x").draft.value)
    }

    @Test
    fun eachChangeGoesToTheKeeperAndSendingClearsIt() = runTest {
        val viewModel = viewModel()
        viewModel.onDraft("h")
        viewModel.onDraft("hi")
        assertEquals(listOf("h", "hi"), session.keepers[room]!!.changes.map { it.text })
        // The app stops: saved at once.
        viewModel.keepDraft()
        assertEquals(listOf(room to Draft("hi", null)), session.draftSaves)

        viewModel.send()
        assertEquals(room to Draft("", null), session.draftSaves.last())
        assertNull(session.drafts[room])
    }

    @Test
    fun aReplyIsKeptWithTheMessageItAnswers() = runTest {
        val viewModel = viewModel()
        show(textEvent("1", alice, "shall we?"))
        viewModel.startReply("1")
        viewModel.onDraft("yes")
        viewModel.keepDraft()
        assertEquals(Draft("yes", "\$1"), session.drafts[room])
        viewModel.cancelComposing()
        viewModel.keepDraft()
        assertEquals(Draft("yes", null), session.drafts[room])
    }

    @Test
    fun aDraftComesBackAsAReplyOnceItsMessageIsThere() = runTest {
        session.drafts[room] = Draft("yes", "\$1")
        // The library finds it before the rows have it.
        timeline.found["\$1"] = "1"
        val viewModel = viewModel()
        assertEquals("yes", viewModel.draft.value)
        assertNull(viewModel.composing.value)
        show(textEvent("1", alice, "shall we?"))
        assertEquals(Composing.Replying("1", "alice", "shall we?", "\$1"), viewModel.composing.value)
        viewModel.send()
        assertEquals(listOf("1" to "yes"), timeline.replies)
        assertNull(session.drafts[room])
    }

    @Test
    fun withoutTheMessageItAnswersTheDraftComesBackAsText() = runTest {
        session.drafts[room] = Draft("yes", "\$gone")
        val viewModel = viewModel()
        show(textEvent("1", alice, "shall we?"))
        assertEquals("yes", viewModel.draft.value)
        assertNull(viewModel.composing.value)
        viewModel.send()
        assertEquals(listOf("yes"), timeline.sent)
    }

    @Test
    fun leavingDuringAnEditKeepsTheDraftFromBefore() = runTest {
        val store = ViewModelStore()
        val viewModel = open(room, store)
        show(ours)
        viewModel.onDraft("half")
        viewModel.startEdit("mine")
        viewModel.onDraft("mine, edited")
        viewModel.keepDraft()
        assertEquals(Draft("half", null), session.drafts[room])
        store.clear()
        assertEquals(Draft("half", null), session.drafts[room])
        val again = open(room)
        assertEquals("half", again.draft.value)
        assertNull(again.composing.value)
    }

    @Test
    fun topicFollowsTheRoomsSettings() = runTest {
        session.settings = roomSettings(may = true)
        val viewModel = TimelineViewModel("!a:x", session, onRead = {}, composeTopic = { "composed ${it.text}" })
        assertEquals(true, viewModel.settings.value?.canChangeTopic)
        viewModel.saveTopic(Topic("Ice", emptyList()))
        viewModel.setPinned("\$1", true)
        assertEquals(listOf("topic composed Ice", "pin \$1"), session.roomCalls)

        // The pinned messages while the panel shows them.
        assertNull(viewModel.pinned.value)
        viewModel.showPinned()
        session.pinnedRoomTimeline.listener!!.onUpdate(listOf(textEvent("p", alice, "pinned")))
        assertEquals(listOf("p"), viewModel.pinned.value?.map { it.id })
        viewModel.hidePinned()
        assertNull(viewModel.pinned.value)
        assertTrue(session.pinnedRoomTimeline.closed)
    }
}
