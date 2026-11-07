package org.mtrx.app.rooms

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material3.AssistChip
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.mtrx.app.ui.MtrxIcons
import uniffi.mtrx.RoomLink
import uniffi.mtrx.Topic
import uniffi.mtrx.linkUrl as libraryLinkUrl

/**
 * The library's rule for a typed address: the address saved, or null if it
 * isn't a web address. The screenshots, without the library, give their
 * own.
 */
val LocalLinkUrl = staticCompositionLocalOf<(String) -> String?> { ::libraryLinkUrl }

/** The topic's text in full, selectable; nothing without one. */
@Composable
fun TopicText(topic: Topic, modifier: Modifier = Modifier) {
    if (topic.text.isBlank()) return
    SelectionContainer(modifier) {
        Text(topic.text, style = MaterialTheme.typography.bodyMedium)
    }
}

/**
 * Opens a topic's call or link with [onOpenLink] (its URL): a call's
 * passcode is copied first, as the call's app opens on top and pasting is
 * the easiest way in; [snackbar] says so, just before the call opens.
 */
@Composable
fun rememberRoomLinkOpener(onOpenLink: (String) -> Unit, snackbar: SnackbarHostState): (RoomLink) -> Unit {
    @Suppress("DEPRECATION") // LocalClipboard's API is suspending; this is a plain copy.
    val clipboard = LocalClipboardManager.current
    val scope = rememberCoroutineScope()
    val open by rememberUpdatedState(onOpenLink)
    return remember<(RoomLink) -> Unit>(clipboard, scope, snackbar) {
        { link: RoomLink ->
            val passcode = link.passcode
            if (passcode == null) {
                open(link.url)
            } else {
                clipboard.setText(AnnotatedString(passcode))
                scope.launch { snackbar.showSnackbar("Passcode $passcode copied") }
                scope.launch {
                    // Seen before the call's app opens on top.
                    delay(PASSCODE_NOTICE_MS)
                    open(link.url)
                }
            }
        }
    }
}

private const val PASSCODE_NOTICE_MS = 700L

