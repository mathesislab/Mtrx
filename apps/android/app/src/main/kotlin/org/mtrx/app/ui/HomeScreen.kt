package org.mtrx.app.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
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
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import java.time.LocalDate
import kotlinx.coroutines.launch
import org.mtrx.app.model.Account
import org.mtrx.app.model.PushRegistration
import org.mtrx.app.model.RoomSort
import org.mtrx.app.model.SignedInSession
import org.mtrx.app.model.directPerson
import org.mtrx.app.model.saved
import org.mtrx.app.model.showsEncryptionBanner
import org.mtrx.app.rooms.SectionPrefs
import org.mtrx.app.rooms.foldedGroups
import org.mtrx.app.rooms.name
import org.mtrx.app.rooms.roomGroups
import org.mtrx.app.rooms.shownRoom
import org.mtrx.app.settings.SettingsContent
import uniffi.mtrx.ConnectionState
import uniffi.mtrx.RoomSummary
import uniffi.mtrx.SpaceKey

/** What the home's other screens open: from the Spaces tab's + menu and a space's name. */
class HomeRoutes(
    val openOverview: (String) -> Unit,
    val newSpace: () -> Unit,
    val newRoom: (parent: String) -> Unit,
    val join: () -> Unit,
)

/**
 * The signed-in home, in four tabs ([nav]): Chats, the direct chats (one
 * opens in the tab, [roomPane]); Rooms, the rooms by space, newest first
 * (one opens in the tab), a space's heading opening it on Spaces; Spaces, a
 * pager of the spaces and the selected space's direct messages and rooms,
 * whose name opens its overview, with + for new spaces, rooms and joining;
 * and Settings. Chats' badge counts unread messages in the direct chats,
 * Rooms' those in the rooms, Spaces' those in the spaces' rooms; a room's
 * back arrow those everywhere else. Banners for the connection over all
 * tabs, and for encryption over the lists.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun HomeScreen(
    account: Account,
    session: SignedInSession,
    push: PushRegistration,
    nav: HomeNav,
    routes: HomeRoutes,
    onEnterRecoveryKey: () -> Unit,
    onVerify: () -> Unit,
    roomPane: @Composable (roomId: String, backCount: Long, onBack: () -> Unit, openSpace: (String) -> Unit) -> Unit,
) {
    val images = session.images
    val list by session.rooms.collectAsStateWithLifecycle()
    val presences by session.presence.presences.collectAsStateWithLifecycle()
    val connection by session.connection.collectAsStateWithLifecycle()
    val encryption by session.encryption.collectAsStateWithLifecycle()
    val selected by session.spaceSelection.selected.collectAsStateWithLifecycle()
    val sort by account.roomSort.sort.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(session) { session.messages.collect { snackbar.showSnackbar(it) } }
    val today = remember(list) { LocalDate.now() }
    val sectionPrefs by account.sections.prefs.collectAsStateWithLifecycle()
    val notes = remember(list) { list.notes() }
    val chatRooms = remember(list) { list.chats() }
    val chatSpaces = remember(list) { (chatRooms + listOfNotNull(notes)).associate { it.id to list.chatSpaces(it.id) } }
    val spaceEntries = remember(list) { list.spaces() }
    val sections = remember(list, selected, sort, sectionPrefs) {
        selected?.let { key ->
            val prefs = sectionPrefs[key.saved] ?: SectionPrefs()
            list.spaceList(key, prefs.library, sort.library, "")
        }.orEmpty()
    }
    var showing by rememberSaveable { mutableStateOf(false) }
    val groups = remember(list, sectionPrefs) { roomGroups(list, folded = foldedGroups(sectionPrefs)) }
    val drafts = remember(list) { list.drafts().toSet() }
    val badges = remember(list) {
        val counts = list.badges()
        mapOf(
            HomeTab.Chats to counts.chats.toLong(),
            HomeTab.Rooms to counts.rooms.toLong(),
            HomeTab.Spaces to counts.spaces.toLong(),
        )
    }
    val open: (RoomSummary) -> Unit = { room -> nav.open(room.id, room) }

    val scope = rememberCoroutineScope()
    // The selected space's rooms are the second page; without one there is none.
    val pager = rememberPagerState { if (selected != null) 2 else 1 }
    val chatsList = rememberLazyListState()
    val spacesList = rememberLazyListState()
    val roomsList = rememberLazyListState()
    val spaceRoomsList = rememberSaveable(selected?.saved, saver = LazyListState.Saver) { LazyListState() }
    val showSpaces: () -> Unit = { scope.launch { pager.animateScrollToPage(0) } }
    // A space's heading on Rooms (or in a room): the space's rooms on Spaces.
    val openSpace: (String) -> Unit = { id ->
        val entry = SpaceKey.Space(id)
        nav.tab = HomeTab.Spaces
        if (entry == selected) scope.launch { pager.animateScrollToPage(1) } else session.spaceSelection.select(entry)
    }

    // A new selection (tapped, or restored at start) shows its rooms; none
    // shows the spaces. The entry paged to is saved with the screen, so
    // coming back from a room keeps the page.
    var pagedTo by rememberSaveable { mutableStateOf<String?>(null) }
    LaunchedEffect(selected) {
        val entry = selected
        if (entry?.saved == pagedTo) return@LaunchedEffect
        pagedTo = entry?.saved
        when {
            entry == null -> pager.scrollToPage(0)
            nav.tab == HomeTab.Spaces -> pager.animateScrollToPage(1)
            else -> pager.scrollToPage(1)
        }
    }
    val onSpaceRooms = nav.tab == HomeTab.Spaces && pager.currentPage == 1 && selected != null
    BackHandler(enabled = onSpaceRooms, onBack = showSpaces)

    val chat = nav.chat.takeIf { nav.tab == HomeTab.Chats }
    val room = shownRoom(list, nav.room).takeIf { nav.tab == HomeTab.Rooms }
    BackHandler(enabled = chat != null) { nav.chat = null }
    BackHandler(enabled = room != null) { nav.room = null }

    val selectedSpace = spaceEntries.firstOrNull { it.key == selected }
    val selectedSpaceId = (selected as? SpaceKey.Space)?.id
    val title: @Composable () -> Unit = {
        when {
            onSpaceRooms -> SpaceTitle(
                selectedSpace?.name.orEmpty(),
                onClick = selectedSpaceId?.let { id -> { routes.openOverview(id) } },
            )
            else -> Text(nav.tab.label)
        }
    }
    // Which of the space's sections show, on this phone.
    val showingEntry = selected
    if (showing && onSpaceRooms && showingEntry != null) {
        val space = showingEntry.saved
        val choices = list.sectionChoices(showingEntry, (sectionPrefs[space] ?: SectionPrefs()).library)
        val keys = choices.map { it.key }
        ShowSheet(
            selectedSpace?.name.orEmpty(),
            choices,
            ShowActions(
                setShown = { key, on -> account.sections.setShown(space, key, on) },
                showAll = { account.sections.showAll(space) },
                hideAll = { account.sections.hideAll(space, keys) },
                move = { from, to -> account.sections.move(space, keys, from, to) },
            ),
            onDismiss = { showing = false },
        )
    }
    HomeFrame(
        tab = nav.tab,
        badges = badges,
        onTab = nav::tapTab,
        snackbar = snackbar,
        // A room shows its own header.
        showTopBar = chat == null && room == null,
        title = title,
        navigationIcon = {
            if (onSpaceRooms) {
                IconButton(onClick = showSpaces) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Spaces")
                }
            }
        },
        actions = {
            if (nav.tab == HomeTab.Spaces) {
                if (onSpaceRooms) {
                    IconButton(onClick = { showing = true }) {
                        Icon(MtrxIcons.Checklist, contentDescription = "Show…")
                    }
                    SortMenu(sort, onSort = account.roomSort::choose)
                }
                // New room in it, if we may add rooms to it.
                val space = (selected as? SpaceKey.Space)?.id
                    ?.takeIf { onSpaceRooms && selectedSpace?.space?.rights?.addRooms == true }
                AddMenu(
                    spaceName = selectedSpace?.name.takeIf { space != null },
                    onNewSpace = routes.newSpace,
                    onJoin = routes.join,
                    onNewRoom = space?.let { { routes.newRoom(it) } },
                )
            }
        },
    ) {
        ConnectionBanner(connection, onSignInAgain = { account.forget() })
        val listShown = nav.tab != HomeTab.Settings && chat == null && room == null
        // Settings says the same in its Encryption section.
        if (listShown && showsEncryptionBanner(encryption)) {
            EncryptionBanner(onClick = { nav.tab = HomeTab.Settings })
        }
        when (nav.tab) {
            HomeTab.Chats -> if (chat != null) {
                roomPane(chat, list.unreadBesides(chat).toLong(), { nav.chat = null }, openSpace)
            } else {
                ChatList(
                    notes,
                    chatRooms,
                    list.loaded(),
                    images,
                    today,
                    onOpenRoom = open,
                    presenceOf = { directPerson(it, session.info.userId)?.let { person -> presences[person] } },
                    spaces = chatSpaces,
                    drafts = drafts,
                    state = chatsList,
                )
            }
            HomeTab.Spaces -> HorizontalPager(pager, Modifier.fillMaxSize()) { page ->
                if (page == 0) {
                    SpaceList(spaceEntries, selected, list.loaded(), images, state = spacesList, onSelect = { entry ->
                        if (entry == selected) {
                            scope.launch { pager.animateScrollToPage(1) }
                        } else {
                            session.spaceSelection.select(entry)
                        }
                    })
                } else {
                    SpaceRoomsList(
                        notes,
                        sections,
                        list.loaded(),
                        images,
                        today,
                        onOpenRoom = open,
                        onFold = { key -> selected?.let { account.sections.toggleFold(it.saved, key) } },
                        drafts = drafts,
                        state = spaceRoomsList,
                    )
                }
            }
            HomeTab.Rooms -> if (room != null) {
                roomPane(room, list.unreadBesides(room).toLong(), { nav.room = null }, openSpace)
            } else {
                RoomGroupsList(
                    groups,
                    list.loaded(),
                    images,
                    today,
                    onOpenRoom = open,
                    onOpenSpace = openSpace,
                    onFold = account.sections::toggleGroupFold,
                    drafts = drafts,
                    state = roomsList,
                )
            }
            HomeTab.Settings -> SettingsContent(account, session, push, onEnterRecoveryKey, onVerify)
        }
    }
}

/**
 * The home's frame: the top bar (left out while a room shows its own),
 * the content, and the tabs with their unread [badges].
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun HomeFrame(
    tab: HomeTab,
    badges: Map<HomeTab, Long>,
    onTab: (HomeTab) -> Unit,
    snackbar: SnackbarHostState = remember { SnackbarHostState() },
    showTopBar: Boolean = true,
    title: @Composable () -> Unit = { Text(tab.label) },
    navigationIcon: @Composable () -> Unit = {},
    actions: @Composable () -> Unit = {},
    content: @Composable () -> Unit,
) {
    Scaffold(
        topBar = {
            if (showTopBar) {
                TopAppBar(title = title, navigationIcon = navigationIcon, actions = { actions() })
            }
        },
        bottomBar = {
            TextScaleAtMost(BAR_TEXT_SCALE) {
                Tabs(tab, badges, onTab)
            }
        },
        snackbarHost = { SnackbarHost(snackbar) },
        // The top bar and the tabs take the system bars; a room's header the status bar.
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
    ) { padding ->
        Column(
            Modifier
                .padding(padding)
                .consumeWindowInsets(padding)
                .fillMaxSize(),
        ) {
            content()
        }
    }
}

/** The tabs, [tab] selected, with their unread [badges]; their labels on one line. */
@Composable
private fun Tabs(tab: HomeTab, badges: Map<HomeTab, Long>, onTab: (HomeTab) -> Unit) {
    NavigationBar {
        HomeTab.entries.forEach {
            val count = badges[it] ?: 0L
            NavigationBarItem(
                selected = tab == it,
                onClick = { onTab(it) },
                icon = {
                    BadgedBox(badge = { if (count > 0) UnreadBadge(count) }) {
                        Icon(it.icon, contentDescription = null)
                    }
                },
                label = { Text(it.label, maxLines = 1, softWrap = false) },
            )
        }
    }
}

