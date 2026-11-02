package org.mtrx.app.ui

import android.content.ActivityNotFoundException
import android.net.Uri
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalUriHandler
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.ViewModelStoreOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import org.mtrx.app.model.Account
import org.mtrx.app.model.AccountState
import org.mtrx.app.model.PushRegistration
import org.mtrx.app.rooms.InviteContent
import org.mtrx.app.rooms.JoinActions
import org.mtrx.app.rooms.JoinContent
import org.mtrx.app.rooms.JoinModel
import org.mtrx.app.rooms.JoinState
import org.mtrx.app.rooms.LeaveDialog
import org.mtrx.app.rooms.NewRoomActions
import org.mtrx.app.rooms.NewRoomContent
import org.mtrx.app.rooms.NewRoomModel
import org.mtrx.app.rooms.OverviewRooms
import org.mtrx.app.rooms.OverviewRowActions
import org.mtrx.app.rooms.RoomInfoContent
import org.mtrx.app.rooms.RoomInfoHeader
import org.mtrx.app.rooms.RoomInfoModel
import org.mtrx.app.rooms.SpaceOverviewContent
import org.mtrx.app.rooms.SpaceOverviewModel
import org.mtrx.app.rooms.roomSpace
import org.mtrx.app.rooms.topLevelSpaces
import org.mtrx.app.settings.AskNotificationPermissionOnce
import org.mtrx.app.settings.RecoveryScreen
import org.mtrx.app.settings.SessionInfo
import org.mtrx.app.settings.VerificationRequestDialog
import org.mtrx.app.settings.VerificationScreen
import org.mtrx.app.timeline.TimelineScreen
import org.mtrx.app.timeline.TimelineViewModel
import org.mtrx.app.timeline.roomHeader
import uniffi.mtrx.DirectoryRoom
import uniffi.mtrx.RoomKind
import uniffi.mtrx.RoomListState
import uniffi.mtrx.parseAddress
import uniffi.mtrx.composeTopic
import uniffi.mtrx.parseTopic

private const val HOME = "home"
private const val INFO = "info/{roomId}"
private const val SPACE = "space/{roomId}"
private const val JOIN = "join"
private const val NEW = "new?space={space}&parent={parent}"
private const val RECOVERY = "settings/recovery"
private const val VERIFICATION = "settings/verification"

/**
 * The screens of a signed-in session: the home (its tabs, with a room open
 * in Chats or Rooms), Room info, a space's overview, joining, a new room
 * or space, recovery and verification; and the prompt for a verification
 * request from another device, over any of them.
 */
