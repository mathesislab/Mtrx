package org.mtrx.app.rooms

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import org.mtrx.app.model.Images
import org.mtrx.app.ui.Avatar
import org.mtrx.app.ui.CompactRow
import org.mtrx.app.ui.MtrxIcons
import org.mtrx.app.ui.BAR_TEXT_SCALE
import org.mtrx.app.ui.TextScaleAtMost
import org.mtrx.app.ui.linesHeight
import uniffi.mtrx.DirectoryRoom
import uniffi.mtrx.JoinAction
import uniffi.mtrx.JoinButton
import uniffi.mtrx.JoinRule
import uniffi.mtrx.Member
import uniffi.mtrx.Membership
import uniffi.mtrx.RoomLink
import uniffi.mtrx.RoomSettings
import uniffi.mtrx.Topic

/** The tabs of a space's overview. */
enum class OverviewTab(val label: String) { ROOMS("Rooms"), MEMBERS("Members"), SETTINGS("Settings") }

/** What the overview's Rooms tab shows. */
data class OverviewRooms(
    /** Null until first listed. */
    val rooms: List<DirectoryRoom>?,
    val updating: Boolean,
    val failed: Boolean,
    /** Rooms being joined or asked to join. */
    val pending: Set<String>,
    /** What a row offers, with our membership as the room lists have it (the library's joinButton). */
    val buttonOf: (DirectoryRoom) -> JoinButton,
)

/** What the Rooms tab's rows do. */
class OverviewRowActions(
    val join: (DirectoryRoom) -> Unit,
    val knock: (DirectoryRoom) -> Unit,
    val open: (DirectoryRoom) -> Unit,
    /** Show the invite, to accept or decline. */
    val answer: (DirectoryRoom) -> Unit,
    val retry: () -> Unit,
)

/**
 * A space's overview, full screen: its picture, name, Invite… and Leave…,
 * its topic with its calls and links as chips, and the tabs Rooms (its rooms
 * to join or open), Members and Settings (the topic to edit, then as Room
 * info).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SpaceOverviewContent(
    header: RoomInfoHeader,
    rooms: OverviewRooms,
    settings: RoomSettings?,
    members: List<Member>?,
    busy: String?,
    error: String?,
    ownUserId: String,
    spaceNames: Map<String, String>,
    images: Images,
    rowActions: OverviewRowActions,
    infoActions: RoomInfoActions,
    onBack: () -> Unit,
    onLeave: () -> Unit,
    onOpenLink: (String) -> Unit,
    initialTab: OverviewTab = OverviewTab.ROOMS,
    listState: LazyListState = rememberLazyListState(),
) {
    var tab by rememberSaveable { mutableStateOf(initialTab) }
    val pill = if (tab == OverviewTab.ROOMS) statusPill(rooms) else null
    // Taller at large text sizes.
    val pillHeight = maxOf(PILL_HEIGHT, linesHeight(MaterialTheme.typography.labelMedium) + 8.dp)
    var filter by rememberSaveable { mutableStateOf("") }
    val snackbar = remember { SnackbarHostState() }
    val openRoomLink = rememberRoomLinkOpener(onOpenLink, snackbar)
    LaunchedEffect(error) {
        val message = error ?: return@LaunchedEffect
        snackbar.showSnackbar(message)
        infoActions.clearError()
    }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(header.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        Box(
            Modifier
                .padding(padding)
                .fillMaxSize(),
        ) {
            // While the pill shows, room below the last row to scroll it clear.
            LazyColumn(
                Modifier.fillMaxSize(),
                state = listState,
                contentPadding = PaddingValues(bottom = if (pill != null) pillHeight + PILL_MARGIN * 2 else 0.dp),
            ) {
                item(key = "header") { InfoHeader(header, settings, images, busy, infoActions, onLeave) }
                if (header.topic.text.isNotBlank() || header.topic.hasLinks) {
                    item(key = "topic") { SpaceTopic(header.topic, openRoomLink) }
                }
                item(key = "tabs") {
                    // As the tabs: above the default text size, no tick, and
                    // the labels at most BAR_TEXT_SCALE, to fit.
                    val ticked = LocalDensity.current.fontScale <= 1f
                    TextScaleAtMost(BAR_TEXT_SCALE) {
                        SingleChoiceSegmentedButtonRow(
                            Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 16.dp, vertical = 8.dp),
                        ) {
                            OverviewTab.entries.forEachIndexed { index, it ->
                                SegmentedButton(
                                    selected = tab == it,
                                    onClick = { tab = it },
                                    shape = SegmentedButtonDefaults.itemShape(index, OverviewTab.entries.size),
                                    icon = { if (ticked) SegmentedButtonDefaults.Icon(tab == it) },
                                ) { Text(it.label, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                            }
                        }
                    }
                }
                when (tab) {
                    OverviewTab.ROOMS -> roomsTab(rooms, filter, { filter = it }, images, rowActions)
                    OverviewTab.MEMBERS -> membersSection(members, images, infoActions)
                    OverviewTab.SETTINGS -> {
                        item(key = "topic-box") {
                            TopicBox(header.topic, header.rights.changeTopic, infoActions::setTopic, openRoomLink)
                        }
                        if (settings == null) {
                            item(key = "loading") { Spinner() }
                        } else {
                            settingsSection(header, settings, members?.count { it.membership == Membership.JOINED }, spaceNames, ownUserId, infoActions)
                        }
                    }
                }
            }
            // Over the list's bottom right, which scrolls clear of it.
            if (pill != null) {
                StatusPill(
                    pill,
                    server = serverOf(ownUserId),
                    onRetry = rowActions.retry,
                    height = pillHeight,
                    modifier = Modifier
                        .align(Alignment.BottomEnd)
                        .padding(PILL_MARGIN),
                )
            }
        }
    }
}

/** What the Rooms tab's pill says: the list updates behind, or the server didn't answer. */
enum class OverviewPill { UPDATING, NO_ANSWER }

