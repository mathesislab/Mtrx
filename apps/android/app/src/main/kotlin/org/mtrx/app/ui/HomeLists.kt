package org.mtrx.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material3.HorizontalDivider
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import java.time.LocalDate
import org.mtrx.app.model.Images
import org.mtrx.app.model.saved
import org.mtrx.app.rooms.RoomGroup
import org.mtrx.app.rooms.name
import org.mtrx.app.rooms.unreadBadge
import uniffi.mtrx.ListSection
import uniffi.mtrx.Presence
import uniffi.mtrx.RoomKind
import uniffi.mtrx.RoomSummary
import uniffi.mtrx.SpaceEntry
import uniffi.mtrx.SpaceKey

// The lists of the home's tabs, in compact rows (#47): without a model, for
// the screens and their screenshot tests.

/**
 * The Chats tab: the Notes to self at the top where pinned there ([notes]),
 * then the direct chats, with the people's presence and the spaces shared
 * with them ([spaces], by room ID); a pencil on those with a draft
 * ([drafts], room IDs).
 */
@Composable
fun ChatList(
    notes: RoomSummary?,
    rooms: List<RoomSummary>,
    loaded: Boolean,
    images: Images,
    today: LocalDate,
    onOpenRoom: (RoomSummary) -> Unit,
    presenceOf: (RoomSummary) -> Presence?,
    spaces: Map<String, List<String>> = emptyMap(),
    drafts: Set<String> = emptySet(),
    state: LazyListState = rememberLazyListState(),
) {
    when {
        notes == null && rooms.isEmpty() && !loaded -> Loading()
        notes == null && rooms.isEmpty() -> Empty("No chats")
        else -> LazyColumn(Modifier.fillMaxSize(), state = state) {
            notesRow(notes, images, today, drafts, onOpenRoom)
            items(rooms, key = { it.id }) { room ->
                RoomRow(
                    room,
                    images,
                    today,
                    onClick = { onOpenRoom(room) },
                    presence = presenceOf(room),
                    spaces = spaces[room.id],
                    draft = room.id in drafts,
                )
            }
        }
    }
}

/** The Notes to self, pinned above the rest, with a line under it. */
private fun LazyListScope.notesRow(
    notes: RoomSummary?,
    images: Images,
    today: LocalDate,
    drafts: Set<String>,
    onOpenRoom: (RoomSummary) -> Unit,
) {
    if (notes == null) return
    item(key = "notes", contentType = "notes") {
        Column {
            RoomRow(notes, images, today, onClick = { onOpenRoom(notes) }, draft = notes.id in drafts)
            HorizontalDivider(Modifier.padding(horizontal = 16.dp))
        }
    }
}

/** The spaces page: [rows], [selected] highlighted, a dot where unread. */
@Composable
fun SpaceList(
    rows: List<SpaceEntry>,
    selected: SpaceKey?,
    loaded: Boolean,
    images: Images,
    onSelect: (SpaceKey) -> Unit,
    state: LazyListState = rememberLazyListState(),
) {
    when {
        rows.isEmpty() && !loaded -> Loading()
        rows.isEmpty() -> Empty("No spaces")
        else -> LazyColumn(Modifier.fillMaxSize(), state = state) {
            items(rows, key = { it.key.saved }) { row ->
                SpaceRowItem(row, row.key == selected, images, onClick = { onSelect(row.key) })
            }
        }
    }
}

/**
 * A space's rooms page: the Notes to self at the top where pinned there
 * ([notes]), then the [sections] shown, in order; a section's heading folds
 * it ([onFold] with its key), and a folded one shows its unread count. A
 * pencil on the rooms with a draft ([drafts]).
 */
@Composable
fun SpaceRoomsList(
    notes: RoomSummary?,
    sections: List<ListSection>,
    loaded: Boolean,
    images: Images,
    today: LocalDate,
    onOpenRoom: (RoomSummary) -> Unit,
    onFold: (String) -> Unit,
    drafts: Set<String> = emptySet(),
    state: LazyListState = rememberLazyListState(),
) {
    val empty = notes == null && sections.isEmpty()
    when {
        empty && !loaded -> Loading()
        empty -> Empty("No rooms")
        else -> LazyColumn(Modifier.fillMaxSize(), state = state) {
            notesRow(notes, images, today, drafts, onOpenRoom)
            for (section in sections) section(section, images, today, drafts, onOpenRoom, onFold)
        }
    }
}

