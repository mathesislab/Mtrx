package org.mtrx.app.model

import uniffi.mtrx.MtrxException

/** What to show for a failed library call; null for a cancelled sign-in. */
fun describe(error: MtrxException): String? = when (error) {
    is MtrxException.LoginCancelled -> null
    is MtrxException.NotLoggedIn -> "Not signed in"
    is MtrxException.AlreadyLoggedIn -> "Already signed in; sign out first"
    is MtrxException.RoomNotFound -> "No room ${error.roomId}"
    is MtrxException.InvalidInput -> error.reason
    is MtrxException.SecretStorage -> "Secret storage: ${error.reason}"
    is MtrxException.PresenceUnavailable -> "This server does not share presence"
    is MtrxException.EventNotFound -> "The original message was not found in the room's history"
    is MtrxException.Other -> error.reason
}
