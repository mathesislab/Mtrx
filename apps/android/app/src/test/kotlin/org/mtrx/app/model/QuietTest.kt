package org.mtrx.app.model

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mtrx.app.FakeSession
import org.mtrx.app.settings.clock
import uniffi.mtrx.MtrxException
import uniffi.mtrx.PauseFor

/** Pausing notifications and quiet hours, as the library tells them. */
@OptIn(ExperimentalCoroutinesApi::class)
class QuietTest {
    @Test
    fun followsTheLibraryUntilClosed() = runTest(UnconfinedTestDispatcher()) {
        val session = FakeSession()
        val quiet = QuietModel(session, this) {}
        assertNull(quiet.state.value)
        quiet.start()
        assertEquals("Notifications on", quiet.state.value?.line)
        quiet.close()
        assertTrue(session.quietSubscriptions.single().second.cancelled)
    }

    @Test
    fun changesGoToTheLibrary() = runTest(UnconfinedTestDispatcher()) {
        val session = FakeSession()
        val quiet = QuietModel(session, this) {}
        quiet.start()
        quiet.pause(PauseFor.Hour).join()
        assertTrue(quiet.state.value?.paused == true)
        quiet.resume().join()
        assertFalse(quiet.state.value?.quiet == true)
        val hours = quiet.state.value!!.hours
        quiet.setHours(hours.copy(on = true, fromMinute = (23 * 60).toUShort())).join()
        quiet.setMentionsThrough(true).join()
        assertEquals(listOf("pause Hour", "resume", "hours 1380-420", "mentions through true"), session.roomCalls)
        assertEquals("23:00", clock(quiet.state.value!!.hours.fromMinute))
        quiet.close()
    }

    @Test
    fun aFailedChangeSaysWhy() = runTest(UnconfinedTestDispatcher()) {
        val session = FakeSession()
        val failures = mutableListOf<String>()
        val quiet = QuietModel(session, this) { failures += it }
        session.roomError = MtrxException.Other("Offline")
        quiet.resume().join()
        assertEquals(listOf("Could not resume notifications: Offline"), failures)
    }
}
