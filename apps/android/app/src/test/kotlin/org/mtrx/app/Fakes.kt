package org.mtrx.app

import java.time.Instant
import kotlinx.coroutines.CompletableDeferred
import kotlin.coroutines.ContinuationInterceptor
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestDispatcher
import org.mtrx.app.model.Account
import org.mtrx.app.model.Distributors
import org.mtrx.app.model.GatewayDiscovery
import org.mtrx.app.model.Logger
import org.mtrx.app.model.PushRegistration
import org.mtrx.app.model.PushStore
import org.mtrx.app.rooms.SectionPrefs
import org.mtrx.app.rooms.SectionStore
import org.mtrx.app.model.PusherDevice
import org.mtrx.app.model.RoomNotifications
import org.mtrx.app.model.RoomSort
import org.mtrx.app.model.RoomSortStore
import org.mtrx.app.model.SelectedSpaceStore
import org.mtrx.app.model.SharePresenceStore
import org.mtrx.app.model.SpaceEntry
import org.mtrx.app.model.originGateway
import org.mtrx.app.push.MessageNotifier
import uniffi.mtrx.Access
import uniffi.mtrx.Address
import uniffi.mtrx.Banned
import uniffi.mtrx.ConnectionListener
import uniffi.mtrx.DirectoryKind
import uniffi.mtrx.DirectoryPage
import uniffi.mtrx.DirectoryRoom
import uniffi.mtrx.History
import uniffi.mtrx.JoinAccess
import uniffi.mtrx.LevelChoice
import uniffi.mtrx.Manage
import uniffi.mtrx.Member
import uniffi.mtrx.Membership
import uniffi.mtrx.NewRoom
import uniffi.mtrx.NotifyMode
import uniffi.mtrx.Permissions
import uniffi.mtrx.Role
import uniffi.mtrx.RoomSettings
import uniffi.mtrx.SpaceLink
import uniffi.mtrx.ConnectionState
import uniffi.mtrx.DeviceInfo
import uniffi.mtrx.Draft
import uniffi.mtrx.EncryptionState
import uniffi.mtrx.EncryptionStateListener
import uniffi.mtrx.EventContent
import uniffi.mtrx.Message
import uniffi.mtrx.MessageAction
import uniffi.mtrx.MessageKind
import uniffi.mtrx.MtrxException
import uniffi.mtrx.MtrxInterface
import uniffi.mtrx.NoHandle
import uniffi.mtrx.NotificationContent
import uniffi.mtrx.OAuthLogin
import uniffi.mtrx.Presence
import uniffi.mtrx.PresenceListener
import uniffi.mtrx.PresenceState
import uniffi.mtrx.PusherInfo
import uniffi.mtrx.Reader
import uniffi.mtrx.RecoveryListener
import uniffi.mtrx.RecoveryStatus
import uniffi.mtrx.RecoveryStep
import uniffi.mtrx.RoomKind
import uniffi.mtrx.RoomListListener
import uniffi.mtrx.RoomListState
import uniffi.mtrx.RoomSummary
import uniffi.mtrx.RowContent
import uniffi.mtrx.SendState
import uniffi.mtrx.SpaceRooms
import uniffi.mtrx.Session
import uniffi.mtrx.TaskHandle
import uniffi.mtrx.Timeline
import uniffi.mtrx.TimelineEvent
import uniffi.mtrx.TimelineRow
import uniffi.mtrx.TimelineRowsListener
import uniffi.mtrx.TypingListener
import uniffi.mtrx.UserProfile
import uniffi.mtrx.Verification
import uniffi.mtrx.VerificationListener
import uniffi.mtrx.VerificationRequestListener
import uniffi.mtrx.VerificationState

// Fakes of the library's objects: subclasses made with NoHandle, which never
// reach the native library.

class FakeHandle : TaskHandle(NoHandle) {
    var cancelled = false

    override fun cancel() {
        cancelled = true
    }

    override fun isFinished(): Boolean = cancelled
}

class FakeSession : Session(NoHandle) {
    var syncStarts = 0
    var syncStops = 0
    var loggedOut = false
    var logoutError: MtrxException? = null
    val handles = mutableListOf<FakeHandle>()
    var roomListListener: RoomListListener? = null

    var encryption = EncryptionState(deviceVerified = false, recovery = RecoveryStatus.INCOMPLETE)
    var encryptionAsks = 0
    var encryptionListener: EncryptionStateListener? = null
    var encryptionSubscription: FakeHandle? = null
    var encryptionSubscriptions = 0

