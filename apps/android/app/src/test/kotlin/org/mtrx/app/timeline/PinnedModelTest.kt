package org.mtrx.app.timeline

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
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
import org.mtrx.app.textEvent

/** The pinned messages, while the topic panel shows them. */
@OptIn(ExperimentalCoroutinesApi::class)
class PinnedModelTest {
    private val session = FakeSession()

    @Before
    fun mainDispatcher() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
    }

    @After
    fun resetMainDispatcher() {
        Dispatchers.resetMain()
    }

    @Test
    fun thePinnedMessagesShowWhileThePanelIsOpen() = runTest {
        val model = PinnedModel("!a:x", session)
        assertNull(model.pinned.value)
        model.show()
        session.pinnedRoomTimeline.listener!!.onUpdate(listOf(textEvent("p", "@alice:example.org", "pinned")))
        assertEquals(listOf("p"), model.pinned.value?.map { it.id })
        assertFalse(session.pinnedRoomTimeline.closed)

        model.hide()
        assertNull(model.pinned.value)
        assertTrue(session.pinnedRoomTimeline.closed)
    }
}
