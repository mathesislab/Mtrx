package org.mtrx.app.timeline

import android.graphics.Bitmap
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.ui.platform.LocalContext
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.BadgedBox
import kotlinx.coroutines.flow.Flow
import org.mtrx.app.rooms.topicPreview
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animate
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
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
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import java.time.LocalDate
import kotlin.math.roundToInt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import org.mtrx.app.model.Images
import org.mtrx.app.rooms.RoomGroup
import org.mtrx.app.rooms.roomGroups
import org.mtrx.app.ui.Avatar
import org.mtrx.app.ui.BAR_TEXT_SCALE
import org.mtrx.app.ui.TextScaleAtMost
import org.mtrx.app.ui.BubbleColors
import org.mtrx.app.ui.LocalOwnBubble
import org.mtrx.app.ui.MtrxIcons
import org.mtrx.app.ui.UnreadBadge
import org.mtrx.app.ui.PresenceLine
import org.mtrx.app.ui.WithPresenceDot
import org.mtrx.app.ui.dayLabel
import org.mtrx.app.ui.identityColor
import org.mtrx.app.ui.linesHeight
import org.mtrx.app.ui.timeOfDay
import uniffi.mtrx.Download
import uniffi.mtrx.DownloadState
import uniffi.mtrx.EventContent
import uniffi.mtrx.Media
import uniffi.mtrx.MediaKind
import uniffi.mtrx.MessageAction
import uniffi.mtrx.MessageKind
import uniffi.mtrx.Presence
import uniffi.mtrx.RoomListsInterface
import uniffi.mtrx.RoomSummary
import uniffi.mtrx.SendState
import uniffi.mtrx.TimelineEvent

/** Rows from the top this close load older events. */
private const val LOAD_MARGIN = 5

/** What shows over the timeline, one at a time; rows by id, as in the latest rows. */
private sealed interface Overlay {
    /** A message's menu; with [unsent], only retrying it or taking it back (its send state was tapped). */
    data class Menu(val rowId: String, val unsent: Boolean = false) : Overlay
    data class EmojiPicker(val rowId: String) : Overlay
    data class Reactors(val rowId: String, val key: String) : Overlay
    data class Readers(val rowId: String) : Overlay
    data class SelectText(val text: String) : Overlay
    data class ConfirmDelete(val rowId: String) : Overlay
}

/** What a message's row does when touched. */
private class RowHandlers(
    val hold: (EventRow) -> Unit,
    val goTo: (String) -> Unit,
    val reply: (String) -> Unit,
    val react: (String, String) -> Unit,
    val showReactors: (String, String) -> Unit,
    val showReaders: (String) -> Unit,
    val openLink: (String) -> Unit,
    /** The send state of a message not sent yet was tapped. */
    val unsent: (String) -> Unit,
    /** A message's file was tapped: open it once downloaded. */
    val openFile: (EventRow) -> Unit,
    val cancelDownload: (ULong) -> Unit,
)

/** What the room screen shows, from its model's flows. */
data class TimelineState(
    val rows: List<TimelineRow>,
    val draft: String = "",
    val composing: Composing? = null,
    val typing: List<String> = emptyList(),
    val seeking: Boolean = false,
    val highlighted: String? = null,
    val error: String? = null,
    val canEditTopic: Boolean = false,
    /** The pinned messages while the topic panel shows; null while loading. */
    val pinned: List<EventRow>? = null,
    /** A direct chat's rooms shared with the person, by space, while their panel shows; null while loading. */
    val shared: List<RoomGroup>? = null,
    /** Each message's file's newest download, by its source. */
    val downloads: Map<String, Download> = emptyMap(),
)

/**
 * A room's timeline. [spaces], the top-level ones we may invite to, to
 * invite a direct chat's person to; [presences], for the header
 * and the person sheet, and [refreshPresence] when the sheet opens.
 * [backCount] on the back button: unread messages in the other rooms and
 * direct chats.
 * The room's name opens [onRoomInfo] (a direct chat's, its person); links of
 * the topic open with [onOpenLink]. A direct chat's rooms shared with the
 * person are grouped as [rooms] lists them; one opens with [onOpenRoom], a
 * space with [onOpenSpace] (on the Spaces tab).
 */