    /** What [timeline] opens. */
    var roomTimeline: Timeline = FakeTimeline()

    /** [userProfile] gives [profile], or throws [profileError]; after [profileGate] if set. */
    var profile: UserProfile? = null
    var profileError: MtrxException? = null
    var profileGate: CompletableDeferred<Unit>? = null

    /** Invites made, as (room, user); [inviteError] fails them, after [inviteGate] if set. */
    val invites = mutableListOf<Pair<String, String>>()
    var inviteError: MtrxException? = null
    var inviteGate: CompletableDeferred<Unit>? = null

    /** The steps [recover] reports, then [recoveryError] if set. */
    var recoverySteps: List<RecoveryStep> = emptyList()
    var recoveryError: MtrxException? = null

    /** Called after each recovery step reached the listener. */
    var onRecoveryStep: () -> Unit = {}
    val recoveryKeys = mutableListOf<String>()

    var selfVerification: FakeVerification = FakeVerification(USER_ID)
    var verificationRequests: VerificationRequestListener? = null

    /** The account's pushers, by pushkey; [pusherError] fails setting one. */
    val serverPushers = linkedMapOf<String, PusherInfo>()
    var pusherError: MtrxException? = null
    val pusherCalls = mutableListOf<String>()

    override suspend fun setPusher(
        gatewayUrl: String,
        pushkey: String,
        appId: String,
        appName: String,
        deviceName: String,
        lang: String,
    ) {
        pusherCalls += "set $pushkey"
        pusherError?.let { throw it }
        serverPushers[pushkey] = PusherInfo(pushkey, appId, appName, deviceName, lang, gatewayUrl, eventIdOnly = true)
    }

    override suspend fun deletePusher(pushkey: String) {
        pusherCalls += "delete $pushkey"
        serverPushers.remove(pushkey)
    }

    override suspend fun pushers(): List<PusherInfo> = serverPushers.values.toList()

    override fun userId(): String = USER_ID

    override fun connection(): ConnectionState = ConnectionState.ONLINE

    /** Sync and presence calls in order: "startSync", "stopSync", "presence online". */
    val calls = mutableListOf<String>()

    override suspend fun startSync() {
        syncStarts++
        calls += "startSync"
    }

    override suspend fun stopSync() {
        syncStops++
        calls += "stopSync"
    }

    /** Our presence as set, in order; [setPresenceError] fails the calls. */
    val presenceSets = mutableListOf<PresenceState>()
    var setPresenceError: MtrxException? = null

    override suspend fun setPresence(state: PresenceState, statusMessage: String?) {
        presenceSets += state
        calls += "presence ${state.name.lowercase()}"
        setPresenceError?.let { throw it }
    }

    /** [presence] gives these, or offline without more; [presenceError] fails it. */
    val presences = mutableMapOf<String, Presence>()
    var presenceError: MtrxException? = null
    val presenceReads = mutableListOf<String>()

    override suspend fun presence(userId: String): Presence {
        presenceReads += userId
        presenceError?.let { throw it }
        return presences[userId] ?: makePresence(PresenceState.OFFLINE)
    }

    /** Each subscription's users and handle; the test calls the last [presenceListener]. */
    val presenceSubscriptions = mutableListOf<Pair<List<String>, FakeHandle>>()
    var presenceListener: PresenceListener? = null

    override fun subscribePresence(userIds: List<String>, listener: PresenceListener): TaskHandle {
        presenceListener = listener
        return FakeHandle().also {
            handles += it
            presenceSubscriptions += userIds to it
        }
    }

    override fun subscribeConnection(listener: ConnectionListener): TaskHandle = FakeHandle().also { handles += it }

    override suspend fun subscribeRoomList(listener: RoomListListener): TaskHandle {
        roomListListener = listener
        return FakeHandle().also { handles += it }
    }

    var forgotten = false

    override suspend fun forget() {
        forgotten = true
    }

    /** Called at the start of [logout], for the order of calls. */
    var onLogout: () -> Unit = {}

    override suspend fun logout() {
        onLogout()
        logoutError?.let { throw it }
        loggedOut = true
    }

    override suspend fun encryptionState(): EncryptionState {
        encryptionAsks++
        return encryption
    }

