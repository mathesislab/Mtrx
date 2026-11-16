package org.mtrx.app.settings

import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import java.time.Duration
import org.mtrx.app.model.Account
import org.mtrx.app.model.SignedInSession
import org.mtrx.app.ui.ChoiceRow
import uniffi.mtrx.LimitChoice
import uniffi.mtrx.disappearingLimitChoices

/** The note under "Share my presence"; the switch is disabled when the server does not share presence. */
fun sharePresenceFootnote(available: Boolean): String =
    if (available) "Others see when you are online or idle." else "This server does not share presence."

/**
 * The Settings tab's "Privacy" section: "Share my presence", and how long
 * after sending our disappearing messages go at the latest.
 */
@Composable
fun PrivacySection(account: Account, session: SignedInSession) {
    val sharing by account.sharePresence.value.collectAsStateWithLifecycle()
    val available by session.presence.available.collectAsStateWithLifecycle()
    val limit by session.disappearingLimit.collectAsStateWithLifecycle()
    val limits = remember { disappearingLimitChoices() }
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
    DisappearingLimitRow(limit, limits, onChoose = { session.setDisappearingLimit(it) })
}

/** "Disappearing messages": ours go at the latest [limit] after sending, one of [choices]. */
@Composable
fun DisappearingLimitRow(limit: Duration, choices: List<LimitChoice>, onChoose: (Duration) -> Unit) {
    ChoiceRow(
        "Disappearing messages",
        limit,
        choices.map { it.atMost to it.label },
        "Messages you send to disappear after read are deleted at the latest this long after sending, also when " +
            "not everyone has read them.",
        enabled = true,
        onChoose = onChoose,
    )
}
