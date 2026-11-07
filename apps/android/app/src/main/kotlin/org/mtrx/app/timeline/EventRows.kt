package org.mtrx.app.timeline

import android.graphics.Bitmap
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.foundation.layout.size
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animate
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import org.mtrx.app.model.Images
import org.mtrx.app.ui.Avatar
import org.mtrx.app.ui.BubbleColors
import org.mtrx.app.ui.LocalOwnBubble
import org.mtrx.app.ui.MtrxIcons
import org.mtrx.app.ui.identityColor
import org.mtrx.app.ui.timeOfDay
import uniffi.mtrx.Download
import uniffi.mtrx.DownloadState
import uniffi.mtrx.EventContent
import uniffi.mtrx.Media
import uniffi.mtrx.MediaKind
import uniffi.mtrx.MessageKind
import uniffi.mtrx.SendState
import uniffi.mtrx.TimelineEvent

/** What a message's row does when touched. */
internal class RowHandlers(
    val hold: (EventRow) -> Unit,
    val goTo: (String) -> Unit,
    val reply: (String) -> Unit,
    val react: (String, String) -> Unit,
    val showReactors: (String, String) -> Unit,
    val showReaders: (String) -> Unit,
    val openLink: (String) -> Unit,
    /** A mention was tapped: the person's sheet (user ID, name). */
    val showPerson: (String, String) -> Unit,
    /** The send state of a message not sent yet was tapped. */
    val unsent: (String) -> Unit,
    /** A message's file was tapped: open it once downloaded. */
    val openFile: (EventRow) -> Unit,
    val cancelDownload: (ULong) -> Unit,
)

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun EventBubble(row: EventRow, images: Images, highlighted: Boolean, download: Download?, handlers: RowHandlers) {
    val event = row.event
    val own = event.own
    val name = event.sender.displayName
    val mark by animateColorAsState(
        if (highlighted) MaterialTheme.colorScheme.primary.copy(alpha = 0.15f) else Color.Transparent,
        label = "highlight",
    )
    val bubbleShape = RoundedCornerShape(16.dp)
    val colors = if (own) {
        LocalOwnBubble.current
    } else {
        MaterialTheme.colorScheme.let { BubbleColors(it.surfaceVariant, it.onSurfaceVariant, it.onSurfaceVariant, it.primary) }
    }
    Column(
        Modifier
            .fillMaxWidth()
            .background(mark, RoundedCornerShape(8.dp))
            .padding(top = if (row.showSender) 8.dp else 2.dp),
    ) {
        if (row.showSender) {
            Text(
                name,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.SemiBold,
                color = identityColor(event.sender.userId),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.padding(start = AVATAR + AVATAR_GAP + 4.dp, end = 48.dp, bottom = 2.dp),
            )
        }
        SwipeToReply(enabled = row.canReply, onReply = { handlers.reply(row.id) }) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(start = if (own) 48.dp else 0.dp, end = if (own) 0.dp else 48.dp),
                horizontalArrangement = if (own) Arrangement.End else Arrangement.Start,
            ) {
                // Beside the bubble, at the first of a group.
                if (row.avatarColumn) {
                    if (row.showSender) {
                        Avatar(name, event.sender.avatarUrl, seed = event.sender.userId, images = images, size = AVATAR)
                    } else {
                        Spacer(Modifier.width(AVATAR))
                    }
                    Spacer(Modifier.width(AVATAR_GAP))
                }
                Column(
                    Modifier.weight(1f, fill = false),
                    horizontalAlignment = if (own) Alignment.End else Alignment.Start,
                ) {
                    Surface(
                        shape = bubbleShape,
                        color = colors.container,
                        contentColor = colors.content,
                        modifier = Modifier
                            .clip(bubbleShape)
                            .combinedClickable(
                                onClick = { handlers.openFile(row) },
                                onLongClick = { handlers.hold(row) },
                            ),
                    ) {
                        Column(Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
                            row.quote?.let { QuoteBlock(it, colors.secondary, handlers.goTo) }
                            MessageBody(
                                event,
                                row.edited,
                                images,
                                row.undecryptableText,
                                colors,
                                download,
                                onOpenLink = handlers.openLink,
                                onShowPerson = handlers.showPerson,
                                onCancelDownload = handlers.cancelDownload,
                                onOpenFile = { handlers.openFile(row) },
                            )
                            Text(
                                timeOfDay(event.timestamp),
                                style = MaterialTheme.typography.labelSmall,
                                color = colors.secondary,
                                modifier = Modifier.align(Alignment.End),
                            )
                        }
                    }
                    ReactionChips(row, handlers)
                    SendStateLine(event.sendState, onClick = { handlers.unsent(row.id) })
                }
            }
        }
        if (row.readers.all.isNotEmpty()) {
            ReadersLine(
                row.readers,
                images,
                onClick = { handlers.showReaders(row.id) },
                modifier = Modifier.align(Alignment.End),
            )
        }
    }
}