/** The pill while the rooms update behind a list shown, or after the server didn't answer; else none. */
fun statusPill(rooms: OverviewRooms): OverviewPill? = when {
    rooms.updating && rooms.rooms != null -> OverviewPill.UPDATING
    rooms.failed && !rooms.updating -> OverviewPill.NO_ANSWER
    else -> null
}

private val PILL_HEIGHT = 32.dp
private val PILL_MARGIN = 16.dp

/**
 * The status pill, as the desktop's: light blue while updating, light pink
 * when [server] didn't answer, with Try again.
 */
@Composable
private fun StatusPill(pill: OverviewPill, server: String, onRetry: () -> Unit, height: Dp, modifier: Modifier = Modifier) {
    val problem = pill == OverviewPill.NO_ANSWER
    Surface(
        modifier.height(height),
        shape = RoundedCornerShape(50),
        color = if (problem) MaterialTheme.colorScheme.errorContainer else MaterialTheme.colorScheme.primaryContainer,
        contentColor = if (problem) MaterialTheme.colorScheme.onErrorContainer else MaterialTheme.colorScheme.onPrimaryContainer,
        shadowElevation = 2.dp,
    ) {
        Row(
            Modifier.padding(start = 12.dp, end = if (problem) 4.dp else 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (problem) {
                Text("$server didn't answer", style = MaterialTheme.typography.labelMedium)
                TextButton(onClick = onRetry, contentPadding = PaddingValues(horizontal = 8.dp)) {
                    Text("Try again", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.error)
                }
            } else {
                CircularProgressIndicator(Modifier.size(12.dp), strokeWidth = 2.dp)
                Spacer(Modifier.width(8.dp))
                Text("Updating…", style = MaterialTheme.typography.labelMedium)
            }
        }
    }
}

/** A space's topic under its name: the text, then its calls and links as chips ([onOpen]); edited in Settings. */
@Composable
private fun SpaceTopic(topic: Topic, onOpen: (RoomLink) -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (topic.text.isNotBlank()) {
            Text(
                topic.text,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                maxLines = 4,
                overflow = TextOverflow.Ellipsis,
            )
        }
        LinkChips(topic, onOpen, Modifier.fillMaxWidth())
    }
}