    // As the library: the state now, then after each change ([pushEncryption]).
    override fun subscribeEncryptionState(listener: EncryptionStateListener): TaskHandle {
        encryptionSubscriptions++
        encryptionListener = listener
        listener.onUpdate(encryption)
        return FakeHandle().also {
            handles += it
            encryptionSubscription = it
        }
    }

    /** The library reports a new encryption state. */
    fun pushEncryption(state: EncryptionState) {
        encryption = state
        encryptionListener?.onUpdate(state)
    }

    override suspend fun timeline(roomId: String): Timeline = roomTimeline

    override suspend fun userProfile(userId: String): UserProfile {
        profileGate?.await()
        profileError?.let { throw it }
        return profile ?: UserProfile(userId = userId, name = null, avatarUrl = null)
    }

    /** [sharedRooms] gives these, by user; [sharedError] fails it. */
    val shared = mutableMapOf<String, List<String>>()
    var sharedError: MtrxException? = null

    override suspend fun sharedRooms(userId: String): List<String> {
        sharedError?.let { throw it }
        return shared[userId].orEmpty()
    }

    override suspend fun invite(roomId: String, userId: String) {
        inviteGate?.await()
        inviteError?.let { throw it }
        invites += roomId to userId
    }

    override suspend fun recover(recoveryKey: String, listener: RecoveryListener) {
        recoveryKeys += recoveryKey
        for (step in recoverySteps) {
            listener.onUpdate(step)
            onRecoveryStep()
        }
        recoveryError?.let { throw it }
    }

    override suspend fun devices(): List<DeviceInfo> =
        listOf(DeviceInfo(id = "PHONE", name = "Mtrx Android", verified = false, current = true))

    override suspend fun requestSelfVerification(): Verification = selfVerification

    override fun subscribeVerificationRequests(listener: VerificationRequestListener): TaskHandle {
        verificationRequests = listener
        return FakeHandle().also { handles += it }
    }

    // Rooms, their settings and people, and the directory: the calls made,
    // in order ("join !a:x", "level @b:x 50"); each change fails with
    // [roomError] if set, after [roomGate] if set.
    val roomCalls = mutableListOf<String>()
    var roomError: MtrxException? = null
    var roomGate: CompletableDeferred<Unit>? = null
    var settings: RoomSettings = roomSettings()
    var settingsReads = 0
    var memberList: List<Member> = emptyList()
    var pinnedRoomTimeline: FakeTimeline = FakeTimeline()
    var pinnedOpened = 0

    /** A space's rooms by space ID; [childrenError] fails the call, after [childrenGate] if set. */
    val children = mutableMapOf<String, List<DirectoryRoom>>()
    var childrenError: MtrxException? = null
    var childrenGate: CompletableDeferred<Unit>? = null

    /** Directory pages by listOf(server, query, kind, page); empty without. */
    val directory = mutableMapOf<List<Any?>, DirectoryPage>()
    var directoryError: MtrxException? = null
    val previews = mutableMapOf<String, DirectoryRoom>()
    val browsedServers = mutableListOf<String>()
    var ownersOffered = false
    val created = mutableListOf<NewRoom>()

    private suspend fun call(record: String) {
        roomCalls += record
        roomGate?.await()
        roomError?.let { throw it }
    }

    override suspend fun roomSettings(roomId: String): RoomSettings {
        settingsReads++
        return settings
    }

    override suspend fun pinnedTimeline(roomId: String): Timeline {
        pinnedOpened++
        return pinnedRoomTimeline
    }

    override suspend fun setPinned(roomId: String, eventId: String, pin: Boolean) = call("${if (pin) "pin" else "unpin"} $eventId")

    override suspend fun setRoomTopic(roomId: String, topic: String) = call("topic $topic")

    override suspend fun setRoomName(roomId: String, name: String) = call("name $name")

    override suspend fun setNotifyMode(roomId: String, mode: NotifyMode) = call("notify $mode")

    override suspend fun setRoomAccess(roomId: String, access: Access) = call("access $access")

    override suspend fun setListed(roomId: String, listed: Boolean) = call("listed $listed")

    override suspend fun setHistory(roomId: String, history: History) = call("history $history")

    override suspend fun setPermissions(roomId: String, permissions: Permissions) = call("permissions $permissions")

    override suspend fun encryptRoom(roomId: String) = call("encrypt")

    override suspend fun setAddress(roomId: String, name: String?) = call("address $name")

