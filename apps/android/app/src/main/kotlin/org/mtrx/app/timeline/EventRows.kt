package org.mtrx.app.timeline

import android.graphics.Bitmap
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.foundation.layout.size
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animate
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
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
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
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
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import java.time.Duration
import kotlin.math.max
import kotlin.math.roundToInt
import kotlin.math.roundToLong
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
import uniffi.mtrx.playbackTime as libraryPlaybackTime

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
    /** A message's file was tapped: open it once downloaded (not audio, which plays in its bubble). */
    val openFile: (EventRow) -> Unit,
    val cancelDownload: (ULong) -> Unit,
    /** An audio message's button: play, pause, cancel or try again. */
    val toggleAudio: (Media) -> Unit,
    /** An audio message's scrubber, let go. */
    val seekAudio: (Media, Duration) -> Unit,
)

/** A message's bubble; an audio message plays as [playback] says, read only there. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun EventBubble(
    row: EventRow,
    images: Images,
    highlighted: Boolean,
    download: Download?,
    /** How much of its file is sent, while it uploads. */
    upload: Float?,
    handlers: RowHandlers,
    playback: () -> Playback,
) {
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
                                upload,
                                playback,
                                onOpenLink = handlers.openLink,
                                onShowPerson = handlers.showPerson,
                                onCancelDownload = handlers.cancelDownload,
                                onOpenFile = { handlers.openFile(row) },
                                onToggleAudio = handlers.toggleAudio,
                                onSeekAudio = handlers.seekAudio,
                            )
                            MessageTime(event, colors.secondary, Modifier.align(Alignment.End))
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

/** The message's time in [color], after a small hourglass if it disappears (its words for TalkBack). */
@Composable
private fun MessageTime(event: TimelineEvent, color: Color, modifier: Modifier) {
    Row(modifier, verticalAlignment = Alignment.CenterVertically) {
        event.disappear?.let {
            Icon(MtrxIcons.Hourglass, contentDescription = it.words, modifier = Modifier.size(12.dp), tint = color)
            Spacer(Modifier.width(2.dp))
        }
        Text(timeOfDay(event.timestamp), style = MaterialTheme.typography.labelSmall, color = color)
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
 * [onOpenLink], mentions with [onShowPerson]. Audio plays in it ([AudioRow]).
 */
@Composable
private fun MessageBody(
    event: TimelineEvent,
    edited: Boolean,
    images: Images,
    undecryptableText: String,
    colors: BubbleColors,
    download: Download?,
    upload: Float?,
    playback: () -> Playback,
    onOpenLink: (String) -> Unit,
    onShowPerson: (String, String) -> Unit,
    onCancelDownload: (ULong) -> Unit,
    onOpenFile: () -> Unit,
    onToggleAudio: (Media) -> Unit,
    onSeekAudio: (Media, Duration) -> Unit,
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
                        UploadRing(upload)
                    }
                    if (message.text.isNotEmpty()) text(Modifier.padding(top = 6.dp))
                }
                media != null -> Row(verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.weight(1f, fill = false)) {
                        if (media.kind == MediaKind.AUDIO) {
                            AudioRow(media, playback, colors, onToggleAudio, onSeekAudio)
                        } else {
                            FileRow(media, download, secondary, onCancelDownload, onOpenFile)
                        }
                    }
                    UploadRing(upload)
                }
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
 * A file that is not an image, video or audio: its name, and its size or
 * how its download goes, with a bar while it runs; cancel while it runs,
 * retry once it failed.
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

/**
 * The library's time as a player shows it ("0:07", "12:34", "1:02:03").
 * The screenshots, without the library, give their own.
 */
val LocalPlaybackTime = staticCompositionLocalOf<(Duration) -> String> { ::libraryPlaybackTime }

/** An audio message's play button, scrubber and time: at most this wide. */
private val AUDIO_WIDTH = 260.dp

/**
 * An audio message, played in its bubble as [playback] says ([AudioPlayer]):
 * a voice message's play button, waveform (a slider without one) and time;
 * other audio's name and size above the button, a slider and the time. The
 * time is the position once started, else the length (a voice message's
 * size if that is unknown). While the file is fetched a ring round the
 * button shows how far, and a tap cancels; if it can't play, the button
 * tries again and the reason shows under it. The scrubber moves the time
 * shown while dragged, and the position once let go ([onSeek]).
 */
