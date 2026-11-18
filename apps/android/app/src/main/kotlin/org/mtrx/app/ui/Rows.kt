package org.mtrx.app.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Badge
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

// Rows and dialogs shared by the lists and Room info: compact (#47).

/**
 * A compact row: [leading] (an avatar), the [title] and a grey [subtitle],
 * and [trailing]; [bold] for unread.
 */
@Composable
fun CompactRow(
    title: String,
    subtitle: String?,
    onClick: (() -> Unit)?,
    modifier: Modifier = Modifier,
    bold: Boolean = false,
    leading: @Composable () -> Unit = {},
    trailing: @Composable () -> Unit = {},
) {
    Row(
        modifier
            .fillMaxWidth()
            .let { if (onClick != null) it.clickable(onClick = onClick) else it }
            .heightIn(min = 48.dp)
            .padding(horizontal = 16.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        leading()
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(
                title,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = if (bold) FontWeight.SemiBold else FontWeight.Normal,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (subtitle != null) {
                Text(
                    subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        Spacer(Modifier.width(8.dp))
        trailing()
    }
}

/** An unread count, red for mentions. */
@Composable
fun UnreadBadge(count: Long, mentions: Boolean = false) {
    Badge(
        containerColor = if (mentions) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary,
        contentColor = if (mentions) MaterialTheme.colorScheme.onError else MaterialTheme.colorScheme.onPrimary,
    ) { Text(if (count > 99) "99+" else count.toString()) }
}

/** A section's title, in the primary colour. */
@Composable
fun SectionHeading(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp),
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
    )
}

/**
 * A setting chosen from [options] (value, label), as a menu; the current
 * one ticked and shown, with its [explanation] below.
 */
@Composable
fun <T> ChoiceRow(
    label: String,
    current: T,
    options: List<Pair<T, String>>,
    explanation: String?,
    enabled: Boolean,
    onChoose: (T) -> Unit,
) {
    MenuRow(
        label,
        options.firstOrNull { it.first == current }?.second.orEmpty(),
        options.map { (value, text) -> MenuEntry(text, value == current) { if (value != current) onChoose(value) } },
        explanation,
        enabled,
    )
}

/** An entry of a [MenuRow]'s menu, ticked if [current]. */
data class MenuEntry(val text: String, val current: Boolean, val onChoose: () -> Unit)

/**
 * A setting showing [shown], whose menu has [entries]; with its
 * [explanation] below.
 */
@Composable
fun MenuRow(
    label: String,
    shown: String,
    entries: List<MenuEntry>,
    explanation: String?,
    enabled: Boolean,
) {
    var open by remember { mutableStateOf(false) }
    Column(
        Modifier
            .fillMaxWidth()
            .clickable(enabled = enabled) { open = true }
            .padding(horizontal = 16.dp, vertical = 8.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(label, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
            Box {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        shown,
                        style = MaterialTheme.typography.bodyMedium,
                        color = if (enabled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    if (enabled) Icon(Icons.Filled.ArrowDropDown, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
                }
                DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                    for (entry in entries) {
                        DropdownMenuItem(
                            text = { Text(entry.text) },
                            onClick = {
                                open = false
                                entry.onChoose()
                            },
                            trailingIcon = if (entry.current) {
                                { Icon(Icons.Filled.Check, contentDescription = "Selected") }
                            } else {
                                null
                            },
                        )
                    }
                }
            }
        }
        if (explanation != null) {
            Text(explanation, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** A setting turned on or off, with its [explanation] below. */
@Composable
fun SwitchRow(label: String, checked: Boolean, explanation: String?, enabled: Boolean = true, onChange: (Boolean) -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(enabled = enabled) { onChange(!checked) }
            .padding(horizontal = 16.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(Modifier.weight(1f)) {
            Text(label, style = MaterialTheme.typography.bodyLarge)
            if (explanation != null) {
                Text(explanation, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        Switch(checked = checked, onCheckedChange = onChange, enabled = enabled)
    }
}

/** Asks before something that can't be undone; [content] below the [message], if any. */
@Composable
fun ConfirmDialog(
    title: String,
    message: String?,
    button: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
    destructive: Boolean = false,
    confirmEnabled: Boolean = true,
    content: @Composable () -> Unit = {},
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                if (message != null) Text(message)
                content()
            }
        },
        confirmButton = {
            TextButton(onClick = onConfirm, enabled = confirmEnabled) {
                Text(button, color = if (destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary)
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