    override suspend fun setSuggested(spaceId: String, roomId: String, suggested: Boolean) = call("suggested $spaceId $suggested")

    override suspend fun addToSpace(spaceId: String, roomId: String) = call("add $spaceId")

    override suspend fun removeFromSpace(spaceId: String, roomId: String) = call("take out $spaceId")

    override suspend fun members(roomId: String): List<Member> = memberList

    override suspend fun setLevel(roomId: String, userId: String, level: Long) = call("level $userId $level")

    override suspend fun removeMember(roomId: String, userId: String, reason: String?, ban: Boolean) =
        call("${if (ban) "ban" else "remove"} $userId $reason")

    override suspend fun unban(roomId: String, userId: String) = call("unban $userId")

    override suspend fun spaceChildren(spaceId: String): List<DirectoryRoom> {
        childrenGate?.await()
        childrenError?.let { throw it }
        return children[spaceId].orEmpty()
    }

    override suspend fun publicRooms(server: String?, query: String, kind: DirectoryKind, page: String?): DirectoryPage {
        roomCalls += "directory $server '$query' $kind $page"
        directoryError?.let { throw it }
        return directory[listOf(server, query, kind, page)] ?: DirectoryPage(rooms = emptyList(), next = null)
    }

    override suspend fun preview(address: Address): DirectoryRoom {
        roomCalls += "preview ${address.room}"
        directoryError?.let { throw it }
        return previews[address.room] ?: throw MtrxException.RoomNotFound(address.room)
    }

    override suspend fun join(room: String, via: List<String>): String {
        call("join $room")
        return room
    }

    override suspend fun knock(room: String, via: List<String>): String {
        call("knock $room")
        return room
    }

    override suspend fun leave(roomId: String) = call("leave $roomId")

    override fun directoryServers(): List<String> = browsedServers.toList()

    override suspend fun rememberDirectoryServer(server: String) {
        browsedServers.remove(server)
        browsedServers.add(0, server)
    }

    override suspend fun offersOwners(): Boolean = ownersOffered

    override suspend fun setNotesAtTop(on: Boolean) = call("notes at top $on")

    /** The drafts kept, by room (an empty one clears it), and each save; [draftError] fails the saves. */
    val drafts = mutableMapOf<String, Draft>()
    val draftSaves = mutableListOf<Pair<String, Draft>>()
    var draftError: MtrxException? = null

    override suspend fun draft(roomId: String): Draft? = drafts[roomId]

    override suspend fun saveDraft(roomId: String, draft: Draft) {
        draftSaves += roomId to draft
        draftError?.let { throw it }
        if (draft.text.isBlank() && draft.replyTo == null) drafts.remove(roomId) else drafts[roomId] = draft
    }

    override suspend fun createRoom(new: NewRoom): String {
        call("create ${new.name}")
        created += new
        return "!new:example.org"
    }

    companion object {
        const val USER_ID = "@me:example.org"
    }
}

/** A room's settings, all ours to change unless [may] is false. */
fun roomSettings(
    may: Boolean = true,
    access: Access = Access.INVITE,
    encrypted: Boolean = false,
    spaces: List<SpaceLink> = emptyList(),
    canAddTo: List<String> = emptyList(),
    banned: List<Banned> = emptyList(),
    address: String? = null,
    listed: Boolean? = false,
): RoomSettings = RoomSettings(
    canRename = may,
    canChangeTopic = may,
    canChangePicture = may,
    canInvite = may,
    notify = NotifyMode.ALL,
    access = access,
    canChangeAccess = may,
    canAddRooms = may,
    address = address,
    canChangeAddress = may,
    listed = listed,
    history = History.JOINED,
    canChangeHistory = may,
    permissions = Permissions.EVERYONE,
    canChangePermissions = may,
    encrypted = encrypted,
    canEncrypt = may && !encrypted,
    version = "12",
    spaces = spaces,
    canAddTo = canAddTo,
    banned = banned,
    canUnban = may,
)