@Composable
fun TimelineScreen(
    header: RoomHeader,
    viewModel: TimelineViewModel,
    images: Images,
    spaces: List<RoomSummary>,
    rooms: RoomListsInterface,
    presences: Map<String, Presence>,
    refreshPresence: suspend (String) -> Unit,
    backCount: Long,
    onBack: () -> Unit,
    onRoomInfo: () -> Unit,
    onOpenLink: (String) -> Unit,
    onOpenRoom: (String) -> Unit,
    onOpenSpace: (String) -> Unit,
) {
    val rows by viewModel.rows.collectAsStateWithLifecycle()
    // At once, not at the next frame: the field compares its text with it.
    val draft by viewModel.draft.collectAsStateWithLifecycle(context = Dispatchers.Main.immediate)
    val composing by viewModel.composing.collectAsStateWithLifecycle(context = Dispatchers.Main.immediate)
    val typing by viewModel.typing.collectAsStateWithLifecycle()
    val seeking by viewModel.seeking.collectAsStateWithLifecycle()
    val highlighted by viewModel.highlighted.collectAsStateWithLifecycle()
    val error by viewModel.error.collectAsStateWithLifecycle()
    val personInfo by viewModel.personInfo.collectAsStateWithLifecycle()
    val pinned by viewModel.pinned.collectAsStateWithLifecycle()
    val sharedIds by viewModel.sharedRooms.collectAsStateWithLifecycle()
    val downloads by viewModel.downloads.collectAsStateWithLifecycle()
    val shared = remember(sharedIds, rooms) { sharedIds?.let { roomGroups(rooms, among = it) } }
    // The app may be ended in the background: the draft is kept first.
    LifecycleEventEffect(Lifecycle.Event.ON_STOP) { viewModel.keepDraft() }
    val state = TimelineState(
        rows = rows,
        draft = draft,
        composing = composing,
        typing = typing,
        seeking = seeking,
        highlighted = highlighted,
        error = error,
        canEditTopic = header.rights.changeTopic,
        pinned = pinned,
        shared = shared,
        downloads = downloads,
    )
    TimelineContent(
        header,
        state,
        viewModel,
        images,
        presence = header.person?.let { presences[it] },
        backCount = backCount,
        scrollTo = viewModel.scrollTo,
        files = viewModel.files,
        onBack = onBack,
        onRoomInfo = onRoomInfo,
        onOpenLink = onOpenLink,
        onOpenRoom = onOpenRoom,
        onOpenSpace = onOpenSpace,
    )

    personInfo?.let { model ->
        // Fresher than the subscription's, and for someone not in it.
        LaunchedEffect(model) { refreshPresence(model.userId) }
        PersonInfoSheet(model, header, presences[model.userId], spaces, images, onDismiss = viewModel::closePersonInfo)
    }
}

