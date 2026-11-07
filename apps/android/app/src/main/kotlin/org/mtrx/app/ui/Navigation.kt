package org.mtrx.app.ui

import android.content.ActivityNotFoundException
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
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.rememberNavController
import org.mtrx.app.model.Account
import org.mtrx.app.model.PushRegistration
import org.mtrx.app.model.SignedInSession
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
import org.mtrx.app.settings.AskNotificationPermissionOnce
import org.mtrx.app.settings.RecoveryScreen
import org.mtrx.app.settings.VerificationRequestDialog
import org.mtrx.app.settings.VerificationScreen
import org.mtrx.app.timeline.TimelineScreen
import org.mtrx.app.timeline.TimelineViewModel
import org.mtrx.app.timeline.roomHeader
import uniffi.mtrx.DirectoryRoom
import uniffi.mtrx.JoinButton
import uniffi.mtrx.Membership
import uniffi.mtrx.RoomKind
import uniffi.mtrx.RoomListsInterface
import uniffi.mtrx.parseAddress
import uniffi.mtrx.composeTopic
import uniffi.mtrx.parseTopic

/**
 * The screens of a signed-in session: the home (its tabs, with a room open
 * in Chats or Rooms), Room info, a space's overview, joining, a new room
 * or space, recovery and verification; and the prompt for a verification
 * request from another device, over any of them.
 */
