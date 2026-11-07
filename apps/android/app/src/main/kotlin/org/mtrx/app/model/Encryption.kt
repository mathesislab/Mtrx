package org.mtrx.app.model

import uniffi.mtrx.EncryptionState
import uniffi.mtrx.MtrxException
import uniffi.mtrx.RecoveryStatus
import uniffi.mtrx.RecoveryStep

/**
 * Whether the room list asks to verify this device or enter the recovery
 * key: this device is not verified, or recovery is not unlocked on it. Not
 * while the state is not known (null, or recovery not known before the
 * first sync).
 */
fun showsEncryptionBanner(state: EncryptionState?): Boolean =
    state != null &&
        state.recovery != RecoveryStatus.UNKNOWN &&
        (!state.deviceVerified || state.recovery == RecoveryStatus.INCOMPLETE)

/** A recovery with the recovery key ([SignedInSession.recover]). */
sealed interface Recovery {
    data object Idle : Recovery

    /** [step] is null until the library reports the first. */
    data class Running(val step: RecoveryStep?) : Recovery

    /** The key is mistyped, or of another account or an older backup. */
    data object InvalidKey : Recovery

    data class Failed(val reason: String) : Recovery

    data object Done : Recovery
}

/** Shown under the recovery key's field for [Recovery.InvalidKey]. */
const val INVALID_RECOVERY_KEY = "The recovery key is not valid."

/** A failed recovery: [MtrxException.InvalidInput] is always the key. */
fun recoveryFailure(error: MtrxException): Recovery = when (error) {
    is MtrxException.InvalidInput -> Recovery.InvalidKey
    else -> Recovery.Failed(describe(error) ?: "")
}