/** The topic's calls, then its links, as buttons: a call's passcode on a second line; [onOpen] with the one tapped. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun TopicLinks(topic: Topic, onOpen: (RoomLink) -> Unit, modifier: Modifier = Modifier) {
    if (!topic.hasLinks) return
    FlowRow(modifier, horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        for ((call, links) in listOf(true to topic.calls, false to topic.links)) {
            for (link in links) {
                FilledTonalButton(onClick = { onOpen(link) }) {
                    LinkIcon(call, Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Column {
                        Text(link.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        link.passcode?.let {
                            Text("Passcode $it", style = MaterialTheme.typography.labelSmall, maxLines = 1)
                        }
                    }
                }
            }
        }
    }
}

/** A space's calls, then its links, as small chips in centred rows; [onOpen] with the one tapped. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun LinkChips(topic: Topic, onOpen: (RoomLink) -> Unit, modifier: Modifier = Modifier) {
    if (!topic.hasLinks) return
    FlowRow(modifier, horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally)) {
        for ((call, links) in listOf(true to topic.calls, false to topic.links)) {
            for (link in links) {
                AssistChip(
                    onClick = { onOpen(link) },
                    label = { Text(link.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    leadingIcon = { LinkIcon(call, Modifier.size(18.dp)) },
                )
            }
        }
    }
}

@Composable
private fun LinkIcon(call: Boolean, modifier: Modifier = Modifier) {
    Icon(if (call) MtrxIcons.Videocam else MtrxIcons.Link, contentDescription = if (call) "Call" else "Link", modifier = modifier)
}

/**
 * The room's calls and links in its top bar: Join while it has a call (a
 * tap joins the room's call, a long press opens the menu), else a link
 * icon that opens the menu. The menu has the calls, then the links, each
 * opened with [onOpen], then Edit links… ([onEdit]) where we may.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun RoomLinksButton(links: HeaderLinks, onOpen: (RoomLink) -> Unit, onEdit: () -> Unit) {
    var menu by remember { mutableStateOf(false) }
    Box {
        val call = links.join
        if (call != null) {
            Surface(
                Modifier
                    .padding(horizontal = 4.dp)
                    .clip(CircleShape)
                    .combinedClickable(
                        onLongClickLabel = "Calls and links",
                        onLongClick = { menu = true },
                        onClick = { onOpen(call) },
                    )
                    .semantics { contentDescription = links.joinLabel ?: "Join" },
                shape = CircleShape,
                color = MaterialTheme.colorScheme.secondaryContainer,
                contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
            ) {
                Row(Modifier.padding(horizontal = 12.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(MtrxIcons.Videocam, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text("Join", style = MaterialTheme.typography.labelLarge, maxLines = 1)
                }
            }
        } else {
            IconButton(onClick = { menu = true }) { Icon(MtrxIcons.Link, contentDescription = "Links") }
        }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            links.groups.forEachIndexed { index, group ->
                if (index > 0) HorizontalDivider()
                Text(
                    group.title,
                    Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                for (link in group.links) {
                    DropdownMenuItem(
                        text = {
                            Column {
                                Text(link.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                link.passcode?.let {
                                    Text(
                                        "Passcode $it",
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                            }
                        },
                        leadingIcon = { LinkIcon(group.call, Modifier.size(20.dp)) },
                        onClick = {
                            menu = false
                            onOpen(link)
                        },
                    )
                }
            }
            if (links.canEdit) {
                HorizontalDivider()
                DropdownMenuItem(
                    text = { Text("Edit links…") },
                    leadingIcon = { Icon(Icons.Filled.Edit, contentDescription = null, modifier = Modifier.size(20.dp)) },
                    onClick = {
                        menu = false
                        onEdit()
                    },
                )
            }
        }
    }
}

/**
 * The topic editor: Cancel and Save on top, always in reach; the text,
 * then the calls and the links, each with a title, URL (red while it isn't
 * a web address) and a call's passcode, dragged by its handle to reorder
 * within its kind: the first call is the room's. [onSave] with the draft.
 */