/** A member at [level] (null for an owner), and what we may do to them; their label and level choices as the library gives them, without what it asks. */
fun member(
    userId: String,
    name: String? = null,
    level: Long? = 0,
    upTo: Long? = null,
    remove: Boolean = false,
    ban: Boolean = false,
    invited: Boolean = false,
    defaultLevel: Long = 0,
): Member = Member(
    userId = userId,
    name = name,
    avatarUrl = null,
    role = when (level) {
        100L -> Role.ADMIN
        50L -> Role.MODERATOR
        else -> Role.MEMBER
    },
    level = level,
    invited = invited,
    own = userId == FakeSession.USER_ID,
    manage = Manage(remove = remove, ban = ban, upTo = upTo, defaultLevel = defaultLevel, alone = false),
    label = when (level) {
        null -> "Owner"
        100L -> "Admin"
        50L -> "Mod"
        defaultLevel -> null
        else -> "Level $level"
    },
    choices = upTo?.let { upTo ->
        listOf("Admin" to 100L, "Moderator" to 50L, "Member" to defaultLevel)
            .filter { (_, at) -> at <= upTo || at == level }
            .map { (title, at) -> LevelChoice(title, at, current = at == level, confirmation = null) }
    }.orEmpty(),
    manageable = upTo != null || remove || ban,
)

/** A listed room or space. */
fun directoryRoom(
    id: String,
    name: String = id.removePrefix("!").substringBefore(':').replaceFirstChar { it.uppercase() },
    access: JoinAccess = JoinAccess.PUBLIC,
    membership: Membership? = null,
    members: Long = 3,
    topic: String? = null,
    space: Boolean = false,
    role: Role? = null,
): DirectoryRoom = DirectoryRoom(
    id = id,
    alias = null,
    name = name,
    topic = topic,
    avatarUrl = null,
    members = members.toULong(),
    isSpace = space,
    children = if (space) 4uL else 0uL,
    access = access,
    membership = membership,
    role = role,
    via = listOf("x"),
    suggested = false,
)

/** A verification with another device of [otherUser]; the test drives [listener]. */
class FakeVerification(private val otherUser: String) : Verification(NoHandle) {
    var listener: VerificationListener? = null
    var subscription: FakeHandle? = null
    var accepted = false
    var confirmed = false
    var mismatched = false
    var cancelled = false
    var current: VerificationState = VerificationState.Waiting

    override fun otherUser(): String = otherUser

    override fun state(): VerificationState = current

    override fun subscribe(listener: VerificationListener): TaskHandle {
        this.listener = listener
        listener.onUpdate(current)
        return FakeHandle().also { subscription = it }
    }

    /** The library reports [state]. */
    fun update(state: VerificationState) {
        current = state
        listener?.onUpdate(state)
    }

    override suspend fun accept() {
        accepted = true
    }

    override suspend fun confirm() {
        confirmed = true
        update(VerificationState.Confirmed)
    }

    override suspend fun mismatch() {
        mismatched = true
        update(VerificationState.Cancelled("The short authentication strings did not match"))
    }

    override suspend fun cancel() {
        cancelled = true
        update(VerificationState.Cancelled("The user cancelled the verification"))
    }
}

/** The app's preferences, in memory: one instance across "launches". */
class FakeSpaceStore(var saved: SpaceEntry? = null) : SelectedSpaceStore {
    override suspend fun load(): SpaceEntry? = saved

    override suspend fun save(entry: SpaceEntry?) {
        saved = entry
    }
}

/** The sections' arrangement, in memory: one instance across "launches". */
class FakeSectionStore(var saved: Map<String, SectionPrefs>? = null) : SectionStore {
    override suspend fun load(): Map<String, SectionPrefs>? = saved

    override suspend fun save(prefs: Map<String, SectionPrefs>) {
        saved = prefs
    }
}

/** The order of the spaces' rooms, in memory: one instance across "launches". */
class FakeSortStore(var saved: RoomSort? = null) : RoomSortStore {
    /** When set, [load] reads the saved order, then waits for it to return. */
    var loading: CompletableDeferred<Unit>? = null

    override suspend fun load(): RoomSort? {
        val read = saved
        loading?.await()
        return read
    }

    override suspend fun save(sort: RoomSort) {
        saved = sort
    }
}

fun makePresence(
    state: PresenceState,
    lastActive: Instant? = null,
    statusMessage: String? = null,
): Presence = Presence(state = state, lastActive = lastActive, currentlyActive = state == PresenceState.ONLINE, statusMessage = statusMessage)