@Composable
fun SignedInNavigation(account: Account, push: PushRegistration, signedIn: AccountState.SignedIn) {
    val nav = rememberNavController()
    val home = rememberSaveable(saver = HomeNav.Saver) { HomeNav() }
    val session = signedIn.session
    val info = remember(session) { SessionInfo(session.userId(), session.homeserver(), session.deviceId()) }
    val rooms by account.rooms.collectAsStateWithLifecycle()
    val uriHandler = LocalUriHandler.current
    val openLink: (String) -> Unit = { url ->
        try {
            uriHandler.openUri(url)
        } catch (e: ActivityNotFoundException) {
            // No app opens it.
        } catch (e: IllegalArgumentException) {
            // Not a URI.
        }
    }
    // A room from another screen: back to the home, where it opens.
    val openRoom: (String) -> Unit = { roomId ->
        home.open(roomId, rooms.rooms.firstOrNull { it.id == roomId })
        nav.popBackStack(HOME, inclusive = false)
    }
    // The verification screen shows the account's verification of the moment:
    // a new one replaces a finished one still on screen.
    val openVerification = {
        nav.navigate(VERIFICATION) { popUpTo(VERIFICATION) { inclusive = true } }
    }
    AskNotificationPermissionOnce(push)

    val request by account.verificationRequest.collectAsStateWithLifecycle()
    if (request != null) {
        VerificationRequestDialog(
            onAccept = {
                account.acceptVerificationRequest()
                openVerification()
            },
            onDecline = { account.declineVerificationRequest() },
        )
    }

    NavHost(nav, startDestination = HOME) {
        composable(HOME) {
            HomeScreen(
                account,
                push,
                info,
                signedIn.images,
                signedIn.presence,
                home,
                HomeRoutes(
                    openOverview = { nav.navigate("space/${Uri.encode(it)}") },
                    newSpace = { nav.navigate("new?space=true&parent=") },
                    newRoom = { parent -> nav.navigate("new?space=false&parent=${Uri.encode(parent)}") },
                    join = { nav.navigate(JOIN) },
                ),
                onEnterRecoveryKey = { nav.navigate(RECOVERY) },
                onVerify = {
                    account.verifyWithAnotherDevice()
                    openVerification()
                },
                roomPane = { roomId, backCount, onBack, openSpace ->
                    RoomPane(account, signedIn, info, rooms, roomId, backCount, onBack, openLink, openRoom, openSpace) {
                        nav.navigate("info/${Uri.encode(roomId)}")
                    }
                },
            )
        }
        composable(RECOVERY) {
            RecoveryScreen(account, onBack = { nav.popBackStack() }, onDone = { nav.popBackStack() })
        }
        composable(VERIFICATION) {
            VerificationScreen(account, onClose = { nav.popBackStack() })
        }
        composable(INFO, arguments = listOf(navArgument("roomId") { type = NavType.StringType })) { entry ->
            val roomId = entry.arguments?.getString("roomId") ?: return@composable
            val model: RoomInfoModel = viewModel { RoomInfoModel(roomId, session, ::composeTopic) }
            InfoScreen(account, signedIn, info, rooms, model, nav, openLink)
        }
        composable(SPACE, arguments = listOf(navArgument("roomId") { type = NavType.StringType })) { entry ->
            val spaceId = entry.arguments?.getString("roomId") ?: return@composable
            val model: RoomInfoModel = viewModel { RoomInfoModel(spaceId, session, ::composeTopic) }
            val overview: SpaceOverviewModel = viewModel { SpaceOverviewModel(spaceId, session, account.spaceChildren) }
            OverviewScreen(account, signedIn, info, rooms, model, overview, nav, openLink, openRoom)
        }
        composable(JOIN) {
            val model: JoinModel = viewModel { JoinModel(session, ::parseAddress) }
            val server by model.server.collectAsStateWithLifecycle()
            val query by model.query.collectAsStateWithLifecycle()
            val kind by model.kind.collectAsStateWithLifecycle()
            val results by model.results.collectAsStateWithLifecycle()
            val pending by model.pending.collectAsStateWithLifecycle()
            val error by model.error.collectAsStateWithLifecycle()
            JoinContent(
                JoinState(
                    ownServer = org.mtrx.app.rooms.serverOf(info.userId),
                    servers = model.servers,
                    server = server,
                    query = query,
                    kind = kind,
                    results = results,
                    pending = pending,
                    joined = joinedIds(rooms),
                    error = error,
                ),
                signedIn.images,
                JoinActions(
                    setServer = model::setServer,
                    setQuery = model::setQuery,
                    setKind = model::setKind,
                    retry = model::retry,
                    more = model::more,
                    clearError = model::clearError,
                    rows = rowActions(model::join, model::knock, nav, openRoom, model::retry),
                ),
                onBack = { nav.popBackStack() },
            )
        }
        composable(
            NEW,
            arguments = listOf(
                navArgument("space") { type = NavType.BoolType },
                navArgument("parent") {
                    type = NavType.StringType
                    defaultValue = ""
                },
            ),
        ) { entry ->
            val space = entry.arguments?.getBoolean("space") ?: false
            val parent = entry.arguments?.getString("parent")?.ifEmpty { null }
            val model: NewRoomModel = viewModel { NewRoomModel(session, space, parent) }
            val form by model.form.collectAsStateWithLifecycle()
            val offersOwners by model.offersOwners.collectAsStateWithLifecycle()
            val creating by model.creating.collectAsStateWithLifecycle()
            val error by model.error.collectAsStateWithLifecycle()
            NewRoomContent(
                space = space,
                parentName = parent?.let { id -> rooms.rooms.firstOrNull { it.id == id }?.name },
                form = form,
                offersOwners = offersOwners,
                creating = creating,
                error = error,
                actions = NewRoomActions(
                    edit = model::edit,
                    addPerson = model::addPerson,
                    removePerson = model::removePerson,
                    setCoOwner = model::setCoOwner,
                    create = {
                        model.create { roomId ->
                            if (space) {
                                nav.navigate("space/${Uri.encode(roomId)}") { popUpTo(HOME) }
                            } else {
                                openRoom(roomId)
                            }
                        }
                    },
                    clearError = model::clearError,
                ),
                onBack = { nav.popBackStack() },
            )
        }
    }

    // A tapped notification's room, over whatever shows (after the graph is set).
    val roomRequest by account.roomRequest.collectAsStateWithLifecycle()
    LaunchedEffect(roomRequest) {
        val roomId = roomRequest ?: return@LaunchedEffect
        openRoom(roomId)
        account.roomRequestHandled()
    }
}