/**
 * The Rooms tab's [groups] (and a direct chat's rooms shared with the
 * person): each under a heading, a space's opening it on the Spaces tab
 * ([onOpenSpace], its ID); with [onFold] (the group's key), a caret at the
 * left folds it, and only a folded one shows its unread count. A pencil on the
 * rooms with a draft ([drafts]). [empty] says there are none.
 */
@Composable
fun RoomGroupsList(
    groups: List<RoomGroup>,
    loaded: Boolean,
    images: Images,
    today: LocalDate,
    onOpenRoom: (RoomSummary) -> Unit,
    onOpenSpace: (String) -> Unit,
    onFold: ((String) -> Unit)? = null,
    drafts: Set<String> = emptySet(),
    empty: String = "No rooms",
    state: LazyListState = rememberLazyListState(),
) {
    when {
        groups.isEmpty() && !loaded -> Loading()
        groups.isEmpty() -> Empty(empty)
        else -> LazyColumn(Modifier.fillMaxSize(), state = state) {
            for (group in groups) {
                item(key = "group:${group.key}", contentType = "group") {
                    GroupHeading(
                        group,
                        images,
                        onOpen = group.space?.let { space -> { onOpenSpace(space.id) } },
                        onFold = onFold?.let { fold -> { fold(group.key) } },
                    )
                }
                if (group.folded) continue
                items(group.rooms, key = { "${group.key}/${it.id}" }, contentType = { "room" }) { room ->
                    RoomRow(room, images, today, onClick = { onOpenRoom(room) }, draft = room.id in drafts)
                }
            }
        }
    }
}

/**
 * A group's heading: with [onFold], a caret (down while unfolded) that
 * folds it; the space's small picture (a house for Home), its name, and
 * its unread count while folded (always if it does not fold). The name
 * opens the space ([onOpen], with a caret after it); Home's, without one,
 * folds it too.
 */