@Composable
fun SignedInNavigation(account: Account, session: SignedInSession, push: PushRegistration) {
    val nav = rememberNavController()
    val home = rememberSaveable(saver = HomeNav.Saver) { HomeNav() }
    val rooms by session.rooms.collectAsStateWithLifecycle()
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
        home.open(roomId, rooms.room(roomId))
        nav.popBackTo(Route.HOME)
    }
    // The verification screen shows the session's verification of the moment:
    // a new one replaces a finished one still on screen.
    val openVerification = {
        nav.go(Destination.Verification) { popUpTo(Route.VERIFICATION.pattern) { inclusive = true } }
    }
    AskNotificationPermissionOnce(push)

    val request by session.verificationRequest.collectAsStateWithLifecycle()
    if (request != null) {
        VerificationRequestDialog(
            onAccept = {
                session.acceptVerificationRequest()
                openVerification()
            },
            onDecline = { session.declineVerificationRequest() },
        )
    }

    NavHost(nav, startDestination = Route.HOME.pattern) {
        screen(Route.HOME) {
            HomeScreen(
                account,
                session,
                push,
                home,
                HomeRoutes(
                    openOverview = { nav.go(Destination.Space(it)) },
                    newSpace = { nav.go(Destination.New(space = true, parent = null)) },
                    newRoom = { parent -> nav.go(Destination.New(space = false, parent = parent)) },
                    join = { nav.go(Destination.Join) },
                ),
                onEnterRecoveryKey = { nav.go(Destination.Recovery) },
                onVerify = {
                    session.verifyWithAnotherDevice()
                    openVerification()
                },
                roomPane = { roomId, backCount, onBack, openSpace ->
                    RoomPane(account, session, rooms, roomId, backCount, onBack, openLink, openRoom, openSpace) {
                        nav.go(Destination.Info(roomId))
                    }
                },
            )
        }
        screen(Route.RECOVERY) {
            RecoveryScreen(session, onBack = { nav.popBackStack() }, onDone = { nav.popBackStack() })
        }
        screen(Route.VERIFICATION) {
            VerificationScreen(session, onClose = { nav.popBackStack() })
        }
        screen(Route.INFO) { (roomId) ->
            val model: RoomInfoModel = viewModel { RoomInfoModel(roomId, session.session, ::composeTopic) }
            InfoScreen(session, rooms, model, nav, openLink)
        }
        screen(Route.SPACE) { (spaceId) ->
            val model: RoomInfoModel = viewModel { RoomInfoModel(spaceId, session.session, ::composeTopic) }
            val overview: SpaceOverviewModel = viewModel { SpaceOverviewModel(spaceId, session.session) }
            OverviewScreen(session, rooms, model, overview, nav, openLink, openRoom)
        }
        screen(Route.JOIN) {
            val model: JoinModel = viewModel { JoinModel(session.session, ::parseAddress) }
            val server by model.server.collectAsStateWithLifecycle()
            val query by model.query.collectAsStateWithLifecycle()
            val kind by model.kind.collectAsStateWithLifecycle()
            val results by model.results.collectAsStateWithLifecycle()
            val pending by model.pending.collectAsStateWithLifecycle()
            val error by model.error.collectAsStateWithLifecycle()
            JoinContent(
                JoinState(
                    ownServer = org.mtrx.app.rooms.serverOf(session.info.userId),
                    servers = model.servers,
                    server = server,
                    query = query,
                    kind = kind,
                    results = results,
                    pending = pending,
                    buttonOf = buttonOf(rooms),
                    error = error,
                ),
                session.images,
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
        screen(Route.NEW) { (space, parent) ->
            val model: NewRoomModel = viewModel { NewRoomModel(session.session, space, parent) }
            val form by model.form.collectAsStateWithLifecycle()
            val offersOwners by model.offersOwners.collectAsStateWithLifecycle()
            val creating by model.creating.collectAsStateWithLifecycle()
            val error by model.error.collectAsStateWithLifecycle()
            NewRoomContent(
                space = space,
                parentName = parent?.let { id -> rooms.room(id)?.name },
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
                                nav.go(Destination.Space(roomId)) { popUpTo(Route.HOME.pattern) }
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

/** What a listed room's row offers, with our membership as [rooms] have it (joined once joining, asked once asked). */
private fun buttonOf(rooms: RoomListsInterface): (DirectoryRoom) -> JoinButton = { room -> rooms.joinButton(room) }

/** Rows of rooms to join: a joined space opens its overview, a room on the home; an invite shows to be answered. */
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
        if (room.isSpace) nav.go(Destination.Space(room.id)) else openRoom(room.id)
    },
    answer = { room -> openRoom(room.id) },
    retry = retry,
)

/** The names of the rooms and spaces, by ID. */
private fun names(rooms: RoomListsInterface): Map<String, String> = rooms.rooms().associate { it.id to it.name }

/**
 * A room in a tab: its timeline (opened while it shows, its draft kept on
 * this device), or for an invite, Accept and Decline. Rooms and spaces it
 * leads to open with [openRoom] and [openSpace].
 */
@Composable
private fun RoomPane(
    account: Account,
    session: SignedInSession,
    rooms: RoomListsInterface,
    roomId: String,
    backCount: Long,
    onBack: () -> Unit,
    openLink: (String) -> Unit,
    openRoom: (String) -> Unit,
    openSpace: (String) -> Unit,
    onRoomInfo: () -> Unit,
) {
    val room = rooms.room(roomId)
    if (room?.membership == Membership.INVITED) {
        var busy by remember(roomId) { mutableStateOf(false) }
        InviteContent(
            room,
            session.images,
            busy,
            onAccept = {
                busy = true
                session.accept(roomId, room.name) { busy = false }
            },
            onDecline = {
                busy = true
                session.leave(roomId, room.name) { left ->
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
                session.session,
                onRead = { account.roomRead(roomId) },
                onInput = session.presence::input,
            )
        }
        // Its notifications go, and pushes for it post none while it shows.
        DisposableEffect(roomId) {
            account.roomOpened(roomId)
            onDispose { account.roomClosed(roomId) }
        }
        val userId = session.info.userId
        val presences by session.presence.presences.collectAsStateWithLifecycle()
        val header = remember(room, userId, rooms) { roomHeader(roomId, room, userId, ::parseTopic, rooms.firstSpaceOf(roomId)) }
        TimelineScreen(
            header,
            viewModel,
            session.images,
            spaces = remember(rooms) { rooms.topLevelSpaces().filter { it.rights.invite } },
            rooms = rooms,
            presences = presences,
            refreshPresence = session.presence::refresh,
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
    session: SignedInSession,
    rooms: RoomListsInterface,
    model: RoomInfoModel,
    nav: NavHostController,
    openLink: (String) -> Unit,
) {
    val settings by model.settings.collectAsStateWithLifecycle()
    val members by model.members.collectAsStateWithLifecycle()
    val busy by model.busy.collectAsStateWithLifecycle()
    val error by model.error.collectAsStateWithLifecycle()
    val room = rooms.room(model.roomId)
    val header = infoHeader(model.roomId, room)
    var leaving by remember { mutableStateOf(false) }
    RoomInfoContent(
        header,
        settings,
        members,
        busy,
        error,
        session.info.userId,
        names(rooms),
        session.images,
        model,
        onBack = { nav.popBackStack() },
        onLeave = { leaving = true },
        onOpenLink = openLink,
    )
    if (leaving) {
        LeaveDialog(header.name, onLeave = {
            leaving = false
            session.leave(model.roomId, header.name) { left -> if (left) nav.popBackTo(Route.HOME) }
        }, onDismiss = { leaving = false })
    }
}

/** A space's overview, from its name on the Spaces tab. */
@Composable
private fun OverviewScreen(
    session: SignedInSession,
    rooms: RoomListsInterface,
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
    val space = rooms.room(model.roomId)
    val header = infoHeader(model.roomId, space)
    var leaving by remember { mutableStateOf(false) }
    SpaceOverviewContent(
        header,
        OverviewRooms(children, updating, failed, pending, buttonOf(rooms)),
        settings,
        members,
        busy,
        infoError ?: overviewError,
        session.info.userId,
        names(rooms),
        session.images,
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
            session.leave(model.roomId, header.name) { left -> if (left) nav.popBackTo(Route.HOME) }
        }, onDismiss = { leaving = false })
    }
}

private fun infoHeader(roomId: String, room: uniffi.mtrx.RoomSummary?): RoomInfoHeader = RoomInfoHeader(
    roomId = roomId,
    name = room?.name ?: roomId,
    avatarUrl = room?.avatarUrl,
    topic = room?.topic?.takeIf { it.isNotBlank() }?.let { parseTopic(it) } ?: org.mtrx.app.rooms.NO_TOPIC,
    space = room?.kind == RoomKind.SPACE,
    rights = room?.rights ?: org.mtrx.app.rooms.NO_RIGHTS,
)