/** The space's name over its rooms, tapped for its overview (none for Home). */
@Composable
fun SpaceTitle(name: String, onClick: (() -> Unit)?) {
    Row(
        Modifier
            .clip(RoundedCornerShape(8.dp))
            .let { if (onClick != null) it.clickable(onClick = onClick) else it }
            .padding(end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(name, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
        if (onClick != null) Icon(Icons.Filled.KeyboardArrowDown, contentDescription = "Space overview")
    }
}

/** +: New space…, Join a space or room…, and inside a space New room in it…. */
@Composable
private fun AddMenu(spaceName: String?, onNewSpace: () -> Unit, onJoin: () -> Unit, onNewRoom: (() -> Unit)?) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) {
            Icon(Icons.Filled.Add, contentDescription = "New or join")
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            if (onNewRoom != null && spaceName != null) {
                DropdownMenuItem(text = { Text("New room in $spaceName…") }, onClick = {
                    open = false
                    onNewRoom()
                })
            }
            DropdownMenuItem(text = { Text("New space…") }, onClick = {
                open = false
                onNewSpace()
            })
            DropdownMenuItem(text = { Text("Join a space or room…") }, onClick = {
                open = false
                onJoin()
            })
        }
    }
}

/** The order of the spaces' rooms, chosen in a menu; the current one checked. */
@Composable
private fun SortMenu(sort: RoomSort, onSort: (RoomSort) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) {
            Icon(MtrxIcons.Sort, contentDescription = "Sort")
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            RoomSort.entries.forEach { option ->
                DropdownMenuItem(
                    text = { Text(option.label) },
                    onClick = {
                        open = false
                        onSort(option)
                    },
                    trailingIcon = if (option == sort) {
                        { Icon(Icons.Filled.Check, contentDescription = "Selected") }
                    } else {
                        null
                    },
                )
            }
        }
    }
}

@Composable
private fun ConnectionBanner(connection: ConnectionState, onSignInAgain: () -> Unit) {
    when (connection) {
        ConnectionState.ONLINE -> {}
        ConnectionState.OFFLINE -> Surface(color = MaterialTheme.colorScheme.secondaryContainer) {
            Text(
                "Offline. Messages are sent when the connection is back.",
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                style = MaterialTheme.typography.bodySmall,
            )
        }
        ConnectionState.SIGNED_OUT -> Surface(color = MaterialTheme.colorScheme.errorContainer) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(start = 16.dp, end = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    "The server signed this device out.",
                    Modifier.weight(1f),
                    style = MaterialTheme.typography.bodySmall,
                )
                TextButton(onClick = onSignInAgain) { Text("Sign in again") }
            }
        }
    }
}

@Composable
private fun EncryptionBanner(onClick: () -> Unit) {
    Surface(color = MaterialTheme.colorScheme.tertiaryContainer) {
        Row(
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(horizontal = 16.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Icon(Icons.Filled.Lock, contentDescription = null)
            Text(
                "Verify this device or enter your recovery key to read encrypted messages.",
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}
