package org.mtrx.app.rooms

import org.junit.Assert.assertEquals
import org.junit.Test
import org.mtrx.app.SpacesFixture.TEAM
import org.mtrx.app.model.HOME_SAVED
import org.mtrx.app.model.RoomSort
import org.mtrx.app.model.saved
import org.mtrx.app.model.spaceKeyOf
import uniffi.mtrx.SpaceKey

/** The forms kept on the phone; the lists are the library's, tested there. */
class RoomListsTest {
    @Test
    fun entriesAreSavedByKey() {
        for (entry in listOf(SpaceKey.Space(TEAM), SpaceKey.Outside)) {
            assertEquals(entry, spaceKeyOf(entry.saved))
        }
        // From when Home was "Other rooms".
        assertEquals(SpaceKey.Outside, spaceKeyOf(HOME_SAVED))
    }

    @Test
    fun theSortsAreTheLibrarys() {
        assertEquals(
            listOf(
                uniffi.mtrx.RoomSort.LIST_ORDER,
                uniffi.mtrx.RoomSort.A_TO_Z,
                uniffi.mtrx.RoomSort.Z_TO_A,
                uniffi.mtrx.RoomSort.MOST_RECENT,
            ),
            RoomSort.entries.map { it.library },
        )
    }
}