/**
 * The room screen as [state] says, without a model: the header with its
 * topic line and panel ([topicOpen] or [editingTopic] at first) and its
 * space, or a direct chat's # and the panel of the rooms shared with the
 * person ([sharedOpen] at first); the timeline, the composer and the menus
 * of messages.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TimelineContent(
    header: RoomHeader,
    state: TimelineState,
    actions: TimelineActions,
    images: Images,
    presence: Presence?,
    backCount: Long,
    scrollTo: Flow<String>,
    onBack: () -> Unit,
    onRoomInfo: () -> Unit,
    onOpenLink: (String) -> Unit,
    onOpenRoom: (String) -> Unit,
    onOpenSpace: (String) -> Unit,
    files: Flow<FileRequest> = emptyFlow(),
    topicOpen: Boolean = false,
    editingTopic: Boolean = false,
    sharedOpen: Boolean = false,
) {
    val rows = state.rows
    val listState = rememberLazyListState()
    val scope = rememberCoroutineScope()
    val snackbar = remember { SnackbarHostState() }
    val haptics = LocalHapticFeedback.current
    @Suppress("DEPRECATION") // LocalClipboard's API is suspending; this is a plain copy.
    val clipboard = LocalClipboardManager.current
    var overlay by remember { mutableStateOf<Overlay?>(null) }
    var panel by remember {
        mutableStateOf(
            when {
                editingTopic -> TopicPanelMode.EDITING
                topicOpen -> TopicPanelMode.SHOWING
                else -> null
            },
        )
    }
    // Newest first: the list is reversed, so it starts at the bottom and
    // stays there as messages arrive.
    val newestFirst = remember(rows) { rows.asReversed() }
    val currentRows by rememberUpdatedState(newestFirst)
    val today = remember(rows) { LocalDate.now() }

    LaunchedEffect(state.error) {
        val message = state.error ?: return@LaunchedEffect
        snackbar.showSnackbar(message)
        actions.clearError()
    }

    // Downloaded files: opened, shared, or saved to Downloads (on API 26–28
    // where the user picks).
    val context = LocalContext.current
    var saving by remember { mutableStateOf<FileRequest?>(null) }
    val createDocument = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("*/*")) { target ->
        val request = saving ?: return@rememberLauncherForActivityResult
        saving = null
        if (target != null) {
            scope.launch {
                snackbar.showSnackbar(if (copyFile(context, request, target)) "Saved" else "Couldn't save the file")
            }
        }
    }
    LaunchedEffect(files) {
        files.collect { request ->
            when (request.use) {
                FileUse.OPEN -> if (!openFile(context, request)) {
                    scope.launch { snackbar.showSnackbar("No app can open this file") }
                }
                FileUse.SHARE -> shareFile(context, request)
                FileUse.SAVE -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    scope.launch {
                        val saved = saveToDownloads(context, request)
                        snackbar.showSnackbar(if (saved) "Saved to Downloads" else "Couldn't save the file")
                    }
                } else {
                    saving = request
                    createDocument.launch(request.media.filename)
                }
            }
        }
    }

    // The pinned messages, while the panel is open.
    val panelOpen = panel != null
    LaunchedEffect(panelOpen) {
        if (panelOpen) actions.showPinned() else actions.hidePinned()
    }
    BackHandler(enabled = panelOpen) { panel = null }

    // A direct chat's rooms shared with the person, over the timeline and
    // the composer (the keyboard goes); loaded as the panel opens.
    var sharing by remember { mutableStateOf(sharedOpen) }
    val keyboard = LocalSoftwareKeyboardController.current
    val focus = LocalFocusManager.current
    val person = header.person
    LaunchedEffect(sharing, person) {
        if (sharing && person != null) {
            focus.clearFocus()
            keyboard?.hide()
            actions.showSharedRooms(person)
        }
    }
    BackHandler(enabled = sharing) { sharing = false }

    // Older events near the top (the reversed list's end).
    LaunchedEffect(listState) {
        snapshotFlow {
            val info = listState.layoutInfo
            (info.visibleItemsInfo.lastOrNull()?.index ?: -1) >= info.totalItemsCount - LOAD_MARGIN
        }.collect { actions.onNearTop(it) }
    }

    // A reply's original (or a pinned message), once its row is on the list.
    LaunchedEffect(scrollTo) {
        scrollTo.collectLatest { id ->
            val list = snapshotFlow { currentRows }.first { list -> list.any { it.id == id } }
            listState.animateScrollToItem(list.indexOfFirst { it.id == id })
        }
    }

    // Follow new messages while at the bottom.
    val newestId = newestFirst.firstOrNull()?.id
    LaunchedEffect(newestId) {
        if (listState.firstVisibleItemIndex <= 1) listState.animateScrollToItem(0)
    }

    // Read while the newest message shows and the screen is in front.
    val lifecycleState by LocalLifecycleOwner.current.lifecycle.currentStateFlow.collectAsState()
    val atBottom by remember { derivedStateOf { listState.firstVisibleItemIndex == 0 } }
    LaunchedEffect(atBottom, newestId, lifecycleState) {
        if (atBottom && lifecycleState.isAtLeast(Lifecycle.State.RESUMED)) actions.markRead()
    }

    val openLink by rememberUpdatedState(onOpenLink)
    val handlers = remember(actions) {
        RowHandlers(
            hold = { row ->
                if (row.actions.isNotEmpty()) {
                    haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                    overlay = Overlay.Menu(row.id)
                }
            },
            goTo = actions::goToOriginal,
            reply = actions::startReply,
            react = actions::react,
            showReactors = { rowId, key -> overlay = Overlay.Reactors(rowId, key) },
            showReaders = { rowId -> overlay = Overlay.Readers(rowId) },
            openLink = { url -> openLink(url) },
            unsent = { rowId -> overlay = Overlay.Menu(rowId, unsent = true) },
            openFile = { row ->
                row.message?.media?.let { actions.file(it, row.event.eventId, FileUse.OPEN) }
            },
            cancelDownload = actions::cancelDownload,
        )
    }

    Scaffold(
        topBar = {
            Column {
                TopAppBar(
                    // Two lines, the name and the topic (or presence), at any text size.
                    expandedHeight = maxOf(
                        TopAppBarDefaults.TopAppBarExpandedHeight,
                        linesHeight(MaterialTheme.typography.titleMedium, MaterialTheme.typography.bodySmall) + 16.dp,
                    ),
                    title = {
                        RoomTitle(
                            header,
                            presence,
                            images,
                            topicLine = topicPreview(header.topic, state.canEditTopic),
                            topicOpen = panelOpen,
                            onTitle = { if (person != null) actions.showPersonInfo(person) else onRoomInfo() },
                            onTopic = { panel = if (panelOpen) null else TopicPanelMode.SHOWING },
                        )
                    },
                    navigationIcon = {
                        IconButton(onClick = onBack) {
                            BadgedBox(
                                badge = {
                                    if (backCount > 0) TextScaleAtMost(BAR_TEXT_SCALE) { UnreadBadge(backCount) }
                                },
                            ) {
                                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                            }
                        }
                    },
                    actions = {
                        val space = header.space
                        when {
                            person != null -> SharedRoomsButton(header.name, open = sharing, onClick = { sharing = !sharing })
                            space != null -> SpaceButton(space, images, onClick = { onOpenSpace(space.id) })
                        }
                    },
                )
                if (state.seeking) LinearProgressIndicator(Modifier.fillMaxWidth())
            }
        },
        bottomBar = {
            if (!sharing) Column {
                TypingLine(typingText(state.typing))
                Composer(
                    draft = state.draft,
                    composing = state.composing,
                    onDraft = actions::onDraft,
                    onCancel = actions::cancelComposing,
                    onSend = {
                        if (actions.send()) scope.launch { listState.scrollToItem(0) }
                    },
                )
            }
        },
        snackbarHost = { SnackbarHost(snackbar) },
        // Back to the newest message, while away from it.
        floatingActionButton = {
            AnimatedVisibility(
                visible = !atBottom && !sharing,
                enter = fadeIn() + scaleIn(),
                exit = fadeOut() + scaleOut(),
            ) {
                SmallFloatingActionButton(
                    onClick = { scope.launch { listState.animateScrollToItem(0) } },
                    shape = CircleShape,
                ) {
                    Icon(Icons.Filled.KeyboardArrowDown, contentDescription = "Latest messages")
                }
            }
        },
    ) { padding ->
        Box(
            Modifier
                .padding(padding)
                .fillMaxSize(),
        ) {
            LazyColumn(
                Modifier.fillMaxSize(),
                state = listState,
                reverseLayout = true,
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
            ) {
                items(newestFirst, key = { it.id }, contentType = { it::class }) { row ->
                    when (row) {
                        is EventRow -> EventBubble(
                            row,
                            images,
                            state.highlighted == row.id,
                            row.message?.media?.let { state.downloads[it.source] },
                            handlers,
                        )
                        is StateRow -> Notice(row.text)
                        is DayRow -> DayDivider(dayLabel(row.date, today))
                        is ReadMarkerRow -> ReadMarker()
                        is StartRow -> Notice("This is the beginning of the room")
                    }
                }
            }
            panel?.let { mode ->
                TopicPanel(
                    topic = header.topic,
                    pinned = state.pinned,
                    pinnedCount = header.pinned.size,
                    canEdit = state.canEditTopic,
                    editing = mode == TopicPanelMode.EDITING,
                    onEdit = { panel = TopicPanelMode.EDITING },
                    onSave = { topic ->
                        actions.saveTopic(topic)
                        panel = TopicPanelMode.SHOWING
                    },
                    onCancel = { panel = TopicPanelMode.SHOWING },
                    onClose = { panel = null },
                    onOpenLink = onOpenLink,
                    onPinned = { eventId ->
                        panel = null
                        actions.goToOriginal(eventId)
                    },
                )
            }
            if (sharing) {
                SharedRoomsPanel(header.name, state.shared, images, today, onOpenRoom = onOpenRoom, onOpenSpace = onOpenSpace)
            }
        }
    }

    val dismiss = { overlay = null }
    // The row as it is now; the overlay closes if it is gone.
    fun rowOf(id: String): EventRow? = rows.firstOrNull { it.id == id } as? EventRow
    when (val shown = overlay) {
        null -> {}
        is Overlay.Menu -> {
            val row = rowOf(shown.rowId)
            val menu = row?.actions.orEmpty().filter {
                !shown.unsent || it == MessageAction.RETRY || it == MessageAction.UNSEND
            }
            if (row == null || menu.isEmpty()) {
                LaunchedEffect(shown) { overlay = null }
            } else {
                val media = row.message?.media.takeIf { !shown.unsent }
                MessageMenu(
                    row = row,
                    actions = menu,
                    hasFile = media != null,
                    onFile = { use ->
                        overlay = null
                        media?.let { actions.file(it, row.event.eventId, use) }
                    },
                    onDismiss = dismiss,
                    onReact = { key ->
                        overlay = null
                        actions.react(row.id, key)
                    },
                    onMoreReactions = { overlay = Overlay.EmojiPicker(row.id) },
                    onAction = { action ->
                        overlay = null
                        when (action) {
                            MessageAction.REACT -> {
                                overlay = Overlay.EmojiPicker(row.id)
                            }
                            MessageAction.REPLY -> actions.startReply(row.id)
                            MessageAction.COPY -> row.message?.let { clipboard.setText(AnnotatedString(it.body)) }
                            MessageAction.SELECT_TEXT -> row.message?.let { overlay = Overlay.SelectText(it.body) }
                            MessageAction.EDIT -> actions.startEdit(row.id)
                            MessageAction.PIN -> row.event.eventId?.let { actions.setPinned(it, true) }
                            MessageAction.UNPIN -> row.event.eventId?.let { actions.setPinned(it, false) }
                            MessageAction.DELETE -> {
                                overlay = Overlay.ConfirmDelete(row.id)
                            }
                            MessageAction.RETRY -> actions.retry(row.id)
                            MessageAction.UNSEND -> actions.unsend(row.id)
                        }
                    },
                )
            }
        }
        is Overlay.EmojiPicker -> EmojiPickerDialog(
            onPick = { key ->
                overlay = null
                actions.react(shown.rowId, key)
            },
            onDismiss = dismiss,
        )
        is Overlay.Reactors -> {
            val reaction = rowOf(shown.rowId)?.reactions?.firstOrNull { it.key == shown.key }
            if (reaction == null) {
                LaunchedEffect(shown) { overlay = null }
            } else {
                PeopleSheet("Reacted with ${reaction.key}", reaction.senders, images, onDismiss = dismiss)
            }
        }
        is Overlay.Readers -> {
            val readers = rowOf(shown.rowId)?.readers?.all.orEmpty()
            if (readers.isEmpty()) {
                LaunchedEffect(shown) { overlay = null }
            } else {
                PeopleSheet("Read by", readers, images, onDismiss = dismiss)
            }
        }
        is Overlay.SelectText -> SelectTextSheet(shown.text, onDismiss = dismiss)
        is Overlay.ConfirmDelete -> ConfirmDeleteDialog(
            onDelete = {
                overlay = null
                actions.delete(shown.rowId)
            },
            onDismiss = dismiss,
        )
    }
}