fun summary(
    id: String,
    kind: RoomKind = RoomKind.ROOM,
    invited: Boolean = false,
    unread: Long = 0,
    directUserId: String? = null,
    name: String = id.removePrefix("!").substringBefore(':'),
    lastActivity: Instant? = null,
    topic: String? = null,
    parents: List<String> = emptyList(),
    pinned: List<String> = emptyList(),
    mentions: Long = 0,
): RoomSummary = RoomSummary(
    id = id,
    name = name,
    avatarUrl = null,
    topic = topic,
    kind = kind,
    invited = invited,
    encrypted = false,
    unreadMessages = unread.toULong(),
    unreadMentions = mentions.toULong(),
    unreadNotifications = unread.toULong(),
    lastActivity = lastActivity,
    directUserId = directUserId,
    alias = null,
    joinedMembers = 2uL,
    parents = parents,
    pinned = pinned,
)

class FakeMtrx(private val restored: Session?) : MtrxInterface {
    var loginSession: Session? = null

    /** What [notification] returns, or throws [notificationError]; after [notificationGate] if set. */
    var notificationContent: NotificationContent? = null
    var notificationError: MtrxException? = null
    var notificationGate: CompletableDeferred<Unit>? = null
    val notificationsRead = mutableListOf<Pair<String, String>>()

    override suspend fun notification(roomId: String, eventId: String): NotificationContent? {
        notificationsRead += roomId to eventId
        notificationGate?.await()
        notificationError?.let { throw it }
        return notificationContent
    }

    override suspend fun login(homeserver: String, user: String, password: String): Session =
        loginSession ?: throw MtrxException.Other("wrong password")

    override suspend fun restore(): Session? = restored

    override suspend fun startOauthLogin(homeserver: String): OAuthLogin = throw MtrxException.Other("no OAuth")

    override suspend fun supportsOauth(homeserver: String): Boolean = false
}

/**
 * A room's timeline: the test drives [listener] and [typingListener]; the
 * actions are recorded. [paginateBack] calls [onPage] with the call's number,
 * then reports the start at call [pagesUntilStart]; [gate], when set, holds
 * the sends and actions until completed.
 */
class FakeTimeline : Timeline(NoHandle) {
    var listener: TimelineRowsListener? = null
    var typingListener: TypingListener? = null
    val typingSubscription = FakeHandle()
    val sent = mutableListOf<String>()
    var sendError: MtrxException? = null
    var actionError: MtrxException? = null
    var gate: CompletableDeferred<Unit>? = null
    var pages = 0
    var pagesUntilStart = 1
    var onPage: (Int) -> Unit = {}
    var markedRead = 0
    val replies = mutableListOf<Pair<String, String>>()
    val edits = mutableListOf<Pair<String, String>>()
    val reactions = mutableListOf<Pair<String, String>>()
    val deleted = mutableListOf<String>()
    val retried = mutableListOf<String>()
    val discarded = mutableListOf<String>()
    val typing = mutableListOf<Boolean>()
    var closed = false

    override fun roomId(): String = "!room:example.org"

    override fun subscribeRows(listener: TimelineRowsListener): TaskHandle {
        this.listener = listener
        return FakeHandle()
    }

    override fun subscribeTyping(listener: TypingListener): TaskHandle {
        typingListener = listener
        return typingSubscription
    }

    override suspend fun paginateBack(events: UShort): Boolean {
        pages++
        onPage(pages)
        return pages >= pagesUntilStart
    }

    private suspend fun action(record: () -> Unit) {
        gate?.await()
        actionError?.let { throw it }
        record()
    }

    override suspend fun sendText(text: String) {
        gate?.await()
        sendError?.let { throw it }
        sent += text
    }

    override suspend fun reply(itemId: String, text: String) = action { replies += itemId to text }

    override suspend fun edit(itemId: String, text: String) = action { edits += itemId to text }

    override suspend fun react(itemId: String, key: String) = action { reactions += itemId to key }

    override suspend fun delete(itemId: String) = action { deleted += itemId }

    override suspend fun retry(itemId: String) = action { retried += itemId }

    override suspend fun discard(itemId: String) = action { discarded += itemId }

    override suspend fun setTyping(typing: Boolean) {
        this.typing += typing
    }

    var markReadError: MtrxException? = null

    override suspend fun markRead() {
        markReadError?.let { throw it }
        markedRead++
    }

    override fun close() {
        closed = true
        super.close()
    }
}

fun textEvent(id: String, sender: String, body: String, own: Boolean = false, at: Long = 0): TimelineRow =
    messageEvent(id, sender, Message(kind = MessageKind.TEXT, body = body, edited = false, media = null), own = own, at = at)