private val AVATAR = 32.dp
private val AVATAR_GAP = 8.dp

/** How far a message is swiped left to reply, and the most it moves. */
private val SWIPE_REPLY = 64.dp
private val SWIPE_MAX = 96.dp

/**
 * [content] that, while [enabled], swiped left past [SWIPE_REPLY] calls
 * [onReply] when let go, the reply icon showing at the right as it moves;
 * it springs back. Horizontal drags only, so vertical ones scroll the list;
 * and swipes from the screen's edge stay the system's back gesture.
 */
@Composable
private fun SwipeToReply(enabled: Boolean, onReply: () -> Unit, content: @Composable () -> Unit) {
    val density = LocalDensity.current
    val threshold = with(density) { SWIPE_REPLY.toPx() }
    val limit = with(density) { SWIPE_MAX.toPx() }
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val reply by rememberUpdatedState(onReply)
    var dragX by remember { mutableFloatStateOf(0f) }
    var settling by remember { mutableStateOf<Job?>(null) }
    val drag = rememberDraggableState { delta ->
        val before = dragX
        dragX = (dragX + delta).coerceIn(-limit, 0f)
        // Felt once the reply is armed.
        if (before > -threshold && dragX <= -threshold) haptics.performHapticFeedback(HapticFeedbackType.LongPress)
    }
    Box(
        Modifier
            .fillMaxWidth()
            .draggable(
                drag,
                Orientation.Horizontal,
                enabled = enabled,
                onDragStarted = { settling?.cancel() },
                onDragStopped = {
                    if (dragX <= -threshold) reply()
                    settling = scope.launch { animate(dragX, 0f) { value, _ -> dragX = value } }
                },
            ),
    ) {
        Icon(
            MtrxIcons.Reply,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.primary,
            modifier = Modifier
                .align(Alignment.CenterEnd)
                .padding(end = 8.dp)
                .graphicsLayer {
                    val progress = (-dragX / threshold).coerceIn(0f, 1f)
                    alpha = progress
                    scaleX = 0.6f + 0.4f * progress
                    scaleY = scaleX
                },
        )
        Box(Modifier.offset { IntOffset(dragX.roundToInt(), 0) }) { content() }
    }
}

