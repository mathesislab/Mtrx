package org.mtrx.app.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import org.mtrx.app.model.Account
import org.mtrx.app.model.PresenceModel
import org.mtrx.app.model.PushRegistration
import org.mtrx.app.model.showsEncryptionBanner
import org.mtrx.app.ui.RoomListSettings
import uniffi.mtrx.EncryptionState
import uniffi.mtrx.RecoveryStatus

/** What the settings show of the session. */
data class SessionInfo(val userId: String, val homeserver: String, val deviceId: String)

/**
 * The Settings tab: the account, the room list (Notes at the top),
 * notifications, privacy (sharing our
 * presence), encryption (its state, recovery and verification), and
 * signing out.
 */
@Composable
fun SettingsContent(
    account: Account,
    push: PushRegistration,
    presence: PresenceModel,
    info: SessionInfo,
    onEnterRecoveryKey: () -> Unit,
    onVerify: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val encryption by account.encryption.collectAsStateWithLifecycle()
    val devices by account.devices.collectAsStateWithLifecycle()
    val rooms by account.rooms.collectAsStateWithLifecycle()
    LaunchedEffect(account) { account.loadDevices() }
    // Unknown (not loaded, or failed) counts as having one: the request says.
    val otherDevice = devices?.any { !it.current } ?: true
    var confirmSignOut by rememberSaveable { mutableStateOf(false) }

    Column(
        modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState()),
    ) {
        SectionTitle("Account")
        Line("User", info.userId)
        Line("Server", info.homeserver)
        Line("Device", info.deviceId)
        HorizontalDivider(Modifier.padding(vertical = 8.dp))
        SectionTitle("Room list")
        RoomListSettings(rooms.notesAtTop(), onNotesAtTop = { account.setNotesAtTop(it) })
        HorizontalDivider(Modifier.padding(vertical = 8.dp))
        SectionTitle("Notifications")
        NotificationsSection(push)
        HorizontalDivider(Modifier.padding(vertical = 8.dp))
        SectionTitle("Privacy")
        PrivacySection(account, presence)
        HorizontalDivider(Modifier.padding(vertical = 8.dp))
        SectionTitle("Encryption")
        EncryptionSection(encryption, otherDevice, onEnterRecoveryKey, onVerify)
        HorizontalDivider(Modifier.padding(vertical = 8.dp))
        ListItem(
            headlineContent = { Text("Sign out", color = MaterialTheme.colorScheme.error) },
            modifier = Modifier.clickable { confirmSignOut = true },
        )
    }

    if (confirmSignOut) {
        AlertDialog(
            onDismissRequest = { confirmSignOut = false },
            title = { Text("Sign out?") },
            text = { Text("Messages and keys of this account are deleted from this device.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmSignOut = false
                    account.signOut()
                }) { Text("Sign out") }
            },
            dismissButton = {
                TextButton(onClick = { confirmSignOut = false }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun EncryptionSection(
    state: EncryptionState?,
    otherDevice: Boolean,
    onEnterRecoveryKey: () -> Unit,
    onVerify: () -> Unit,
) {
    if (state == null) {
        Line("This device", "Checking…")
        return
    }
    Line("This device", if (state.deviceVerified) "Verified" else "Not verified")
    Line("Recovery", recoveryLabel(state.recovery))
    if (!showsEncryptionBanner(state)) return
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
        Text(
            "Verify this device or enter your recovery key to read encrypted messages.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (state.recovery != RecoveryStatus.DISABLED) {
            OutlinedButton(onClick = onEnterRecoveryKey, modifier = Modifier.padding(top = 12.dp)) {
                Text("Enter recovery key")
            }
        }
        if (!state.deviceVerified) {
            if (otherDevice) {
                OutlinedButton(onClick = onVerify, modifier = Modifier.padding(top = 8.dp)) {
                    Text("Verify with another device")
                }
            } else {
                Text(
                    "Verifying needs another device signed in to this account, such as the desktop app.",
                    Modifier.padding(top = 12.dp),
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        }
    }
}

private fun recoveryLabel(status: RecoveryStatus): String = when (status) {
    RecoveryStatus.ENABLED -> "Set up"
    RecoveryStatus.DISABLED -> "Not set up"
    RecoveryStatus.INCOMPLETE -> "Not unlocked on this device"
    RecoveryStatus.UNKNOWN -> "Not known yet"
}

@Composable
private fun SectionTitle(text: String) {
    Text(
        text,
        Modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp),
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
    )
}

@Composable
private fun Line(label: String, value: String) {
    ListItem(
        headlineContent = { Text(value) },
        overlineContent = { Text(label) },
    )
}
