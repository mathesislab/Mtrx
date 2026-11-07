package org.mtrx.app.model

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import org.mtrx.app.FakeMtrx
import org.mtrx.app.FakeSession
import org.mtrx.app.testAccount

@OptIn(ExperimentalCoroutinesApi::class)
class PreferenceTest {
    private fun TestScope.scope() = CoroutineScope(UnconfinedTestDispatcher(testScheduler))

    /** A store whose [load] reads the saved value, then waits for [loading]. */
    private class SlowStore<T>(saved: T?) : PreferenceStore<T> {
        val memory = MemoryStore(saved)
        val loading = CompletableDeferred<Unit>()

        override suspend fun load(): T? {
            val read = memory.saved
            loading.await()
            return read
        }

        override suspend fun save(value: T) = memory.save(value)
    }

    @Test
    fun withoutASavedValueTheDefaultIsUsed() = runTest {
        assertEquals(RoomSort.DEFAULT, Preference(MemoryStore(), RoomSort.DEFAULT, scope()).value.value)
    }

    @Test
    fun theChosenValueIsSavedAndRestoredAtTheNextLaunch() = runTest {
        val store = MemoryStore<RoomSort>()
        val first = Preference(store, RoomSort.DEFAULT, scope())
        first.set(RoomSort.Z_TO_A)
        assertEquals(RoomSort.Z_TO_A, first.value.value)
        assertEquals(RoomSort.Z_TO_A, store.saved)

        assertEquals(RoomSort.Z_TO_A, Preference(store, RoomSort.DEFAULT, scope()).value.value)
    }

    @Test
    fun aChoiceBeforeTheSavedValueIsReadWins() = runTest {
        val store = SlowStore(RoomSort.A_TO_Z)
        val choice = Preference(store, RoomSort.DEFAULT, scope())
        choice.set(RoomSort.RECENT)
        store.loading.complete(Unit)
        assertEquals(RoomSort.RECENT, choice.value.value)
        assertEquals(RoomSort.RECENT, store.memory.saved)
    }

    @Test
    fun currentWaitsForTheSavedValue() = runTest {
        val store = SlowStore(false)
        val sharing = Preference(store, true, scope())
        val current = scope().async { sharing.current() }
        assertFalse(current.isCompleted)
        store.loading.complete(Unit)
        assertEquals(false, current.await())
    }

    @Test
    fun anUpdateChangesTheLatestValue() = runTest {
        val store = MemoryStore(listOf("a"))
        val list = Preference(store, emptyList(), scope())
        list.update { it + "b" }
        list.update { it + "c" }
        assertEquals(listOf("a", "b", "c"), store.saved)
    }

    @Test
    fun signingOutKeepsTheOrder() = runTest {
        val store = MemoryStore(RoomSort.RECENT)
        val account = testAccount(FakeMtrx(restored = FakeSession()), scope(), sort = store)
        account.start()
        account.signOut()
        assertEquals(RoomSort.RECENT, account.roomSort.value.value)
        assertEquals(RoomSort.RECENT, store.saved)
    }
}