@Composable
private fun AudioRow(
    media: Media,
    playback: () -> Playback,
    colors: BubbleColors,
    onToggle: (Media) -> Unit,
    onSeek: (Media, Duration) -> Unit,
) {
    val now = playback()
    val state = now.stateOf(media)
    val total = now.durationOf(media)
    // Where the scrubber is dragged to, until let go.
    var dragged by remember(media.source) { mutableStateOf<Duration?>(null) }
    val position = dragged ?: now.positionOf(media)
    val time = LocalPlaybackTime.current
    val shown = if (position > Duration.ZERO || state == PlayState.Playing) {
        time(position)
    } else {
        // Other audio shows its size above.
        media.durationLabel ?: fileSize(media).takeIf { media.voice && it.isNotEmpty() }
    }
    Column {
        if (!media.voice) {
            Text(media.filename, maxLines = 2, overflow = TextOverflow.Ellipsis)
            val size = fileSize(media)
            if (size.isNotEmpty()) Text(size, style = MaterialTheme.typography.bodySmall, color = colors.secondary)
        }
        Row(
            Modifier.widthIn(max = AUDIO_WIDTH),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            PlayButton(state, colors.link) { onToggle(media) }
            // In seconds, at least one so that the range is never empty; it moves once the length is known.
            val length = maxOf(total?.let(::secondsOf) ?: 0f, secondsOf(position), 1f)
            val at = secondsOf(position).coerceAtMost(length)
            val enabled = total != null
            if (media.voice && media.waveform.isNotEmpty()) {
                val spoken = total?.let { "${time(position)} of ${time(it)}" } ?: time(position)
                Waveform(
                    media.waveform,
                    played = at / length,
                    enabled = enabled,
                    accent = colors.link,
                    other = colors.secondary.copy(alpha = 0.4f),
                    onDrag = { dragged = timeAt(it * length) },
                    onRelease = {
                        dragged = null
                        onSeek(media, timeAt(it * length))
                    },
                    modifier = Modifier
                        .weight(1f)
                        .semantics {
                            contentDescription = "Voice message"
                            stateDescription = spoken
                            progressBarRangeInfo = ProgressBarRangeInfo(at, 0f..length)
                            if (enabled) {
                                setProgress { target ->
                                    onSeek(media, timeAt(target.coerceIn(0f, length)))
                                    true
                                }
                            }
                        },
                )
            } else {
                Slider(
                    value = at,
                    onValueChange = { dragged = timeAt(it) },
                    onValueChangeFinished = {
                        dragged?.let { onSeek(media, it) }
                        dragged = null
                    },
                    valueRange = 0f..length,
                    enabled = enabled,
                    colors = SliderDefaults.colors(
                        thumbColor = colors.link,
                        activeTrackColor = colors.link,
                        inactiveTrackColor = colors.secondary.copy(alpha = 0.3f),
                    ),
                    modifier = Modifier.weight(1f),
                )
            }
            if (shown != null) {
                Text(
                    shown,
                    style = MaterialTheme.typography.bodySmall.copy(fontFeatureSettings = "tnum"),
                    color = colors.secondary,
                    maxLines = 1,
                )
            }
        }
        (state as? PlayState.Failed)?.let {
            Text(it.message, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
        }
    }
}

/** Play, pause, cancel the fetch (a ring round it shows how far), or try again; in [accent]. */
@Composable
private fun PlayButton(state: PlayState?, accent: Color, onClick: () -> Unit) {
    Box(Modifier.size(48.dp), contentAlignment = Alignment.Center) {
        if (state is PlayState.Fetching) {
            val fraction = state.fraction
            val ring = Modifier.size(44.dp)
            if (fraction != null) {
                CircularProgressIndicator(progress = { fraction }, modifier = ring, color = accent)
            } else {
                CircularProgressIndicator(ring, color = accent)
            }
        }
        val (icon, label) = when (state) {
            PlayState.Playing -> MtrxIcons.Pause to "Pause"
            is PlayState.Fetching -> Icons.Filled.Close to "Cancel loading"
            is PlayState.Failed -> Icons.Filled.Refresh to "Try again"
            PlayState.Paused, null -> Icons.Filled.PlayArrow to "Play"
        }
        IconButton(onClick = onClick) {
            Icon(icon, contentDescription = label, tint = accent)
        }
    }
}

/**
 * A voice message's waveform ([Media.waveform]): its bars across the
 * width, those [played] (a fraction) in [accent], the others in [other].
 * While [enabled], a tap or a horizontal drag moves the position: [onDrag]
 * while it moves, [onRelease] where it ends, each as a fraction of the
 * length.
 */
@Composable
private fun Waveform(
    bars: List<Float>,
    played: Float,
    enabled: Boolean,
    accent: Color,
    other: Color,
    onDrag: (Float) -> Unit,
    onRelease: (Float) -> Unit,
    modifier: Modifier = Modifier,
) {
    val drag by rememberUpdatedState(onDrag)
    val release by rememberUpdatedState(onRelease)
    Canvas(
        modifier
            .height(28.dp)
            .pointerInput(enabled) {
                if (!enabled) return@pointerInput
                detectTapGestures { offset -> release(fraction(offset.x, size.width)) }
            }
            .pointerInput(enabled) {
                if (!enabled) return@pointerInput
                // Horizontal only, so that the list still scrolls.
                var at = 0f
                detectHorizontalDragGestures(
                    onDragStart = { offset ->
                        at = fraction(offset.x, size.width)
                        drag(at)
                    },
                    onDragEnd = { release(at) },
                    onDragCancel = { release(at) },
                    onHorizontalDrag = { change, _ ->
                        change.consume()
                        at = fraction(change.position.x, size.width)
                        drag(at)
                    },
                )
            },
    ) {
        val step = size.width / bars.size
        val barWidth = max(1.dp.toPx(), step * 0.6f)
        for ((index, bar) in bars.withIndex()) {
            val height = max(barWidth, size.height * bar)
            drawRoundRect(
                color = if ((index + 0.5f) / bars.size <= played) accent else other,
                topLeft = Offset(index * step + (step - barWidth) / 2, (size.height - height) / 2),
                size = Size(barWidth, height),
                cornerRadius = CornerRadius(barWidth / 2),
            )
        }
    }
}

/** Where [x] is across [width], from 0 to 1. */
private fun fraction(x: Float, width: Int): Float = if (width > 0) (x / width).coerceIn(0f, 1f) else 0f

private fun secondsOf(time: Duration): Float = time.toMillis() / 1000f

private fun timeAt(seconds: Float): Duration = Duration.ofMillis((seconds * 1000).roundToLong())

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
