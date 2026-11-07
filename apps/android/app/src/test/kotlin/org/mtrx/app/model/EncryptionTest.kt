package org.mtrx.app.model

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mtrx.app.FakeMtrx
import org.mtrx.app.FakeSession
import org.mtrx.app.FakeSortStore
import org.mtrx.app.FakeSpaceStore
import org.mtrx.app.FakeVerification
import org.mtrx.app.signedIn
import org.mtrx.app.testAccount
import uniffi.mtrx.EncryptionState
import uniffi.mtrx.MtrxException
import uniffi.mtrx.RecoveryStatus
import uniffi.mtrx.RecoveryStep
import uniffi.mtrx.VerificationState

@OptIn(ExperimentalCoroutinesApi::class)
class EncryptionTest {
    private fun state(verified: Boolean, recovery: RecoveryStatus) = EncryptionState(verified, recovery)

    @Test
    fun bannerWhileNotVerifiedOrRecoveryNotUnlocked() {
        assertFalse(showsEncryptionBanner(null))
        assertFalse(showsEncryptionBanner(state(false, RecoveryStatus.UNKNOWN)))
        assertFalse(showsEncryptionBanner(state(true, RecoveryStatus.ENABLED)))
        assertFalse(showsEncryptionBanner(state(true, RecoveryStatus.DISABLED)))
        assertTrue(showsEncryptionBanner(state(false, RecoveryStatus.ENABLED)))
        assertTrue(showsEncryptionBanner(state(false, RecoveryStatus.DISABLED)))
        assertTrue(showsEncryptionBanner(state(true, RecoveryStatus.INCOMPLETE)))
    }

    private fun TestScope.signedIn(session: FakeSession): Account =
        testAccount(FakeMtrx(restored = session), CoroutineScope(UnconfinedTestDispatcher(testScheduler)), FakeSpaceStore(), FakeSortStore())
            .also { it.start() }

    private val Account.encryption get() = signedIn.encryption

    @Test
    fun followsTheLibraryOnceSyncStarts() = runTest {
        val session = FakeSession()
        val account = signedIn(session)
        // Not known before sync.
        assertEquals(0, session.encryptionSubscriptions)
        assertNull(account.encryption.value)

        account.setForeground(true)
        assertEquals(1, session.encryptionSubscriptions)
        assertEquals(session.encryption, account.encryption.value)
        assertTrue(showsEncryptionBanner(account.encryption.value))

        session.pushEncryption(state(false, RecoveryStatus.UNKNOWN))
        assertFalse(showsEncryptionBanner(account.encryption.value))
        session.pushEncryption(state(true, RecoveryStatus.INCOMPLETE))
        assertTrue(showsEncryptionBanner(account.encryption.value))

        // One subscription for the session, across background and foreground.
        account.setForeground(false)
        account.setForeground(true)
        assertEquals(1, session.encryptionSubscriptions)
        assertFalse(session.encryptionSubscription!!.cancelled)
    }

    @Test
    fun recoveryReportsItsStepsInOrderAndTheBannerGoesWithThePushedState() = runTest {
        val session = FakeSession()
        val account = signedIn(session)
        account.setForeground(true)
        val steps = listOf(
            RecoveryStep.CheckingKey,
            RecoveryStep.ImportingIdentity,
            RecoveryStep.DownloadingKeys(0u, 2u),
            RecoveryStep.DownloadingKeys(1u, 2u),
        )
        val seen = mutableListOf<Recovery>()
        session.recoverySteps = steps
        session.onRecoveryStep = { seen += account.signedIn.recovery.value }

        account.signedIn.recover("EsTc 1234")
        assertEquals(listOf("EsTc 1234"), session.recoveryKeys)
        assertEquals(steps.map { Recovery.Running(it) }, seen)
        assertEquals(Recovery.Done, account.signedIn.recovery.value)
        // The state is the library's to report, not asked for.
        assertTrue(showsEncryptionBanner(account.encryption.value))

        session.pushEncryption(EncryptionState(true, RecoveryStatus.ENABLED, keysArriving = true))
        assertFalse(showsEncryptionBanner(account.encryption.value))
        assertTrue(account.encryption.value!!.keysArriving)
        session.pushEncryption(EncryptionState(true, RecoveryStatus.ENABLED, keysArriving = false))
        assertFalse(account.encryption.value!!.keysArriving)

        account.signedIn.resetRecovery()
        assertEquals(Recovery.Idle, account.signedIn.recovery.value)
    }

