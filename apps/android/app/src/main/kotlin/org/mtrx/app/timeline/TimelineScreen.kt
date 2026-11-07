package org.mtrx.app.timeline

import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.ui.platform.LocalContext
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.BadgedBox
import kotlinx.coroutines.flow.Flow
import org.mtrx.app.rooms.topicPreview
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import java.time.LocalDate
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import org.mtrx.app.model.Images
import org.mtrx.app.rooms.RoomInfoModel
import org.mtrx.app.ui.Avatar
import org.mtrx.app.ui.BAR_TEXT_SCALE
import org.mtrx.app.ui.TextScaleAtMost
import org.mtrx.app.ui.MtrxIcons
import org.mtrx.app.ui.UnreadBadge
import org.mtrx.app.ui.PresenceLine
import org.mtrx.app.ui.WithPresenceDot
import org.mtrx.app.ui.dayLabel
import org.mtrx.app.ui.linesHeight
import uniffi.mtrx.Download
import uniffi.mtrx.MessageAction
import uniffi.mtrx.Presence
import uniffi.mtrx.RoomListsInterface
import uniffi.mtrx.RoomSummary
import uniffi.mtrx.Session
import uniffi.mtrx.Topic

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
    /** Each message's file's newest download, by its source. */
    val downloads: Map<String, Download> = emptyMap(),
)

/**
 * A room's timeline ([viewModel]), its pinned messages ([pinned], while the
 * topic panel shows) and its topic, saved by [info]. [spaces], the
 * top-level ones we may invite to, to invite a direct chat's person to;
 * [presences], for the header and the person sheet, and [refreshPresence]
 * when the sheet opens. [backCount] on the back button: unread messages in
 * the other rooms and direct chats.
 * The room's name opens [onRoomInfo] (a direct chat's, its person); links of
 * the topic open with [onOpenLink]. A direct chat's rooms shared with the
 * person are read from [session] and grouped as [rooms] lists them; one
 * opens with [onOpenRoom], a space with [onOpenSpace] (on the Spaces tab).
 */
@Composable
fun TimelineScreen(
    header: RoomHeader,
    viewModel: TimelineViewModel,
    pinned: PinnedModel,
    info: RoomInfoModel,
    session: Session,
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
    val topicError by info.error.collectAsStateWithLifecycle()
    val personInfo by viewModel.personInfo.collectAsStateWithLifecycle()
    val pinnedRows by pinned.pinned.collectAsStateWithLifecycle()
    val downloads by viewModel.downloads.collectAsStateWithLifecycle()
    // The app may be ended in the background: the draft is kept first.
    LifecycleEventEffect(Lifecycle.Event.ON_STOP) { viewModel.keepDraft() }
    val state = TimelineState(
        rows = rows,
        draft = draft,
        composing = composing,
        typing = typing,
        seeking = seeking,
        highlighted = highlighted,
        // The timeline's first, then the topic's.
        error = error ?: topicError,
        canEditTopic = header.rights.changeTopic,
        pinned = pinnedRows,
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
        onSaveTopic = info::setTopic,
        onShowPinned = { shown -> if (shown) pinned.show() else pinned.hide() },
        onErrorShown = { if (error != null) viewModel.clearError() else info.clearError() },
        sharedRooms = { person ->
            SharedRoomsPanel(person, header.name, session, rooms, images, onOpenRoom = onOpenRoom, onOpenSpace = onOpenSpace)
        },
    )

    personInfo?.let { model ->
        // Fresher than the subscription's, and for someone not in it.
        LaunchedEffect(model) { refreshPresence(model.userId) }
        PersonInfoSheet(model, presences[model.userId], spaces, images, onDismiss = viewModel::closePersonInfo)
    }
}

/**
 * The room screen as [state] says, without a model: the header with its
 * topic line and panel ([topicOpen] or [editingTopic] at first; the topic
 * saved with [onSaveTopic], the pinned messages shown while it is open,
 * [onShowPinned]) and its space, or a direct chat's # and the panel of the
 * rooms shared with the person ([sharedRooms], [sharedOpen] at first); the
 * timeline, the composer and the menus of messages. [state]'s error shows
 * once, then [onErrorShown].
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
    onSaveTopic: (Topic) -> Unit,
    onShowPinned: (Boolean) -> Unit,
    onErrorShown: () -> Unit,
    /** The panel of the rooms shared with a direct chat's person (their user ID). */
    sharedRooms: @Composable (String) -> Unit,
    /** Downloaded files to open, share or save; none in the screenshots (no activity). */
    files: Flow<FileRequest>? = null,
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
        onErrorShown()
    }

    // Downloaded files: opened, shared or saved.
    if (files != null) FileRequests(files) { message -> scope.launch { snackbar.showSnackbar(message) } }

    // The pinned messages, while the panel is open.
    val panelOpen = panel != null
    LaunchedEffect(panelOpen) { onShowPinned(panelOpen) }
    BackHandler(enabled = panelOpen) { panel = null }

    // A direct chat's rooms shared with the person, over the timeline and
    // the composer (the keyboard goes).
    var sharing by remember { mutableStateOf(sharedOpen) }
    val keyboard = LocalSoftwareKeyboardController.current
    val focus = LocalFocusManager.current
    val person = header.person
    LaunchedEffect(sharing, person) {
        if (sharing && person != null) {
            focus.clearFocus()
            keyboard?.hide()
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
            showPerson = { userId, name -> actions.showPersonInfo(PersonCard(userId, name, avatarUrl = null)) },
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
                            onTitle = {
                                if (person != null) actions.showPersonInfo(PersonCard(person, header.name, header.avatarUrl)) else onRoomInfo()
                            },
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
                        onSaveTopic(topic)
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
            if (sharing && person != null) sharedRooms(person)
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

/**
 * Hands [files] to other apps: opened, shared, or saved to Downloads (on
 * API 26–28 where the user picks); [say] what came of it.
 */
@Composable
private fun FileRequests(files: Flow<FileRequest>, say: (String) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var saving by remember { mutableStateOf<FileRequest?>(null) }
    val createDocument = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("*/*")) { target ->
        val request = saving ?: return@rememberLauncherForActivityResult
        saving = null
        if (target != null) {
            scope.launch { say(if (copyFile(context, request, target)) "Saved" else "Couldn't save the file") }
        }
    }
    LaunchedEffect(files) {
        files.collect { request ->
            when (request.use) {
                FileUse.OPEN -> if (!openFile(context, request)) say("No app can open this file")
                FileUse.SHARE -> shareFile(context, request)
                FileUse.SAVE -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    scope.launch { say(if (saveToDownloads(context, request)) "Saved to Downloads" else "Couldn't save the file") }
                } else {
                    saving = request
                    createDocument.launch(request.media.filename)
                }
            }
        }
    }
}
