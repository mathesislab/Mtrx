package org.mtrx.app.timeline

import android.view.ContextThemeWrapper
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.emoji2.emojipicker.EmojiPickerView
import org.mtrx.app.model.Images
import org.mtrx.app.ui.Avatar
import uniffi.mtrx.MessageAction
import uniffi.mtrx.Reader

/**
 * Holding a message: the common reactions (ours marked) and "+" for the
 * full picker, then the other [actions] (the row's).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MessageMenu(
    row: EventRow,
    actions: List<MessageAction>,
    onDismiss: () -> Unit,
    onReact: (String) -> Unit,
    onMoreReactions: () -> Unit,
    onAction: (MessageAction) -> Unit,
) {
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        if (MessageAction.REACT in actions) {
            val ours = row.reactions.filter { it.byUs }.map { it.key }.toSet()
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 12.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.SpaceEvenly,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                for (emoji in COMMON_REACTIONS) {
                    Box(
                        Modifier
                            .size(44.dp)
                            .clip(CircleShape)
                            .background(if (emoji in ours) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surface)
                            .clickable { onReact(emoji) },
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(emoji, fontSize = 24.sp)
                    }
                }
                IconButton(onClick = onMoreReactions) {
                    Icon(Icons.Filled.Add, contentDescription = "More reactions")
                }
            }
            HorizontalDivider()
        }
        for (action in actions - MessageAction.REACT) {
            val destructive = action == MessageAction.DELETE || action == MessageAction.DISCARD
            ListItem(
                headlineContent = {
                    Text(action.label, color = if (destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface)
                },
                modifier = Modifier.clickable { onAction(action) },
            )
        }
    }
}

/** The whole set of emoji, to react with one. */
@Composable
fun EmojiPickerDialog(onPick: (String) -> Unit, onDismiss: () -> Unit) {
    val pick by rememberUpdatedState(onPick)
    Dialog(onDismissRequest = onDismiss) {
        Surface(shape = RoundedCornerShape(16.dp), tonalElevation = 6.dp) {
            AndroidView(
                factory = { context ->
                    // The picker's resources expect an AppCompat theme; DayNight follows the system.
                    EmojiPickerView(ContextThemeWrapper(context, androidx.appcompat.R.style.Theme_AppCompat_DayNight)).apply {
                        setOnEmojiPickedListener { item -> pick(item.emoji) }
                    }
                },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(420.dp),
            )
        }
    }
}

/** People with their avatars and names: who reacted, or read up to a message. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PeopleSheet(title: String, people: List<Reader>, images: Images, onDismiss: () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Text(
            title,
            style = MaterialTheme.typography.titleMedium,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        )
        LazyColumn {
            items(people, key = { it.userId }) { person ->
                val name = personName(person)
                ListItem(
                    leadingContent = { Avatar(name, person.avatarUrl, seed = person.userId, images = images, size = 40.dp) },
                    headlineContent = { Text(name) },
                    supportingContent = { Text(person.userId) },
                )
            }
        }
    }
}

/** A message's text, to select part of it. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SelectTextSheet(text: String, onDismiss: () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(
            Modifier
                .verticalScroll(rememberScrollState())
                .padding(start = 16.dp, end = 16.dp, bottom = 24.dp),
        ) {
            SelectionContainer {
                Text(text, style = MaterialTheme.typography.bodyLarge)
            }
        }
    }
}

/** Asks before deleting a message for everyone. */
@Composable
fun ConfirmDeleteDialog(onDelete: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Delete this message for everyone?") },
        confirmButton = {
            TextButton(onClick = onDelete) {
                Text("Delete", color = MaterialTheme.colorScheme.error)
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}