/** The message a reply answers, its text in [secondary]; a loaded one goes to it. */
@Composable
private fun QuoteBlock(quote: Quote, secondary: Color, onGoTo: (String) -> Unit) {
    Row(
        Modifier
            .padding(bottom = 6.dp)
            .height(IntrinsicSize.Min)
            .clip(RoundedCornerShape(4.dp))
            .then(if (quote.tappable) Modifier.clickable { onGoTo(quote.eventId) } else Modifier),
    ) {
        Box(
            Modifier
                .width(3.dp)
                .fillMaxHeight()
                .background(MaterialTheme.colorScheme.primary),
        )
        Column(Modifier.padding(start = 8.dp, end = 4.dp)) {
            quote.sender?.let {
                Text(it, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Text(
                quote.text,
                style = MaterialTheme.typography.bodyMedium,
                color = secondary,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

/** Reactions under the bubble, ours in the accent colour: tap adds or removes ours, hold lists who. */
@OptIn(ExperimentalLayoutApi::class, ExperimentalFoundationApi::class)
@Composable
private fun ReactionChips(row: EventRow, handlers: RowHandlers) {
    val reactions = row.reactions
    if (reactions.isEmpty()) return
    FlowRow(
        Modifier.padding(top = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        for (reaction in reactions) {
            val shape = RoundedCornerShape(12.dp)
            val ours = reaction.byUs
            Surface(
                shape = shape,
                color = if (ours) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceVariant,
                border = if (ours) BorderStroke(1.dp, MaterialTheme.colorScheme.primary) else null,
                modifier = Modifier
                    .clip(shape)
                    .combinedClickable(
                        onClick = { handlers.react(row.id, reaction.key) },
                        onLongClick = { handlers.showReactors(row.id, reaction.key) },
                    ),
            ) {
                Text(
                    "${reaction.key} ${reaction.count}",
                    style = MaterialTheme.typography.labelLarge,
                    color = if (ours) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                )
            }
        }
    }
}

/** Who read up to here, at the row's right margin: tap lists them. */
@Composable
private fun ReadersLine(readers: Readers, images: Images, onClick: () -> Unit, modifier: Modifier = Modifier) {
    Row(
        modifier
            .padding(top = 2.dp)
            .clip(RoundedCornerShape(8.dp))
            .clickable(onClick = onClick)
            .padding(horizontal = 4.dp, vertical = 2.dp),
        horizontalArrangement = Arrangement.spacedBy(2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        for (reader in readers.shown) {
            Avatar(reader.displayName, reader.avatarUrl, seed = reader.userId, images = images, size = 16.dp)
        }
        if (readers.more > 0) {
            Text(
                "+${readers.more}",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(start = 2.dp),
            )
        }
    }
}

/**
 * A message's content; notices, deleted and undecryptable messages in the
 * bubble's secondary colour. Its text is [MessageText]: links open with
 * [onOpenLink], mentions with [onShowPerson].
 */
@Composable
private fun MessageBody(
    event: TimelineEvent,
    edited: Boolean,
    images: Images,
    undecryptableText: String,
    colors: BubbleColors,
    download: Download?,
    onOpenLink: (String) -> Unit,
    onShowPerson: (String, String) -> Unit,
    onCancelDownload: (ULong) -> Unit,
    onOpenFile: () -> Unit,
) {
    val secondary = colors.secondary
    when (val content = event.content) {
        is EventContent.Message -> {
            val message = content.message
            val media = message.media
            val look = TextLook(
                text = if (message.kind == MessageKind.NOTICE) secondary else colors.content,
                link = colors.link,
                muted = secondary,
            )
            val text = @Composable { modifier: Modifier ->
                MessageText(message.text, edited, look, images, onOpenLink, onShowPerson, modifier)
            }
            when {
                media != null && (media.kind == MediaKind.IMAGE || media.kind == MediaKind.VIDEO) -> {
                    Box(contentAlignment = Alignment.Center) {
                        MediaPreview(media, images)
                        DownloadProgress(download, onCancelDownload, onOpenFile)
                    }
                    if (message.text.isNotEmpty()) text(Modifier.padding(top = 6.dp))
                }
                media != null -> FileRow(media, download, secondary, onCancelDownload, onOpenFile)
                else -> text(Modifier)
            }
        }
        EventContent.Redacted -> Text("Message deleted", color = secondary, fontStyle = FontStyle.Italic)
        // Turns into the message once its key arrives (recovery, verification, the backup).
        EventContent.UnableToDecrypt -> Text(
            undecryptableText,
            color = secondary,
            fontStyle = FontStyle.Italic,
        )
        is EventContent.Other -> Text("sent a ${content.description}", color = secondary)
        // State events are StateRows.
        is EventContent.Membership, is EventContent.DisplayNameChange, is EventContent.OtherState -> {}
    }
}

/** An image's (or video's) preview, laid out at its size before it loads. */
@Composable
private fun MediaPreview(media: Media, images: Images) {
    val width = media.width?.toLong()
    val height = media.height?.toLong()
    val ratio = if (width != null && height != null && width > 0 && height > 0) {
        (width.toFloat() / height.toFloat()).coerceIn(0.5f, 2.5f)
    } else {
        4f / 3f
    }
    val bitmap: Bitmap? by produceState(images.cachedPreview(media), media.source) {
        value = images.cachedPreview(media) ?: images.preview(media)
    }
    Box(
        Modifier
            .width(240.dp)
            .aspectRatio(ratio)
            .clip(RoundedCornerShape(12.dp))
            .background(MaterialTheme.colorScheme.surface),
    ) {
        val loaded = bitmap
        if (loaded != null) {
            val image = remember(loaded) { loaded.asImageBitmap() }
            Image(image, contentDescription = media.filename, modifier = Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        }
    }
}

/**
 * A file that is not an image or video: its name, and its size or how its
 * download goes, with a bar while it runs; cancel while it runs, retry once
 * it failed.
 */
@Composable
private fun FileRow(media: Media, download: Download?, secondary: Color, onCancel: (ULong) -> Unit, onRetry: () -> Unit) {
    val running = download != null && download.state is DownloadState.Downloading
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Column(Modifier.weight(1f, fill = false)) {
            Text(media.filename, maxLines = 2, overflow = TextOverflow.Ellipsis)
            // The library's line while it runs or after it failed; else the size.
            val line = if (download != null && (running || download.stopped)) download.status else fileSize(media)
            if (line.isNotEmpty()) Text(line, style = MaterialTheme.typography.bodySmall, color = secondary)
            if (download != null && running) {
                val fraction = download.fraction
                val bar = Modifier.padding(top = 4.dp).width(160.dp)
                if (fraction != null) LinearProgressIndicator(progress = { fraction }, modifier = bar) else LinearProgressIndicator(bar)
            }
        }
        if (download != null && running) {
            IconButton(onClick = { onCancel(download.id) }) {
                Icon(Icons.Filled.Close, contentDescription = "Cancel the download")
            }
        } else if (download != null && download.stopped) {
            IconButton(onClick = onRetry) {
                Icon(Icons.Filled.Refresh, contentDescription = "Try again")
            }
        }
    }
}

/** Over an image or video: a ring while it downloads (tap to cancel), retry once it failed. */
@Composable
private fun DownloadProgress(download: Download?, onCancel: (ULong) -> Unit, onRetry: () -> Unit) {
    if (download == null) return
    val background = MaterialTheme.colorScheme.surface.copy(alpha = 0.8f)
    when {
        download.state is DownloadState.Downloading -> Box(
            Modifier
                .size(44.dp)
                .clip(CircleShape)
                .background(background)
                .clickable(onClickLabel = "Cancel the download") { onCancel(download.id) },
            contentAlignment = Alignment.Center,
        ) {
            val fraction = download.fraction
            if (fraction != null) CircularProgressIndicator(progress = { fraction }) else CircularProgressIndicator()
            Icon(Icons.Filled.Close, contentDescription = null, modifier = Modifier.size(16.dp))
        }
        download.stopped -> Box(
            Modifier
                .size(44.dp)
                .clip(CircleShape)
                .background(background)
                .clickable(onClickLabel = "Try again", onClick = onRetry),
            contentAlignment = Alignment.Center,
        ) {
            Icon(Icons.Filled.Refresh, contentDescription = "Couldn't download")
        }
    }
}

/** A file's size, e.g. "1.2 MB"; empty if unknown. */
private fun fileSize(media: Media): String {
    val bytes = media.size?.toLong() ?: return ""
    return when {
        bytes >= 1_000_000 -> "%.1f MB".format(bytes / 1_000_000.0)
        bytes >= 1_000 -> "${bytes / 1_000} kB"
        else -> "$bytes B"
    }
}

/** A message not sent yet says so; a tap offers to retry it (if refused) or take it back. */
@Composable
private fun SendStateLine(state: SendState, onClick: () -> Unit) {
    val (text, error) = when (state) {
        SendState.Sent -> return
        SendState.Sending -> "sending…" to false
        SendState.Waiting -> "waiting for connection…" to false
        is SendState.Failed -> "not sent: ${state.error}" to true
    }
    Text(
        text,
        style = MaterialTheme.typography.labelSmall,
        color = if (error) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier
            .clickable(onClickLabel = "Retry or cancel", onClick = onClick)
            .padding(horizontal = 4.dp, vertical = 2.dp),
    )
}

@Composable
internal fun Notice(text: String) {
    Text(
        text,
        Modifier
            .fillMaxWidth()
            .padding(vertical = 6.dp),
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        textAlign = TextAlign.Center,
    )
}

@Composable
internal fun DayDivider(label: String) {
    Row(Modifier.padding(vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        HorizontalDivider(Modifier.weight(1f))
        Text(
            label,
            Modifier.padding(horizontal = 12.dp),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        HorizontalDivider(Modifier.weight(1f))
    }
}

@Composable
internal fun ReadMarker() {
    Row(Modifier.padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        HorizontalDivider(Modifier.weight(1f), color = MaterialTheme.colorScheme.primary)
        Text(
            "New",
            Modifier.padding(horizontal = 8.dp),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.primary,
        )
    }
}