/** A message's row; [eventId] null while not sent. */
fun messageEvent(
    id: String,
    sender: String,
    message: Message,
    own: Boolean = false,
    at: Long = 0,
    eventId: String? = "\$$id",
    sendState: SendState = SendState.Sent,
    editable: Boolean = false,
    readBy: List<Reader> = emptyList(),
): TimelineRow = eventItem(id, sender, EventContent.Message(message), own, at, eventId, sendState, editable, readBy)

fun eventItem(
    id: String,
    sender: String,
    content: EventContent,
    own: Boolean = false,
    at: Long = 0,
    eventId: String? = "\$$id",
    sendState: SendState = SendState.Sent,
    editable: Boolean = false,
    readBy: List<Reader> = emptyList(),
): TimelineRow = messageRow(
    id,
    TimelineEvent(
        eventId = eventId,
        senderId = sender,
        senderName = null,
        senderAvatarUrl = null,
        timestamp = Instant.ofEpochSecond(at),
        own = own,
        content = content,
        sendState = sendState,
        editable = editable,
        readBy = readBy,
    ),
)

/**
 * The row of [event] as the library gives it: the first of its group unless
 * said otherwise, with [actions] (else those of a message in a room where we
 * may not pin).
 */
fun messageRow(
    id: String,
    event: TimelineEvent,
    firstInGroup: Boolean = true,
    decrypting: Boolean = false,
    actions: List<MessageAction> = actionsOf(event),
): TimelineRow = TimelineRow(
    id,
    RowContent.Message(
        event,
        firstInGroup = firstInGroup,
        showSender = firstInGroup && !event.own,
        avatarColumn = !event.own,
        decrypting = decrypting,
        actions = actions,
    ),
)

/** What holding a message offers, as the library says. */
fun actionsOf(event: TimelineEvent): List<MessageAction> = buildList {
    val sent = event.eventId != null
    val message = (event.content as? EventContent.Message)?.message
    if (message != null) {
        if (sent) addAll(listOf(MessageAction.REACT, MessageAction.REPLY))
        if (message.body.isNotEmpty()) addAll(listOf(MessageAction.COPY, MessageAction.SELECT_TEXT))
    }
    if (event.editable) add(MessageAction.EDIT)
    if (event.own && sent && event.content !is EventContent.Redacted) add(MessageAction.DELETE)
    if (event.sendState is SendState.Failed) addAll(listOf(MessageAction.RETRY, MessageAction.DISCARD))
}

/**
 * The rooms by space as in the library's integration test, most recently
 * active first: spaces Team and Other; Lab and, through the nested space
 * Sub, Deep in Team; an invite to Offer in Team; a direct chat with Bob, a
 * member of Team; an invite from Carol to a direct chat, and to Party,
 * outside spaces with Loose; and the Notes to self, listed in every space.
 */
object SpacesFixture {
    const val TEAM = "!team:x"
    const val OTHER = "!other:x"
    const val SUB = "!sub:x"
    const val LAB = "!lab:x"
    const val DEEP = "!deep:x"
    const val LOOSE = "!loose:x"
    const val WITH_BOB = "!with-bob:x"
    const val OFFER = "!offer:x"
    const val WITH_CAROL = "!with-carol:x"
    const val PARTY = "!party:x"
    const val NOTES = "!notes:x"

    fun state(loaded: Boolean = true): RoomListState = RoomListState(
        rooms = listOf(
            summary(WITH_CAROL, RoomKind.DIRECT, invited = true, directUserId = "@carol:x"),
            summary(WITH_BOB, RoomKind.DIRECT, unread = 2, directUserId = "@bob:x"),
            summary(OFFER, invited = true),
            summary(LAB, unread = 1),
            summary(DEEP),
            summary(PARTY, invited = true),
            summary(NOTES, RoomKind.DIRECT, directUserId = FakeSession.USER_ID),
            summary(LOOSE),
            summary(SUB, RoomKind.SPACE),
            summary(OTHER, RoomKind.SPACE),
            summary(TEAM, RoomKind.SPACE),
        ),
        loaded = loaded,
        spaces = listOf(
            SpaceRooms(spaceId = OTHER, roomIds = listOf(NOTES)),
            SpaceRooms(spaceId = TEAM, roomIds = listOf(WITH_BOB, OFFER, LAB, DEEP, NOTES)),
        ),
        roomsOutsideSpaces = listOf(PARTY, LOOSE),
    )
}

