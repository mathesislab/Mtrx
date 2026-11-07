package org.mtrx.app.rooms

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mtrx.app.FakeMtrx
import org.mtrx.app.FakeSession
import org.mtrx.app.model.HOME_SAVED
import org.mtrx.app.model.MemoryStore
import org.mtrx.app.model.Preference
import org.mtrx.app.signedIn
import org.mtrx.app.testAccount
import uniffi.mtrx.SectionLayout

/** How this phone shows the spaces' sections; the library arranges them by it. */
@OptIn(ExperimentalCoroutinesApi::class)
class SectionsTest {
    private fun TestScope.scope() = CoroutineScope(UnconfinedTestDispatcher(testScheduler))

    private fun TestScope.layouts(store: MemoryStore<Map<String, SectionPrefs>>) =
        SectionLayouts(Preference(store, emptyMap(), scope()))

    @Test
    fun theLayoutAsTheLibraryTakesIt() {
        assertEquals(SectionLayout(emptyList(), emptyList(), emptyList()), SectionPrefs().library)
        val prefs = SectionPrefs(order = listOf("rooms", "invites"), hidden = setOf("rooms", "invites"), folded = setOf("b", "a"))
        assertEquals(SectionLayout(listOf("rooms", "invites"), listOf("invites", "rooms"), listOf("a", "b")), prefs.library)
    }

    @Test
    fun foldingShowingAndOrderAreSavedPerSpace() = runTest {
        val store = MemoryStore<Map<String, SectionPrefs>>()
        val layouts = layouts(store)
        val keys = listOf("invites", "direct_messages", "rooms")
        layouts.toggleFold("!s:x", "rooms")
        layouts.setShown("!s:x", "invites", false)
        layouts.move("!s:x", keys, 2, 0)
        assertEquals(SectionPrefs(order = listOf("rooms", "invites", "direct_messages"), hidden = setOf("invites"), folded = setOf("rooms")), store.saved!!["!s:x"])
        layouts.toggleFold("!s:x", "rooms")
        layouts.hideAll("!s:x", keys)
        assertEquals(keys.toSet(), layouts.prefs.value["!s:x"]!!.hidden)
        layouts.showAll("!s:x")
        assertTrue(layouts.prefs.value["!s:x"]!!.hidden.isEmpty())
        assertTrue(layouts.prefs.value["!s:x"]!!.folded.isEmpty())
        // Other spaces keep theirs; the next launch reads them.
        assertNull(layouts.prefs.value["!other:x"])
        assertEquals(store.saved, layouts(store).prefs.value)
    }

    @Test
    fun theRoomsTabsGroupsFoldPerSpaceHomeToo() = runTest {
        val store = MemoryStore<Map<String, SectionPrefs>>()
        val layouts = layouts(store)
        layouts.toggleGroupFold("!s:x")
        // By the group's key, saved by Home's.
        layouts.toggleGroupFold(HOME_GROUP)
        layouts.toggleFold("!t:x", "rooms")
        assertEquals(setOf("!s:x", HOME_SAVED), store.saved!!.filterValues { it.groupFolded }.keys)
        assertEquals(setOf("!s:x", HOME_GROUP), foldedGroups(store.saved!!).toSet())
        // Its sections' folding apart.
        assertEquals(SectionPrefs(groupFolded = true), store.saved!!["!s:x"])
        layouts.toggleGroupFold("!s:x")
        assertEquals(listOf(HOME_GROUP), foldedGroups(layouts(store).prefs.value))
    }

    @Test
    fun theSettingsSwitchAsksTheSession() = runTest {
        val session = FakeSession()
        val account = testAccount(FakeMtrx(restored = session), scope())
        account.start()
        account.signedIn.setNotesAtTop(false)
        assertEquals(listOf("notes at top false"), session.roomCalls)
    }
}