/** A direct chat's #: the rooms shared with the person ([name]), highlighted while [open]. */
@Composable
private fun SharedRoomsButton(name: String, open: Boolean, onClick: () -> Unit) {
    IconButton(
        onClick = onClick,
        colors = if (open) {
            IconButtonDefaults.iconButtonColors(
                containerColor = MaterialTheme.colorScheme.primaryContainer,
                contentColor = MaterialTheme.colorScheme.primary,
            )
        } else {
            IconButtonDefaults.iconButtonColors()
        },
    ) {
        Icon(MtrxIcons.Hash, contentDescription = if (open) "Hide the rooms shared with $name" else "Rooms shared with $name")
    }
}

/** A room's space, its small picture: tapped, the space on the Spaces tab. */
@Composable
private fun SpaceButton(space: RoomSummary, images: Images, onClick: () -> Unit) {
    IconButton(onClick = onClick, modifier = Modifier.semantics { contentDescription = "Space ${space.name}" }) {
        Avatar(space.name, space.avatarUrl, space.id, images, size = 28.dp, shape = RoundedCornerShape(6.dp))
    }
}

/** The topic panel: showing the topic, or editing it. */
private enum class TopicPanelMode { SHOWING, EDITING }

/**
 * The room's name, tapped for [onTitle]; a direct chat's with the person's
 * avatar and [presence] (the dot, and the line under the name). Below a
 * room's name, a caret and [topicLine] (after the pinned messages' count),
 * tapped for [onTopic].
 */