/** The rooms the room list has us in (not invited). */
private fun joinedIds(rooms: RoomListState): Set<String> = rooms.rooms.filter { !it.invited }.map { it.id }.toSet()

/** Rows of rooms to join: a joined space opens its overview, a room on the home. */
private fun rowActions(
    join: (DirectoryRoom) -> Unit,
    knock: (DirectoryRoom) -> Unit,
    nav: NavHostController,
    openRoom: (String) -> Unit,
    retry: () -> Unit,
) = OverviewRowActions(
    join = join,
    knock = knock,
    open = { room ->
        if (room.isSpace) nav.navigate("space/${Uri.encode(room.id)}") else openRoom(room.id)
    },
    retry = retry,
)

/** The names of the rooms and spaces, by ID. */
private fun names(rooms: RoomListState): Map<String, String> = rooms.rooms.associate { it.id to it.name }

/**
 * A room in a tab: its timeline (opened while it shows, its draft kept on
 * this device), or for an invite, Accept and Decline. Rooms and spaces it
 * leads to open with [openRoom] and [openSpace].
 */
@Composable
private fun RoomPane(
    account: Account,
    signedIn: AccountState.SignedIn,
    info: SessionInfo,
    rooms: RoomListState,
    roomId: String,
    backCount: Long,
    onBack: () -> Unit,
    openLink: (String) -> Unit,
    openRoom: (String) -> Unit,
    openSpace: (String) -> Unit,
    onRoomInfo: () -> Unit,
) {
    val room = rooms.rooms.firstOrNull { it.id == roomId }
    if (room?.invited == true) {
        var busy by remember(roomId) { mutableStateOf(false) }
        InviteContent(
            room,
            signedIn.images,
            busy,
            onAccept = {
                busy = true
                account.accept(roomId, room.name) { busy = false }
            },
            onDecline = {
                busy = true
                account.leave(roomId, room.name) { left ->
                    busy = false
                    if (left) onBack()
                }
            },
            onBack = onBack,
        )
        return
    }
    key(roomId) {
        // The timeline's model lives while the room shows.
        val owner = remember {
            object : ViewModelStoreOwner {
                override val viewModelStore = ViewModelStore()
            }
        }
        DisposableEffect(owner) { onDispose { owner.viewModelStore.clear() } }
        val viewModel: TimelineViewModel = viewModel(viewModelStoreOwner = owner) {
            TimelineViewModel(
                roomId,
                signedIn.session,
                onRead = { account.roomRead(roomId) },
                onInput = account::input,
            )
        }
        // Its notifications go, and pushes for it post none while it shows.
        DisposableEffect(roomId) {
            account.roomOpened(roomId)
            onDispose { account.roomClosed(roomId) }
        }
        val presences by signedIn.presence.presences.collectAsStateWithLifecycle()
        val header = remember(room, info.userId, rooms) { roomHeader(roomId, room, info.userId, ::parseTopic, roomSpace(rooms, roomId)) }
        TimelineScreen(
            header,
            viewModel,
            signedIn.images,
            spaces = remember(rooms) { topLevelSpaces(rooms) },
            rooms = rooms,
            presences = presences,
            refreshPresence = signedIn.presence::refresh,
            backCount = backCount,
            onBack = onBack,
            onRoomInfo = onRoomInfo,
            onOpenLink = openLink,
            onOpenRoom = openRoom,
            onOpenSpace = openSpace,
        )
    }
}

