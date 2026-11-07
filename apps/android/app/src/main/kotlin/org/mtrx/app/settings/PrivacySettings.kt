package org.mtrx.app.settings

import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import org.mtrx.app.model.Account
import org.mtrx.app.model.PresenceModel

/** The note under "Share my presence"; the switch is disabled when the server does not share presence. */
fun sharePresenceFootnote(available: Boolean): String =
    if (available) "Others see when you are online or idle." else "This server does not share presence."

/** The Settings tab's "Privacy" section: "Share my presence". */
@Composable
fun PrivacySection(account: Account, presence: PresenceModel) {
    val sharing by account.sharePresence.value.collectAsStateWithLifecycle()
    val available by presence.available.collectAsStateWithLifecycle()
    ListItem(
        headlineContent = { Text("Share my presence") },
        trailingContent = {
            Switch(checked = sharing, enabled = available, onCheckedChange = account::setSharePresence)
        },
    )
    Text(
        sharePresenceFootnote(available),
        Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}