@Composable
private fun RoomTitle(
    header: RoomHeader,
    presence: Presence?,
    images: Images,
    topicLine: String?,
    topicOpen: Boolean,
    onTitle: () -> Unit,
    onTopic: () -> Unit,
) {
    val person = header.person
    Column {
        Row(
            Modifier
                .clip(RoundedCornerShape(8.dp))
                .clickable(onClick = onTitle),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (person != null) {
                WithPresenceDot(presence, avatarSize = 32.dp, dotSize = 10.dp, ring = MaterialTheme.colorScheme.surface) {
                    Avatar(header.name, header.avatarUrl, seed = header.seed, images = images, size = 32.dp)
                }
                Spacer(Modifier.width(10.dp))
            }
            Column {
                Text(header.name, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.titleMedium)
                if (person != null) PresenceLine(presence, style = MaterialTheme.typography.bodySmall)
            }
        }
        if (person == null && topicLine != null) {
            Row(
                Modifier
                    .clip(RoundedCornerShape(6.dp))
                    .clickable(onClick = onTopic)
                    .padding(end = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    if (topicOpen) Icons.Filled.KeyboardArrowDown else Icons.AutoMirrored.Filled.KeyboardArrowRight,
                    contentDescription = if (topicOpen) "Hide the topic" else "Show the topic",
                    modifier = Modifier.size(16.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (header.pinned.isNotEmpty()) {
                    Icon(
                        MtrxIcons.Pin,
                        contentDescription = "Pinned",
                        modifier = Modifier.size(12.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text(
                        "${header.pinned.size}  ",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Text(
                    topicLine,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

/** Who else is typing, in a line whose height stays while nobody is. */
@Composable
private fun TypingLine(text: String) {
    Text(
        text,
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 2.dp),
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        maxLines = 1,
        minLines = 1,
        overflow = TextOverflow.Ellipsis,
    )
}

@Composable
private fun Composer(
    draft: String,
    composing: Composing?,
    onDraft: (String) -> Unit,
    onCancel: () -> Unit,
    onSend: () -> Unit,
) {
    // Typed here; set from outside (an edit starts, a send clears), the cursor goes to the end.
    var field by remember { mutableStateOf(TextFieldValue(draft, TextRange(draft.length))) }
    val shown = if (field.text == draft) field else TextFieldValue(draft, TextRange(draft.length))
    val focus = remember { FocusRequester() }
    LaunchedEffect(composing) {
        if (composing != null) focus.requestFocus()
    }
    Surface(tonalElevation = 2.dp) {
        Column(Modifier.fillMaxWidth()) {
            if (composing != null) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(start = 16.dp, end = 4.dp, top = 4.dp)
                        .height(IntrinsicSize.Min),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(
                        Modifier
                            .width(3.dp)
                            .fillMaxHeight()
                            .background(MaterialTheme.colorScheme.primary),
                    )
                    Column(
                        Modifier
                            .weight(1f)
                            .padding(start = 8.dp),
                    ) {
                        val heading = when (composing) {
                            is Composing.Replying -> "Replying to ${composing.name}"
                            is Composing.Editing -> "Editing"
                        }
                        Text(heading, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
                        if (composing is Composing.Replying && composing.preview.isNotEmpty()) {
                            Text(
                                composing.preview,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                    IconButton(onClick = onCancel) {
                        Icon(Icons.Filled.Close, contentDescription = "Cancel")
                    }
                }
            }
            Row(
                Modifier
                    .fillMaxWidth()
                    .navigationBarsPadding()
                    .imePadding()
                    .padding(horizontal = 8.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OutlinedTextField(
                    value = shown,
                    onValueChange = {
                        field = it
                        if (it.text != draft) onDraft(it.text)
                    },
                    modifier = Modifier
                        .weight(1f)
                        .focusRequester(focus),
                    placeholder = { Text("Message") },
                    maxLines = 6,
                    keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                )
                IconButton(onClick = onSend, enabled = draft.isNotBlank()) {
                    Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "Send")
                }
            }
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun EventBubble(row: EventRow, images: Images, highlighted: Boolean, download: Download?, handlers: RowHandlers) {
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
                                name,
                                images,
                                row.undecryptableText,
                                colors,
                                download,
                                onOpenLink = handlers.openLink,
                                onHold = { handlers.hold(row) },
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
 * bubble's secondary colour. Web addresses in its text are links in the
 * bubble's link colour, tapped with [onOpenLink]; a hold on the text is
 * [onHold], as on the bubble.
 */
@Composable
private fun MessageBody(
    event: TimelineEvent,
    edited: Boolean,
    name: String,
    images: Images,
    undecryptableText: String,
    colors: BubbleColors,
    download: Download?,
    onOpenLink: (String) -> Unit,
    onHold: () -> Unit,
    onCancelDownload: (ULong) -> Unit,
    onOpenFile: () -> Unit,
) {
    val secondary = colors.secondary
    // Its links, and "(edited)", muted, after the text.
    fun text(body: String) = buildAnnotatedString {
        append(body)
        for (link in findLinks(body)) {
            addStyle(SpanStyle(color = colors.link, textDecoration = TextDecoration.Underline), link.start, link.end)
            addStringAnnotation(URL_TAG, link.url, link.start, link.end)
        }
        if (edited) withStyle(SpanStyle(color = secondary, fontStyle = FontStyle.Normal)) { append(" (edited)") }
    }
    when (val content = event.content) {
        is EventContent.Message -> {
            val message = content.message
            val media = message.media
            when {
                media != null && (media.kind == MediaKind.IMAGE || media.kind == MediaKind.VIDEO) -> {
                    Box(contentAlignment = Alignment.Center) {
                        MediaPreview(media, images)
                        DownloadProgress(download, onCancelDownload, onOpenFile)
                    }
                    messageText(message)?.let { LinkedText(text(it), onOpenLink, onHold, Modifier.padding(top = 6.dp)) }
                }
                media != null -> FileRow(media, download, secondary, onCancelDownload, onOpenFile)
                message.kind == MessageKind.EMOTE -> LinkedText(text("* $name ${message.body}"), onOpenLink, onHold, fontStyle = FontStyle.Italic)
                message.kind == MessageKind.NOTICE -> LinkedText(text(message.body), onOpenLink, onHold, color = secondary)
                else -> LinkedText(text(message.body), onOpenLink, onHold)
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

/** The annotation of a link's URL in a message's text. */
private const val URL_TAG = "url"

/**
 * [text] whose links ([URL_TAG]) open with [onOpenLink] when tapped (and
 * by the screen reader's actions); held anywhere it is [onHold]. Without
 * links, plain text: the bubble takes the touches.
 */
@Composable
private fun LinkedText(
    text: AnnotatedString,
    onOpenLink: (String) -> Unit,
    onHold: () -> Unit,
    modifier: Modifier = Modifier,
    color: Color = Color.Unspecified,
    fontStyle: FontStyle? = null,
) {
    val links = remember(text) { text.getStringAnnotations(URL_TAG, 0, text.length) }
    if (links.isEmpty()) {
        Text(text, modifier, color = color, fontStyle = fontStyle)
        return
    }
    var layout by remember { mutableStateOf<TextLayoutResult?>(null) }
    val open by rememberUpdatedState(onOpenLink)
    val hold by rememberUpdatedState(onHold)
    Text(
        text,
        modifier
            .pointerInput(links) {
                detectTapGestures(
                    onTap = { position -> layout?.let { linkAt(it, links, position) }?.let(open) },
                    onLongPress = { hold() },
                )
            }
            .semantics {
                customActions = links.map { link -> CustomAccessibilityAction("Open ${link.item}") { open(link.item); true } }
            },
        color = color,
        fontStyle = fontStyle,
        onTextLayout = { layout = it },
    )
}

/** The URL of the link of [links] at [position] in the text laid out as [layout], if any. */
private fun linkAt(layout: TextLayoutResult, links: List<AnnotatedString.Range<String>>, position: Offset): String? {
    // The character under it: before or after the nearest caret position.
    val nearest = layout.getOffsetForPosition(position)
    val char = listOf(nearest, nearest - 1).firstOrNull {
        it in 0 until layout.layoutInput.text.length && layout.getBoundingBox(it).contains(position)
    } ?: return null
    return links.firstOrNull { char >= it.start && char < it.end }?.item
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
private fun Notice(text: String) {
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
private fun DayDivider(label: String) {
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
private fun ReadMarker() {
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