    @Test
    fun failedRecoveryGivesTheReason() = runTest {
        val session = FakeSession().apply {
            recoverySteps = listOf(RecoveryStep.CheckingKey)
            recoveryError = MtrxException.Other("The Base58 decoded key has an invalid length")
        }
        val account = testAccount(FakeMtrx(restored = session), CoroutineScope(UnconfinedTestDispatcher(testScheduler)), FakeSpaceStore(), FakeSortStore())
        account.start()
        account.signedIn.recover("wrong")
        assertEquals(Recovery.Failed("The Base58 decoded key has an invalid length"), account.signedIn.recovery.value)
    }

    @Test
    fun invalidKeyIsShownUnderTheField() = runTest {
        val session = FakeSession().apply {
            recoverySteps = listOf(RecoveryStep.CheckingKey)
            recoveryError = MtrxException.InvalidInput("The recovery key is not valid")
        }
        val account = testAccount(FakeMtrx(restored = session), CoroutineScope(UnconfinedTestDispatcher(testScheduler)), FakeSpaceStore(), FakeSortStore())
        account.start()
        account.signedIn.recover("EsTc not a key")
        assertEquals(Recovery.InvalidKey, account.signedIn.recovery.value)
        assertEquals("The recovery key is not valid.", INVALID_RECOVERY_KEY)
    }

    @Test
    fun requestFromAnotherUserIsCancelled() = runTest {
        val session = FakeSession()
        val account = testAccount(FakeMtrx(restored = session), CoroutineScope(UnconfinedTestDispatcher(testScheduler)), FakeSpaceStore(), FakeSortStore())
        account.start()
        val request = FakeVerification("@eve:example.org")
        session.verificationRequests!!.onRequest(request)
        assertTrue(request.cancelled)
        assertTrue(request.uniffiIsDestroyed)
        assertNull(account.signedIn.verificationRequest.value)
    }

    @Test
    fun requestFromOurOtherDeviceIsAcceptedOrDeclined() = runTest {
        val session = FakeSession()
        val account = testAccount(FakeMtrx(restored = session), CoroutineScope(UnconfinedTestDispatcher(testScheduler)), FakeSpaceStore(), FakeSortStore())
        account.start()
        val signedIn = account.signedIn

        val declined = FakeVerification(FakeSession.USER_ID)
        session.verificationRequests!!.onRequest(declined)
        assertEquals(declined, signedIn.verificationRequest.value)
        signedIn.declineVerificationRequest()
        assertTrue(declined.cancelled)
        assertNull(signedIn.verificationRequest.value)

        val accepted = FakeVerification(FakeSession.USER_ID)
        session.verificationRequests!!.onRequest(accepted)
        signedIn.acceptVerificationRequest()
        assertTrue(accepted.accepted)
        val model = signedIn.verification.value!!
        assertFalse(model.outgoing)
        assertEquals(VerificationState.Waiting, model.state.value)

        // While it runs, another request is declined.
        val second = FakeVerification(FakeSession.USER_ID)
        session.verificationRequests!!.onRequest(second)
        assertTrue(second.cancelled)
        assertNull(signedIn.verificationRequest.value)
    }

    @Test
    fun verifiedDeviceLosesTheBannerAndSignOutClosesTheVerificationAndSubscription() = runTest {
        val session = FakeSession()
        val account = signedIn(session)
        account.setForeground(true)
        val signedIn = account.signedIn
        signedIn.verifyWithAnotherDevice()
        val model = signedIn.verification.value!!
        assertTrue(model.outgoing)
        val first = session.selfVerification
        first.update(VerificationState.Done)
        assertTrue(showsEncryptionBanner(account.encryption.value))
        session.pushEncryption(EncryptionState(true, RecoveryStatus.ENABLED, keysArriving = true))
        assertFalse(showsEncryptionBanner(account.encryption.value))

        // A new one, still waiting at sign-out: cancelled and closed.
        val second = FakeVerification(FakeSession.USER_ID)
        session.selfVerification = second
        signedIn.verifyWithAnotherDevice()
        // The ended one is closed without cancelling.
        assertTrue(first.uniffiIsDestroyed)
        assertFalse(first.cancelled)
        account.signOut()
        assertEquals(AccountState.SignedOut(), account.state.value)
        assertTrue(second.cancelled)
        assertTrue(second.uniffiIsDestroyed)
        val subscription = session.encryptionSubscription!!
        assertTrue(subscription.cancelled)
        assertTrue(subscription.uniffiIsDestroyed)
    }

    @Test
    fun aRequestArrivingAsTheSessionClosesIsDeclined() = runTest {
        val session = FakeSession()
        val account = signedIn(session)
        val listener = session.verificationRequests!!
        account.signOut()
        val late = FakeVerification(FakeSession.USER_ID)
        listener.onRequest(late)
        assertTrue(late.cancelled)
        assertTrue(late.uniffiIsDestroyed)
    }
}