/** The UnifiedPush connector: [installed] distributors, the saved one, and the calls made. */
class FakeDistributors(
    var installed: List<String> = listOf(NTFY),
    var saved: String? = null,
    var default: String? = null,
) : Distributors {
    var registrations = 0
    var unregistrations = 0

    /** Called at each [unregister], for the order of calls. */
    var onUnregister: () -> Unit = {}

    override fun installed(): List<String> = installed

    override fun saved(): String? = saved?.takeIf { it in installed }

    override fun default(): String? = default

    override fun save(packageName: String) {
        saved = packageName
    }

    override fun register() {
        registrations++
    }

    override fun unregister() {
        unregistrations++
        saved = null
        onUnregister()
    }

    override fun label(packageName: String): String = when (packageName) {
        NTFY -> "ntfy"
        else -> packageName.substringAfterLast('.')
    }

    companion object {
        const val NTFY = "io.heckel.ntfy"
        const val OTHER = "org.example.push"
    }
}

/** Push registration's preferences, in memory: one instance across "launches". */
class FakePushStore(var enabled: Boolean = true, var endpoint: String? = null) : PushStore {
    var permissionAsked = false

    override suspend fun enabled(): Boolean = enabled

    override suspend fun setEnabled(value: Boolean) {
        enabled = value
    }

    override suspend fun endpoint(): String? = endpoint

    override suspend fun setEndpoint(value: String?) {
        endpoint = value
    }

    override suspend fun permissionAsked(): Boolean = permissionAsked

    override suspend fun setPermissionAsked() {
        permissionAsked = true
    }
}

/** Push registration on fakes; the gateway is the endpoint's origin. */
fun pushRegistration(
    scope: CoroutineScope,
    distributors: FakeDistributors = FakeDistributors(),
    store: FakePushStore = FakePushStore(),
): PushRegistration = PushRegistration(
    distributors,
    store,
    object : GatewayDiscovery {
        override suspend fun gatewayFor(endpoint: String): String = originGateway(endpoint)!!
    },
    scope,
) { PusherDevice(name = "Pixel 9", lang = "en-GB") }

/** The posted notifications, by room: the contents, or "fallback". */
class FakeNotifier : MessageNotifier {
    val posted = mutableListOf<NotificationContent>()
    val fallbacks = mutableListOf<String>()
    val cleared = mutableListOf<String>()
    var clearedAll = 0

    override suspend fun post(content: NotificationContent) {
        posted += content
    }

    override fun postFallback(roomId: String) {
        fallbacks += roomId
    }

    override fun clearRoom(roomId: String) {
        cleared += roomId
    }

    override fun clearAll() {
        clearedAll++
    }
}

/** "Share my presence", in memory: one instance across "launches". */
class FakeSharePresenceStore(var saved: Boolean? = null) : SharePresenceStore {
    override suspend fun load(): Boolean? = saved

    override suspend fun save(on: Boolean) {
        saved = on
    }
}

/** The virtual time of [scope]'s test dispatcher, in milliseconds; 0 without one. */
@OptIn(ExperimentalCoroutinesApi::class)
fun testClock(scope: CoroutineScope): () -> Long {
    val scheduler = (scope.coroutineContext[ContinuationInterceptor] as? TestDispatcher)?.scheduler ?: return { 0L }
    return { scheduler.currentTime }
}

/** An account on fakes, with push registration and notifications that tests may look at; idling in virtual time. */
fun testAccount(
    mtrx: MtrxInterface,
    scope: CoroutineScope,
    spaces: SelectedSpaceStore = FakeSpaceStore(),
    sort: RoomSortStore = FakeSortStore(),
    push: PushRegistration = pushRegistration(scope),
    notifications: RoomNotifications = FakeNotifier(),
    sharePresence: SharePresenceStore = FakeSharePresenceStore(),
    sections: SectionStore = FakeSectionStore(),
    log: FakeLog = FakeLog(),
): Account = Account(mtrx, scope, spaces, sort, push, notifications, sharePresence, sections, testClock(scope), log)

/** What the models logged, as "tag: message". */
class FakeLog : Logger {
    val lines = mutableListOf<String>()

    override fun info(tag: String, message: String) {
        lines += "$tag: $message"
    }

    override fun warn(tag: String, message: String) {
        lines += "$tag: $message"
    }
}
