package org.mtrx.app.timeline

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import org.mtrx.app.rooms.TopicEditor
import org.mtrx.app.rooms.TopicLinks
import org.mtrx.app.rooms.TopicText
import org.mtrx.app.ui.MtrxIcons
import uniffi.mtrx.Topic

/**
 * The topic panel, dropped over the timeline below the header: the topic's
 * text in full, its links as buttons, the pinned messages (tapped for
 * [onPinned], with their event ID), and Edit where we may ([canEdit]);
 * while [editing], the topic editor. A tap outside closes it, except while
 * editing.
 */
@Composable
fun TopicPanel(
    topic: Topic,
    pinned: List<EventRow>?,
    pinnedCount: Int,
    canEdit: Boolean,
    editing: Boolean,
    onEdit: () -> Unit,
    onSave: (Topic) -> Unit,
    onCancel: () -> Unit,
    onClose: () -> Unit,
    onOpenLink: (String) -> Unit,
    onPinned: (String) -> Unit,
) {
    Box(
        Modifier
            .fillMaxSize()
            .background(Color.Black.copy(alpha = 0.25f))
            .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {
                if (!editing) onClose()
            },
    ) {
        Surface(
            Modifier
                .fillMaxWidth()
                .heightIn(max = 520.dp)
                // Taps inside stay inside.
                .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {},
            shape = RoundedCornerShape(bottomStart = 16.dp, bottomEnd = 16.dp),
            tonalElevation = 3.dp,
            shadowElevation = 6.dp,
        ) {
            Column(
                Modifier
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp, vertical = 12.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                if (editing) {
                    TopicEditor(topic, onSave = onSave, onCancel = onCancel)
                    return@Column
                }
                Row(verticalAlignment = Alignment.Top) {
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        if (topic.text.isBlank() && topic.links.isEmpty()) {
                            Text(
                                "No topic or links yet.",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        TopicText(topic)
                    }
                    if (canEdit) TextButton(onClick = onEdit) { Text("Edit") }
                }
                TopicLinks(topic.links, onOpen = onOpenLink)
                if (pinnedCount > 0) {
                    HorizontalDivider()
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(MtrxIcons.Pin, contentDescription = null, modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.primary)
                        Spacer(Modifier.width(6.dp))
                        Text("Pinned", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                    }
                    when {
                        pinned == null -> CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                        pinned.isEmpty() -> Text(
                            "The pinned messages could not be loaded.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        else -> pinned.forEach { row -> PinnedRow(row, onPinned) }
                    }
                }
            }
        }
    }
}

/** A pinned message: its sender and first line, tapped to go to it. */
@Composable
private fun PinnedRow(row: EventRow, onPinned: (String) -> Unit) {
    val eventId = row.event.eventId
    Column(
        Modifier
            .fillMaxWidth()
            .clickable(enabled = eventId != null) { eventId?.let(onPinned) }
            .padding(vertical = 4.dp),
    ) {
        Text(
            row.event.sender.displayName,
            style = MaterialTheme.typography.labelLarge,
            fontWeight = FontWeight.SemiBold,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            preview(row),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
    }
}