/** The filter, the rows, and what is not listed. */
private fun LazyListScope.roomsTab(
    rooms: OverviewRooms,
    filter: String,
    onFilter: (String) -> Unit,
    images: Images,
    actions: OverviewRowActions,
) {
    item(key = "filter") {
        OutlinedTextField(
            value = filter,
            onValueChange = onFilter,
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 4.dp),
            singleLine = true,
            placeholder = { Text("Filter rooms") },
            leadingIcon = { Icon(Icons.Filled.Search, contentDescription = null) },
        )
    }
    val list = rooms.rooms
    if (list == null) {
        if (!rooms.failed) item(key = "loading") { Spinner() }
        return
    }
    val shown = filtered(list, filter)
    if (shown.isEmpty()) {
        item(key = "none") {
            Text(
                if (list.isEmpty()) "No rooms you can see." else "No rooms match.",
                Modifier.padding(16.dp),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
    items(shown, key = { "child:" + it.id }) { room ->
        DirectoryRow(room, rooms.buttonOf(room), room.id in rooms.pending, images, actions)
    }
    item(key = "invite-only") {
        Text(
            "Rooms only invited people can join aren't listed.",
            Modifier.padding(16.dp),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/**
 * A room to join or open: its picture or who may join, name, details and
 * our role; Join and Ask to join in the primary colour, Open and Invited,
 * or what stops us. A joined row opens on a tap.
 */
@Composable
fun DirectoryRow(room: DirectoryRoom, button: JoinButton, pending: Boolean, images: Images, actions: OverviewRowActions) {
    val action = button.action
    CompactRow(
        title = room.name,
        subtitle = rowDetails(room),
        onClick = if (action == JoinAction.OPEN && !pending) ({ actions.open(room) }) else null,
        leading = { RoomPicture(room, images) },
        trailing = {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                // Compact, but room for the label at large text sizes.
                val buttonHeight = maxOf(32.dp, linesHeight(MaterialTheme.typography.labelLarge) + 8.dp)
                roleTag(room)?.let {
                    Text(it, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                val label = button.title
                when {
                    pending -> CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                    action == JoinAction.JOIN -> Button(onClick = { actions.join(room) }, Modifier.height(buttonHeight), contentPadding = ROW_BUTTON) { Text(label) }
                    action == JoinAction.ASK_TO_JOIN -> Button(onClick = { actions.knock(room) }, Modifier.height(buttonHeight), contentPadding = ROW_BUTTON) { Text(label) }
                    action == JoinAction.OPEN -> OutlinedButton(onClick = { actions.open(room) }, Modifier.height(buttonHeight), contentPadding = ROW_BUTTON) { Text(label) }
                    action == JoinAction.INVITED -> OutlinedButton(onClick = { actions.answer(room) }, Modifier.height(buttonHeight), contentPadding = ROW_BUTTON) { Text(label) }
                    else -> Text(label, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        },
    )
}

/** Buttons in compact rows. */
private val ROW_BUTTON = PaddingValues(horizontal = 14.dp, vertical = 0.dp)

/** A room's picture, else an icon of who may join: # anyone, spaces' squares, a lock otherwise. */
@Composable
private fun RoomPicture(room: DirectoryRoom, images: Images) {
    val shape = if (room.isSpace) RoundedCornerShape(8.dp) else CircleShape
    if (room.avatarUrl != null) {
        Avatar(room.name, room.avatarUrl, room.id, images, size = 36.dp, shape = shape)
        return
    }
    Box(
        Modifier
            .size(36.dp)
            .clip(shape)
            .background(MaterialTheme.colorScheme.surfaceVariant),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            when {
                room.isSpace -> MtrxIcons.Spaces
                room.joinRule == JoinRule.PUBLIC -> MtrxIcons.Hash
                room.joinRule == JoinRule.SPACE_MEMBERS -> Icons.Filled.Person
                else -> Icons.Filled.Lock
            },
            contentDescription = null,
            modifier = Modifier.size(20.dp),
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun Spinner() {
    Box(
        Modifier
            .fillMaxWidth()
            .padding(24.dp),
        contentAlignment = Alignment.Center,
    ) { CircularProgressIndicator() }
}
