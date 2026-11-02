package org.mtrx.app.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import kotlin.math.roundToInt
import uniffi.mtrx.SectionChoice

/** What the Show sheet does, by section key and position. */
class ShowActions(
    val setShown: (String, Boolean) -> Unit,
    val showAll: () -> Unit,
    val hideAll: () -> Unit,
    val move: (from: Int, to: Int) -> Unit,
)

/** The Show sheet of [spaceName]'s list, over the screen. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ShowSheet(spaceName: String, choices: List<SectionChoice>, actions: ShowActions, onDismiss: () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        ShowSheetContent(spaceName, choices, actions)
    }
}

private val ROW_HEIGHT = 52.dp

/**
 * Which sections [spaceName]'s list shows, on this phone: a tick for each,
 * with its number of rooms, Show all and Hide all, and a handle to drag a
 * section to its place.
 */
@Composable
fun ShowSheetContent(spaceName: String, choices: List<SectionChoice>, actions: ShowActions) {
    // Taller at large text sizes.
    val rowHeight = maxOf(ROW_HEIGHT, linesHeight(MaterialTheme.typography.bodyLarge) + 16.dp)
    val rowPx = with(LocalDensity.current) { rowHeight.toPx() }
    // The section being dragged, and how far.
    var dragging by remember { mutableStateOf<Int?>(null) }
    var offset by remember { mutableFloatStateOf(0f) }
    Column(Modifier.padding(bottom = 16.dp)) {
        Row(
            Modifier
                .fillMaxWidth()
                .padding(start = 24.dp, end = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("Show in $spaceName", Modifier.weight(1f), style = MaterialTheme.typography.titleMedium)
            TextButton(onClick = actions.showAll) { Text("Show all") }
            TextButton(onClick = actions.hideAll) { Text("Hide all") }
        }
        Text(
            "On this phone only. Drag to reorder.",
            Modifier.padding(horizontal = 24.dp),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        choices.forEachIndexed { index, choice ->
            val position by rememberUpdatedState(index)
            val dragged = dragging == index
            Surface(
                Modifier
                    .fillMaxWidth()
                    .height(rowHeight)
                    .zIndex(if (dragged) 1f else 0f)
                    .graphicsLayer { translationY = if (dragged) offset else 0f },
                tonalElevation = if (dragged) 6.dp else 0.dp,
                shadowElevation = if (dragged) 4.dp else 0.dp,
            ) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clickable { actions.setShown(choice.key, !choice.shown) }
                        .padding(start = 12.dp, end = 8.dp)
                        .semantics {
                            customActions = listOfNotNull(
                                CustomAccessibilityAction("Move up") { actions.move(position, position - 1); true }
                                    .takeIf { position > 0 },
                                CustomAccessibilityAction("Move down") { actions.move(position, position + 1); true }
                                    .takeIf { position < choices.size - 1 },
                            )
                        },
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Checkbox(checked = choice.shown, onCheckedChange = { actions.setShown(choice.key, it) })
                    Spacer(Modifier.width(4.dp))
                    Text(
                        choice.title,
                        Modifier.weight(1f),
                        style = MaterialTheme.typography.bodyLarge,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        choice.rooms.toString(),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Icon(
                        MtrxIcons.DragHandle,
                        contentDescription = "Drag to reorder",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier
                            .padding(8.dp)
                            .pointerInput(choice.key) {
                                detectDragGestures(
                                    onDragStart = {
                                        dragging = position
                                        offset = 0f
                                    },
                                    onDragEnd = {
                                        val from = dragging
                                        if (from != null) {
                                            val to = (from + (offset / rowPx).roundToInt()).coerceIn(0, choices.size - 1)
                                            if (to != from) actions.move(from, to)
                                        }
                                        dragging = null
                                        offset = 0f
                                    },
                                    onDragCancel = {
                                        dragging = null
                                        offset = 0f
                                    },
                                ) { change, amount ->
                                    change.consume()
                                    offset += amount.y
                                }
                            },
                    )
                }
            }
        }
    }
}

/** Settings' Room list part: Notes at the top, the account's choice for all its devices. */
@Composable
fun RoomListSettings(notesAtTop: Boolean, onNotesAtTop: (Boolean) -> Unit) {
    SwitchRow(
        "Notes at the top",
        checked = notesAtTop,
        explanation = "Your notes to self stay at the top of Chats and of every space.",
        onChange = onNotesAtTop,
    )
}
