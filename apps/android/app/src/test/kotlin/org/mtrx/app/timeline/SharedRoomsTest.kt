package org.mtrx.app.timeline

import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Test
import org.mtrx.app.FakeSession
import uniffi.mtrx.MtrxException

/** The rooms shared with a direct chat's person, as their panel reads them. */
class SharedRoomsTest {
    private val session = FakeSession()

    @Test
    fun theSharedRoomsAreReadOrSayWhyNot() = runTest {
        session.shared["@bob:x"] = listOf("!lab:x", "!team:x")
        assertEquals(SharedRooms.Loaded(listOf("!lab:x", "!team:x")), loadSharedRooms(session, "@bob:x"))
        session.sharedError = MtrxException.Other("offline")
        assertEquals(SharedRooms.Failed("offline"), loadSharedRooms(session, "@bob:x"))
    }
}
