package org.mtrx.app.settings

import java.time.Duration
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Test
import org.mtrx.app.FakeMtrx
import org.mtrx.app.FakeSession
import org.mtrx.app.signedIn
import org.mtrx.app.testAccount
import uniffi.mtrx.MtrxException

@OptIn(ExperimentalCoroutinesApi::class)
class PrivacySettingsTest {
    @Test
    fun theDisappearingLimitShowsAtOnceAndAFailedChangeGoesBack() = runTest {
        val session = FakeSession()
        val account = testAccount(FakeMtrx(restored = session), CoroutineScope(UnconfinedTestDispatcher(testScheduler)))
        account.start()
        val signedIn = account.signedIn
        val messages = mutableListOf<String>()
        backgroundScope.launch(UnconfinedTestDispatcher(testScheduler)) { signedIn.messages.collect { messages += it } }
        assertEquals(Duration.ofDays(1), signedIn.disappearingLimit.value)

        signedIn.setDisappearingLimit(Duration.ofHours(6))
        assertEquals(Duration.ofHours(6), signedIn.disappearingLimit.value)
        assertEquals(listOf("limit PT6H"), session.roomCalls)

        // Not saved: the saved one again, and why.
        session.roomError = MtrxException.Other("offline")
        signedIn.setDisappearingLimit(Duration.ofDays(7))
        assertEquals(Duration.ofHours(6), signedIn.disappearingLimit.value)
        assertEquals(listOf("Could not change the limit of disappearing messages: offline"), messages)

        // Changed on another device: followed.
        session.limit = Duration.ofHours(1)
        assertEquals(Duration.ofHours(1), signedIn.disappearingLimit.value)
    }
}