/** Room info of a room, from its name; leaving goes back to the home. */
@Composable
private fun InfoScreen(
    account: Account,
    signedIn: AccountState.SignedIn,
    info: SessionInfo,
    rooms: RoomListState,
    model: RoomInfoModel,
    nav: NavHostController,
    openLink: (String) -> Unit,
) {
    val settings by model.settings.collectAsStateWithLifecycle()
    val members by model.members.collectAsStateWithLifecycle()
    val busy by model.busy.collectAsStateWithLifecycle()
    val error by model.error.collectAsStateWithLifecycle()
    val room = rooms.rooms.firstOrNull { it.id == model.roomId }
    val header = infoHeader(model.roomId, room)
    var leaving by remember { mutableStateOf(false) }
    RoomInfoContent(
        header,
        settings,
        members,
        busy,
        error,
        info.userId,
        names(rooms),
        signedIn.images,
        model,
        onBack = { nav.popBackStack() },
        onLeave = { leaving = true },
        onOpenLink = openLink,
    )
    if (leaving) {
        LeaveDialog(header.name, onLeave = {
            leaving = false
            account.leave(model.roomId, header.name) { left -> if (left) nav.popBackStack(HOME, inclusive = false) }
        }, onDismiss = { leaving = false })
    }
}

/** A space's overview, from its name on the Spaces tab. */
@Composable
private fun OverviewScreen(
    account: Account,
    signedIn: AccountState.SignedIn,
    info: SessionInfo,
    rooms: RoomListState,
    model: RoomInfoModel,
    overview: SpaceOverviewModel,
    nav: NavHostController,
    openLink: (String) -> Unit,
    openRoom: (String) -> Unit,
) {
    val settings by model.settings.collectAsStateWithLifecycle()
    val members by model.members.collectAsStateWithLifecycle()
    val busy by model.busy.collectAsStateWithLifecycle()
    val infoError by model.error.collectAsStateWithLifecycle()
    val children by overview.rooms.collectAsStateWithLifecycle()
    val updating by overview.updating.collectAsStateWithLifecycle()
    val failed by overview.failed.collectAsStateWithLifecycle()
    val pending by overview.pending.collectAsStateWithLifecycle()
    val overviewError by overview.error.collectAsStateWithLifecycle()
    val space = rooms.rooms.firstOrNull { it.id == model.roomId }
    val header = infoHeader(model.roomId, space)
    var leaving by remember { mutableStateOf(false) }
    SpaceOverviewContent(
        header,
        OverviewRooms(children, updating, failed, pending, joinedIds(rooms)),
        settings,
        members,
        busy,
        infoError ?: overviewError,
        info.userId,
        names(rooms),
        signedIn.images,
        rowActions(overview::join, overview::knock, nav, openRoom, overview::refresh),
        object : org.mtrx.app.rooms.RoomInfoActions by model {
            override fun clearError() {
                model.clearError()
                overview.clearError()
            }
        },
        onBack = { nav.popBackStack() },
        onLeave = { leaving = true },
        onOpenLink = openLink,
    )
    if (leaving) {
        LeaveDialog(header.name, onLeave = {
            leaving = false
            account.leave(model.roomId, header.name) { left -> if (left) nav.popBackStack(HOME, inclusive = false) }
        }, onDismiss = { leaving = false })
    }
}

private fun infoHeader(roomId: String, room: uniffi.mtrx.RoomSummary?): RoomInfoHeader = RoomInfoHeader(
    roomId = roomId,
    name = room?.name ?: roomId,
    avatarUrl = room?.avatarUrl,
    topic = room?.topic?.takeIf { it.isNotBlank() }?.let { parseTopic(it) } ?: org.mtrx.app.rooms.NO_TOPIC,
    space = room?.kind == RoomKind.SPACE,
)