@Composable
fun TopicEditor(initial: Topic, onSave: (TopicDraft) -> Unit, onCancel: () -> Unit, modifier: Modifier = Modifier) {
    var draft by remember(initial) { mutableStateOf(TopicDraft.of(initial)) }
    val linkUrl = LocalLinkUrl.current
    Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text("Edit topic", Modifier.weight(1f), style = MaterialTheme.typography.titleSmall)
            TextButton(onClick = onCancel) { Text("Cancel") }
            Spacer(Modifier.width(4.dp))
            FilledTonalButton(onClick = { onSave(draft) }) { Text("Save") }
        }
        OutlinedTextField(
            value = draft.text,
            onValueChange = { draft = draft.copy(text = it) },
            modifier = Modifier.fillMaxWidth(),
            label = { Text("Topic") },
            placeholder = { Text("What the room is about") },
            minLines = 2,
            maxLines = 6,
        )
        for (call in listOf(true, false)) {
            val links = draft.kind(call)
            if (links.isEmpty()) continue
            Text(
                if (call) "Calls" else "Links",
                Modifier.padding(top = 4.dp),
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.primary,
            )
            ReorderColumn(links.map { it.id }, onMove = { from, to -> draft = draft.move(call, from, to) }) { index, handle ->
                LinkEditor(
                    links[index],
                    call,
                    valid = linkUrl(links[index].url) != null,
                    handle = handle,
                    onChange = { draft = draft.change(call, index, it) },
                    onRemove = { draft = draft.remove(call, index) },
                )
            }
            if (call) {
                Text(
                    "The first call is the room's: Join opens it.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = { draft = draft.add(call = true) }) {
                Icon(MtrxIcons.Videocam, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("Add call")
            }
            TextButton(onClick = { draft = draft.add(call = false) }) {
                Icon(MtrxIcons.Link, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("Add link")
            }
        }
        if (draft.dropped(linkUrl) > 0) {
            Text(
                "Links without a web address are left out.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/** A call's or link's fields beside its drag [handle]: title, URL and a call's passcode. */
@Composable
private fun LinkEditor(
    link: TopicDraft.Link,
    call: Boolean,
    valid: Boolean,
    handle: Modifier,
    onChange: (TopicDraft.Link) -> Unit,
    onRemove: () -> Unit,
) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
        Icon(
            MtrxIcons.DragHandle,
            contentDescription = "Drag to reorder",
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = handle.padding(top = 16.dp, end = 8.dp, bottom = 16.dp),
        )
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            OutlinedTextField(
                value = link.title,
                onValueChange = { onChange(link.copy(title = it)) },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Title") },
                singleLine = true,
            )
            OutlinedTextField(
                value = link.url,
                onValueChange = { onChange(link.copy(url = it)) },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("URL") },
                placeholder = { Text(if (call) "https://zoom.us/j/123456789" else "https://…") },
                isError = link.url.isNotBlank() && !valid,
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
            )
            if (call) {
                OutlinedTextField(
                    value = link.passcode,
                    onValueChange = { onChange(link.copy(passcode = it)) },
                    modifier = Modifier.fillMaxWidth(),
                    label = { Text("Passcode") },
                    singleLine = true,
                )
            }
        }
        IconButton(onClick = onRemove) { Icon(Icons.Filled.Close, contentDescription = "Remove") }
    }
}

/**
 * The rows of [keys], one under the other, each dragged by its handle (the
 * modifier [row] gets): it moves under the finger, and passing the middle
 * of a neighbour swaps the two ([onMove] with the places), so rows of any
 * height reorder as they are dragged. Move up and Move down for
 * accessibility.
 */
@Composable
private fun ReorderColumn(keys: List<Long>, onMove: (Int, Int) -> Unit, row: @Composable (Int, Modifier) -> Unit) {
    var dragged by remember { mutableStateOf<Long?>(null) }
    var offset by remember { mutableFloatStateOf(0f) }
    val heights = remember { mutableStateMapOf<Long, Int>() }
    val order by rememberUpdatedState(keys)
    val move by rememberUpdatedState(onMove)
    val spacing = with(LocalDensity.current) { ROW_SPACING.toPx() }
    Column(verticalArrangement = Arrangement.spacedBy(ROW_SPACING)) {
        keys.forEachIndexed { index, id ->
            key(id) {
                val handle = Modifier.pointerInput(id) {
                    // The order as moved so far: the parent's catches up a frame later.
                    var places = order
                    detectDragGestures(
                        onDragStart = {
                            places = order
                            dragged = id
                            offset = 0f
                        },
                        onDragEnd = { dragged = null },
                        onDragCancel = { dragged = null },
                    ) { change, amount ->
                        change.consume()
                        offset += amount.y
                        val at = places.indexOf(id)
                        if (at < 0) return@detectDragGestures
                        val below = places.getOrNull(at + 1)?.let { (heights[it] ?: 0) + spacing }
                        val above = places.getOrNull(at - 1)?.let { (heights[it] ?: 0) + spacing }
                        val to = when {
                            below != null && offset > below / 2 -> {
                                offset -= below
                                at + 1
                            }
                            above != null && offset < -above / 2 -> {
                                offset += above
                                at - 1
                            }
                            else -> return@detectDragGestures
                        }
                        places = places.toMutableList().apply { add(to, removeAt(at)) }
                        move(at, to)
                    }
                }
                val moving = dragged == id
                Box(
                    Modifier
                        .onSizeChanged { heights[id] = it.height }
                        .zIndex(if (moving) 1f else 0f)
                        .graphicsLayer { translationY = if (moving) offset else 0f }
                        .semantics {
                            customActions = listOfNotNull(
                                CustomAccessibilityAction("Move up") { move(index, index - 1); true }.takeIf { index > 0 },
                                CustomAccessibilityAction("Move down") { move(index, index + 1); true }
                                    .takeIf { index < keys.lastIndex },
                            )
                        },
                ) {
                    row(index, handle)
                }
            }
        }
    }
}

private val ROW_SPACING = 8.dp