@Composable
private fun GroupHeading(group: RoomGroup, images: Images, onOpen: (() -> Unit)?, onFold: (() -> Unit)?) {
    val foldLabel = if (group.folded) "Unfold" else "Fold"
    // As tall as the name, at any text size.
    val iconSize = linesHeight(MaterialTheme.typography.titleSmall)
    Row(
        Modifier
            .fillMaxWidth()
            .padding(start = if (onFold != null) 0.dp else 16.dp, end = 16.dp, top = 12.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (onFold != null) {
            Box(
                Modifier
                    .clickable(onClickLabel = foldLabel, onClick = onFold)
                    .padding(start = 8.dp, end = 4.dp, top = 4.dp, bottom = 4.dp),
            ) {
                Icon(
                    if (group.folded) Icons.AutoMirrored.Filled.KeyboardArrowRight else Icons.Filled.KeyboardArrowDown,
                    contentDescription = foldLabel,
                    modifier = Modifier.size(20.dp),
                    tint = MaterialTheme.colorScheme.primary,
                )
            }
        }
        // The name at the left, the count at the right edge (as the rooms').
        val tap = when {
            onOpen != null -> Modifier.clickable(onClickLabel = "Open the space", onClick = onOpen)
            onFold != null -> Modifier.clickable(onClickLabel = foldLabel, onClick = onFold)
            else -> Modifier
        }
        Row(Modifier.weight(1f).then(tap), verticalAlignment = Alignment.CenterVertically) {
            val space = group.space
            if (space != null) {
                Avatar(space.name, space.avatarUrl, space.id, images, size = iconSize, shape = RoundedCornerShape(4.dp))
            } else {
                Icon(Icons.Filled.Home, contentDescription = null, modifier = Modifier.size(iconSize), tint = MaterialTheme.colorScheme.primary)
            }
            Spacer(Modifier.width(8.dp))
            Text(
                group.name,
                Modifier.weight(1f, fill = false),
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.primary,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (onOpen != null) {
                Icon(
                    Icons.AutoMirrored.Filled.KeyboardArrowRight,
                    contentDescription = null,
                    modifier = Modifier.size(iconSize),
                    tint = MaterialTheme.colorScheme.primary,
                )
            }
        }
        // Unfolded, its rooms show their own.
        if ((onFold == null || group.folded) && group.unread > 0) UnreadBadge(group.unread, group.mentions)
    }
}

private fun LazyListScope.section(
    section: ListSection,
    images: Images,
    today: LocalDate,
    drafts: Set<String>,
    onOpenRoom: (RoomSummary) -> Unit,
    onFold: (String) -> Unit,
) {
    item(key = "section:${section.key}", contentType = "section") {
        SectionHeading(section, onClick = { onFold(section.key) })
    }
    if (section.folded) return
    items(section.rooms, key = { "${section.key}/${it.id}" }, contentType = { "room" }) { room ->
        RoomRow(room, images, today, onClick = { onOpenRoom(room) }, draft = room.id in drafts)
    }
}

/** A section's heading: a caret (down while unfolded), its title, and while folded its unread count. */
@Composable
private fun SectionHeading(section: ListSection, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClickLabel = if (section.folded) "Unfold" else "Fold", onClick = onClick)
            .padding(start = 8.dp, end = 16.dp, top = 8.dp, bottom = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            if (section.folded) Icons.AutoMirrored.Filled.KeyboardArrowRight else Icons.Filled.KeyboardArrowDown,
            contentDescription = null,
            modifier = Modifier.size(20.dp),
            tint = MaterialTheme.colorScheme.primary,
        )
        Spacer(Modifier.width(4.dp))
        Text(
            section.title,
            Modifier.weight(1f),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        if (section.folded && section.unread > 0uL) UnreadBadge(section.unread.toLong(), section.mentions)
    }
}

@Composable
private fun SpaceRowItem(row: SpaceEntry, selected: Boolean, images: Images, onClick: () -> Unit) {
    val shape = RoundedCornerShape(8.dp)
    CompactRow(
        title = row.name,
        subtitle = null,
        onClick = onClick,
        bold = row.unread,
        modifier = if (selected) Modifier.background(MaterialTheme.colorScheme.secondaryContainer) else Modifier,
        leading = {
            val space = row.space
            if (space != null) {
                // Spaces are squares, as on the desktop.
                Avatar(space.name, space.avatarUrl, space.id, images, size = 36.dp, shape = shape)
            } else {
                Box(
                    Modifier
                        .size(36.dp)
                        .clip(shape)
                        .background(MaterialTheme.colorScheme.surfaceVariant),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(Icons.Filled.Home, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        },
        trailing = {
            if (row.unread) {
                Box(
                    Modifier
                        .size(10.dp)
                        .clip(CircleShape)
                        .background(MaterialTheme.colorScheme.primary)
                        .semantics { contentDescription = "Unread" },
                )
            }
        },
    )
}

/**
 * A room in a compact row: avatar (with [presence]'s dot), name, a grey
 * line (what the room is; for a direct chat the [spaces] shared with the
 * person, if any), its last activity, and below it a small grey pencil if
 * it has a [draft] and the unread count.
 */
@Composable
fun RoomRow(
    room: RoomSummary,
    images: Images,
    today: LocalDate,
    onClick: () -> Unit,
    presence: Presence? = null,
    spaces: List<String>? = null,
    draft: Boolean = false,
) {
    val badge = unreadBadge(room)
    val activity = room.lastActivity
    val line = when {
        spaces != null -> listOfNotNull("Invited".takeIf { room.invited }, spaces.joinToString(", ").ifEmpty { null })
            .joinToString(" · ")
            .ifEmpty { null }
        room.invited -> "Invited"
        room.kind == RoomKind.SPACE -> "Space"
        else -> room.topic?.lineSequence()?.firstOrNull { it.isNotBlank() } ?: room.alias
    }
    CompactRow(
        title = room.name,
        subtitle = line,
        onClick = onClick,
        bold = room.unreadMessages > 0uL,
        leading = {
            WithPresenceDot(presence, avatarSize = 36.dp, dotSize = 10.dp, ring = MaterialTheme.colorScheme.surface) {
                Avatar(
                    name = room.name,
                    mxcUri = room.avatarUrl,
                    seed = room.directUser?.userId ?: room.id,
                    images = images,
                    size = 36.dp,
                    // Spaces are squares, as on the desktop.
                    shape = if (room.kind == RoomKind.SPACE) RoundedCornerShape(8.dp) else CircleShape,
                )
            }
        },
        trailing = {
            Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(2.dp)) {
                if (activity != null) {
                    Text(
                        lastActivity(activity, today),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (draft || badge != null) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        if (draft) {
                            Icon(
                                Icons.Outlined.Edit,
                                contentDescription = "Draft",
                                modifier = Modifier.size(14.dp),
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (badge != null) UnreadBadge(badge.first, badge.second)
                    }
                }
            }
        },
    )
}

@Composable
fun Loading() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
}

@Composable
fun Empty(text: String) {
    Box(
        Modifier
            .fillMaxWidth()
            .fillMaxSize()
            .padding(24.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center)
    }
}
