package org.mtrx.app

import java.time.Instant
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.awaitCancellation
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
import org.mtrx.app.model.originGateway
import org.mtrx.app.push.MessageNotifier
import uniffi.mtrx.Address
import uniffi.mtrx.Banned
import uniffi.mtrx.ConnectionListener
import uniffi.mtrx.DirectoryKind
import uniffi.mtrx.DirectoryPage
import uniffi.mtrx.DirectoryRoom
import uniffi.mtrx.History
import uniffi.mtrx.JoinAction
import uniffi.mtrx.JoinButton
import uniffi.mtrx.JoinRule
import uniffi.mtrx.JoinRuleChoice
import uniffi.mtrx.LevelChoice
import uniffi.mtrx.Manage
import uniffi.mtrx.Member
import uniffi.mtrx.Membership
import uniffi.mtrx.NewRoom
import uniffi.mtrx.NotifyMode
import uniffi.mtrx.Permissions
import uniffi.mtrx.Rights
import uniffi.mtrx.Role
import uniffi.mtrx.RoomSettings
import uniffi.mtrx.SpaceLink
import uniffi.mtrx.ConnectionState
import uniffi.mtrx.DeviceInfo
import uniffi.mtrx.Draft
import uniffi.mtrx.DraftKeeper
import uniffi.mtrx.EncryptionState
import uniffi.mtrx.EncryptionStateListener
import uniffi.mtrx.EventContent
import uniffi.mtrx.Message
import uniffi.mtrx.MessageAction
import uniffi.mtrx.MessageKind
import uniffi.mtrx.MtrxException
import uniffi.mtrx.MtrxInterface
import uniffi.mtrx.NoHandle
import uniffi.mtrx.Notification
import uniffi.mtrx.OAuthLogin
import uniffi.mtrx.Presence
import uniffi.mtrx.PresenceActivity
import uniffi.mtrx.PresenceDot
import uniffi.mtrx.PresenceListener
import uniffi.mtrx.PresenceState
import uniffi.mtrx.PusherInfo
import uniffi.mtrx.RecoveryListener
import uniffi.mtrx.RecoveryStatus
import uniffi.mtrx.RecoveryStep
import uniffi.mtrx.Room
import uniffi.mtrx.RoomKind
import uniffi.mtrx.Badges
import uniffi.mtrx.ListSection
import uniffi.mtrx.RoomLists
import uniffi.mtrx.RoomListsListener
import uniffi.mtrx.RoomSort as LibrarySort
import uniffi.mtrx.RoomSummary
import uniffi.mtrx.RowContent
import uniffi.mtrx.SectionChoice
import uniffi.mtrx.SectionLayout
import uniffi.mtrx.SpaceEntry
import uniffi.mtrx.SpaceKey
import uniffi.mtrx.SendState
import uniffi.mtrx.Session
import uniffi.mtrx.TaskHandle
import uniffi.mtrx.Timeline
import uniffi.mtrx.TimelineEvent
import uniffi.mtrx.TimelineRow
import uniffi.mtrx.TimelineRowsListener
import uniffi.mtrx.TypingListener
import uniffi.mtrx.User
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
    var roomListListener: RoomListsListener? = null

    var encryption = EncryptionState(deviceVerified = false, recovery = RecoveryStatus.INCOMPLETE)
    var encryptionAsks = 0
    var encryptionListener: EncryptionStateListener? = null
    var encryptionSubscription: FakeHandle? = null
    var encryptionSubscriptions = 0

    /** What [timeline] opens. */
    var roomTimeline: Timeline = FakeTimeline()

    /** [userProfile] gives [profile], or throws [profileError]; after [profileGate] if set. */
    var profile: User? = null
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

    /** Sync and presence calls in order: "startSync", "stopSync", "presence foreground", "sharing true". */
    val calls = mutableListOf<String>()

    override suspend fun startSync() {
        syncStarts++
        calls += "startSync"
    }

    override suspend fun stopSync() {
        syncStops++
        calls += "stopSync"
    }

    /**
     * What the library was told for our presence, in order; [ownPresenceError] fails the calls, and
     * they wait for [ownPresenceGate].
     */
    val presenceCalls get() = calls.filter { it.startsWith("presence ") || it.startsWith("sharing ") }
    var ownPresenceError: MtrxException? = null
    var ownPresenceGate: CompletableDeferred<Unit>? = null

    override suspend fun presenceActivity(activity: PresenceActivity) {
        calls += "presence ${activity.name.lowercase()}"
        ownPresenceGate?.await()
        ownPresenceError?.let { throw it }
    }

    override suspend fun setPresenceSharing(on: Boolean) {
        calls += "sharing $on"
        ownPresenceGate?.await()
        ownPresenceError?.let { throw it }
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

    override suspend fun subscribeRoomLists(listener: RoomListsListener): TaskHandle {
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

    override suspend fun userProfile(userId: String): User {
        profileGate?.await()
        profileError?.let { throw it }
        return profile ?: user(userId)
    }

    /** [sharedRooms] gives these, by user; [sharedError] fails it. */
    val shared = mutableMapOf<String, List<String>>()
    var sharedError: MtrxException? = null

    override suspend fun sharedRooms(userId: String): List<String> {
        sharedError?.let { throw it }
        return shared[userId].orEmpty()
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

    suspend fun call(record: String) {
        roomCalls += record
        roomGate?.await()
        roomError?.let { throw it }
    }

    override fun room(roomId: String): Room = FakeRoom(this, roomId)

    override suspend fun pinnedTimeline(roomId: String): Timeline {
        pinnedOpened++
        return pinnedRoomTimeline
    }

    override suspend fun spaceChildren(spaceId: String): List<DirectoryRoom> {
        childrenGate?.await()
        childrenError?.let { throw it }
        return children[spaceId].orEmpty().also { listed[spaceId] = it }
    }

    /** Each space's rooms as [spaceChildren] last gave them, as the library keeps them. */
    val listed = mutableMapOf<String, List<DirectoryRoom>>()

    override fun listedSpaceChildren(spaceId: String): List<DirectoryRoom>? = listed[spaceId]

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

    override fun directoryServers(): List<String> = browsedServers.toList()

    override suspend fun rememberDirectoryServer(server: String) {
        browsedServers.remove(server)
        browsedServers.add(0, server)
    }

    override suspend fun offersOwners(): Boolean = ownersOffered

    override suspend fun setNotesAtTop(on: Boolean) = call("notes at top $on")

    /** The drafts kept, by room (an empty one clears it), each save, and each room's last keeper. */
    val drafts = mutableMapOf<String, Draft>()
    val draftSaves = mutableListOf<Pair<String, Draft>>()
    val keepers = mutableMapOf<String, FakeDraftKeeper>()

    override fun draftKeeper(roomId: String): DraftKeeper = FakeDraftKeeper(this, roomId).also { keepers[roomId] = it }

    /** Keep [draft] for [roomId], as the library does. */
    fun keepDraft(roomId: String, draft: Draft) {
        draftSaves += roomId to draft
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

/**
 * A room of [session]: it reads the session's settings and members, and
 * records its changes in [FakeSession.roomCalls] (invites in
 * [FakeSession.invites]).
 */
class FakeRoom(private val session: FakeSession, private val roomId: String) : Room(NoHandle) {
    override suspend fun settings(): RoomSettings {
        session.settingsReads++
        return session.settings
    }

    override suspend fun members(): List<Member> = session.memberList

    override suspend fun setPinned(eventId: String, pin: Boolean) = session.call("${if (pin) "pin" else "unpin"} $eventId")

    override suspend fun setTopic(topic: String) = session.call("topic $topic")

    override suspend fun setName(name: String) = session.call("name $name")

    override suspend fun setNotify(mode: NotifyMode) = session.call("notify $mode")

    override suspend fun setJoinRule(rule: JoinRule) = session.call("join rule $rule")

    override suspend fun setListed(listed: Boolean) = session.call("listed $listed")

    override suspend fun setHistory(history: History) = session.call("history $history")

    override suspend fun setPermissions(permissions: Permissions) = session.call("permissions $permissions")

    override suspend fun encrypt() = session.call("encrypt")

    override suspend fun setAddress(name: String?) = session.call("address $name")

    /** On a space's room, as the following two. */
    override suspend fun setSuggested(roomId: String, suggested: Boolean) =
        session.call("suggested ${this.roomId} $suggested")

    override suspend fun addChild(roomId: String) = session.call("add ${this.roomId}")

    override suspend fun removeChild(roomId: String) = session.call("take out ${this.roomId}")

    override suspend fun setLevel(userId: String, level: Long) = session.call("level $userId $level")

    override suspend fun remove(userId: String, ban: Boolean, reason: String?) =
        session.call("${if (ban) "ban" else "remove"} $userId $reason")

    override suspend fun unban(userId: String) = session.call("unban $userId")

    override suspend fun invite(userId: String) {
        session.inviteGate?.await()
        session.inviteError?.let { throw it }
        session.invites += roomId to userId
    }

    override suspend fun leave() = session.call("leave $roomId")
}

/** A join rule's title in the settings' menu, as the library words it. */
fun ruleTitle(rule: JoinRule): String = when (rule) {
    JoinRule.INVITE -> "Invited people"
    JoinRule.KNOCK -> "Ask to join"
    JoinRule.SPACE_MEMBERS -> "Space members"
    JoinRule.PUBLIC -> "Anyone"
}

/** A join rule as a room's facts say it, as the library words it. */
fun ruleFact(rule: JoinRule): String = when (rule) {
    JoinRule.INVITE -> "Private: by invitation"
    JoinRule.KNOCK -> "Private: people ask to join"
    JoinRule.SPACE_MEMBERS -> "Private: space members can join"
    JoinRule.PUBLIC -> "Public: anyone can join"
}

/** A listed room's button with our [membership] of it, as the library gives it. */
fun joinButton(room: DirectoryRoom, membership: Membership? = room.membership): JoinButton {
    val action = when (membership) {
        Membership.JOINED -> JoinAction.OPEN
        Membership.INVITED -> JoinAction.INVITED
        Membership.ASKED -> JoinAction.ASKED
        null -> when (room.joinRule) {
            JoinRule.PUBLIC, JoinRule.SPACE_MEMBERS -> JoinAction.JOIN
            JoinRule.KNOCK -> JoinAction.ASK_TO_JOIN
            JoinRule.INVITE -> JoinAction.INVITE_ONLY
        }
    }
    val title = when (action) {
        JoinAction.OPEN -> "Open"
        JoinAction.JOIN -> "Join"
        JoinAction.ASK_TO_JOIN -> "Ask to join"
        JoinAction.INVITED -> "Invited"
        JoinAction.ASKED -> "Asked"
        JoinAction.INVITE_ONLY -> "Invite only"
    }
    return JoinButton(action, title)
}

/** A room's settings, all ours to change unless [may] is false; Space members offered [inSpace] or when so. */
fun roomSettings(
    may: Boolean = true,
    joinRule: JoinRule = JoinRule.INVITE,
    inSpace: Boolean = false,
    encrypted: Boolean = false,
    spaces: List<SpaceLink> = emptyList(),
    canAddTo: List<String> = emptyList(),
    banned: List<Banned> = emptyList(),
    address: String? = null,
    listed: Boolean? = false,
): RoomSettings = RoomSettings(
    rights = rights(may),
    notify = NotifyMode.ALL,
    joinRule = joinRule,
    joinRuleFact = ruleFact(joinRule),
    joinRuleChoices = JoinRule.entries
        .filter { it != JoinRule.SPACE_MEMBERS || inSpace || joinRule == JoinRule.SPACE_MEMBERS }
        .map { JoinRuleChoice(it, ruleTitle(it), ruleFact(it)) },
    address = address,
    listed = listed,
    history = History.JOINED,
    permissions = Permissions.EVERYONE,
    encrypted = encrypted,
    version = "12",
    spaces = spaces,
    canAddTo = canAddTo,
    banned = banned,
)

/** Our rights in a room: everything, unless [may] is false, at a plain member's level 0. */
fun rights(may: Boolean = true): Rights = Rights(
    send = may,
    redactOthers = may,
    pin = may,
    mentionRoom = may,
    rename = may,
    changeTopic = may,
    changePicture = may,
    invite = may,
    kick = may,
    ban = may,
    changeJoinRule = may,
    changeHistory = may,
    changeAddress = may,
    changeLevels = may,
    encrypt = may,
    addRooms = may,
    setCallService = may,
    level = 0,
    defaultLevel = 0,
)

/** A member at [level] (null for an owner), and what we may do to them; their label and level choices as the library gives them, without what it asks. */
fun member(
    userId: String,
    name: String? = null,
    level: Long? = 0,
    upTo: Long? = null,
    remove: Boolean = false,
    ban: Boolean = false,
    membership: Membership = Membership.JOINED,
    defaultLevel: Long = 0,
): Member = Member(
    user = user(userId, name),
    role = when (level) {
        100L -> Role.ADMIN
        50L -> Role.MODERATOR
        else -> Role.MEMBER
    },
    level = level,
    membership = membership,
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
    joinRule: JoinRule = JoinRule.PUBLIC,
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
    joinRule = joinRule,
    joinRuleFact = ruleFact(joinRule),
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
class FakeSpaceStore(var saved: SpaceKey? = null) : SelectedSpaceStore {
    override suspend fun load(): SpaceKey? = saved

    override suspend fun save(entry: SpaceKey?) {
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

/** With its dot, as the library sends it (the line needs the library). */
fun makePresence(
    state: PresenceState,
    lastActive: Instant? = null,
    statusMessage: String? = null,
): Presence = Presence(
    state = state,
    lastActive = lastActive,
    currentlyActive = state == PresenceState.ONLINE,
    statusMessage = statusMessage,
    dot = when (state) {
        PresenceState.ONLINE -> PresenceDot.ONLINE
        PresenceState.UNAVAILABLE -> PresenceDot.IDLE
        PresenceState.OFFLINE -> null
    },
)

/** [userId], named [name] if given; shown as the library names them: by name, else the ID's localpart. */
fun user(userId: String, name: String? = null): User = User(
    userId = userId,
    name = name,
    avatarUrl = null,
    displayName = name ?: userId.removePrefix("@").substringBefore(':'),
)

fun summary(
    id: String,
    kind: RoomKind = RoomKind.ROOM,
    membership: Membership = Membership.JOINED,
    unread: Long = 0,
    directUserId: String? = null,
    name: String = id.removePrefix("!").substringBefore(':'),
    lastActivity: Instant? = null,
    topic: String? = null,
    pinned: List<String> = emptyList(),
    mentions: Long = 0,
    rights: Rights = org.mtrx.app.rights(),
): RoomSummary = RoomSummary(
    id = id,
    name = name,
    avatarUrl = null,
    topic = topic,
    kind = kind,
    membership = membership,
    encrypted = false,
    unreadMessages = unread.toULong(),
    unreadMentions = mentions.toULong(),
    unreadNotifications = unread.toULong(),
    lastActivity = lastActivity,
    directUser = directUserId?.let { user(it) },
    alias = null,
    joinedMembers = 2uL,
    joinRule = JoinRule.INVITE,
    joinRuleFact = ruleFact(JoinRule.INVITE),
    pinned = pinned,
    rights = rights,
)

class FakeMtrx(private val restored: Session?) : MtrxInterface {
    var loginSession: Session? = null

    /** What [notification] returns, or throws [notificationError]; after [notificationGate] if set. */
    var notificationContent: Notification? = null
    var notificationError: MtrxException? = null
    var notificationGate: CompletableDeferred<Unit>? = null
    val notificationsRead = mutableListOf<Pair<String, String>>()

    override suspend fun notification(roomId: String, eventId: String): Notification? {
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
 * A room's draft keeper over [FakeSession.drafts]: the one left there, once;
 * the draft as last written, kept on [flush]. When to save is the library's
 * rule, tested there.
 */
class FakeDraftKeeper(private val session: FakeSession, private val roomId: String) : DraftKeeper(NoHandle) {
    private var restored = false

    /** Each draft the model said is written, in order. */
    val changes = mutableListOf<Draft>()

    override suspend fun restore(): Draft? {
        if (restored) return null
        restored = true
        return session.drafts[roomId]
    }

    override fun changed(draft: Draft) {
        changes += draft
    }

    override suspend fun flush() {
        session.keepDraft(roomId, changes.lastOrNull() ?: return)
    }
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

    /** The row [seek] and [itemOf] find for each event ID; others are not found ([itemOf] waits until cancelled). */
    val found = mutableMapOf<String, String>()
    val seeks = mutableListOf<String>()
    var onSeek: (String) -> Unit = {}

    override suspend fun seek(eventId: String): String {
        seeks += eventId
        onSeek(eventId)
        return found[eventId] ?: throw MtrxException.EventNotFound()
    }

    override suspend fun itemOf(eventId: String): String = found[eventId] ?: awaitCancellation()

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
    readBy: List<User> = emptyList(),
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
    readBy: List<User> = emptyList(),
): TimelineRow = messageRow(
    id,
    TimelineEvent(
        eventId = eventId,
        sender = user(sender),
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
 * The library's room lists, canned: the answers a test needs, whatever it
 * asks (the rules are the library's, tested there). [sections] for any
 * space's list, [choices] its Show sheet's, [groups] the Rooms tab's.
 */
class FakeRoomLists(
    private val rooms: List<RoomSummary> = emptyList(),
    private val loaded: Boolean = true,
    private val chats: List<RoomSummary> = emptyList(),
    private val notes: RoomSummary? = null,
    private val spaces: List<SpaceEntry> = emptyList(),
    private val sections: List<ListSection> = emptyList(),
    private val choices: List<SectionChoice> = emptyList(),
    private val groups: List<ListSection> = emptyList(),
    private val chatSpaces: Map<String, List<String>> = emptyMap(),
    private val firstSpaces: Map<String, RoomSummary> = emptyMap(),
    private val drafts: List<String> = emptyList(),
    private val badges: Badges = Badges(0uL, 0uL, 0uL),
    private val notesAtTop: Boolean = true,
    private val joining: List<String> = emptyList(),
    /** Rooms asked to join (in no list). */
    private val asked: List<String> = emptyList(),
) : RoomLists(NoHandle) {
    override fun badges() = badges

    override fun chatSpaces(chatId: String) = chatSpaces[chatId].orEmpty()

    override fun chats() = chats

    override fun drafts() = drafts

    override fun firstSpaceOf(roomId: String) = firstSpaces[roomId]

    override fun joining() = joining

    override fun membership(roomId: String) = when (roomId) {
        in joining -> Membership.JOINED
        in asked -> Membership.ASKED
        else -> room(roomId)?.membership
    }

    override fun joinButton(room: DirectoryRoom) = org.mtrx.app.joinButton(room, membership(room.id))

    override fun loaded() = loaded

    override fun notes() = notes

    override fun notesAtTop() = notesAtTop

    override fun room(roomId: String) = rooms.firstOrNull { it.id == roomId }

    override fun roomGroups(among: List<String>?, folded: List<String>) = groups

    override fun rooms() = rooms

    override fun search(key: SpaceKey?, filter: String) = emptyList<RoomSummary>()

    override fun searchSpaces(filter: String) = spaces

    override fun sectionChoices(key: SpaceKey, layout: SectionLayout) = choices

    override fun space(key: SpaceKey) = spaces.firstOrNull { it.key == key }

    override fun spaceList(key: SpaceKey, layout: SectionLayout, sort: LibrarySort, filter: String) = sections

    override fun spaces() = spaces

    override fun topLevelSpaces() = spaces.mapNotNull { it.space }

    override fun unreadBesides(roomId: String) = 0uL
}

/** A section of a list, or a group of the Rooms tab ([space]). */
fun listSection(
    key: String,
    title: String,
    rooms: List<RoomSummary>,
    folded: Boolean = false,
    space: String? = null,
) = ListSection(
    key = key,
    title = title,
    rooms = rooms,
    folded = folded,
    unread = rooms.sumOf { it.unreadMessages },
    mentions = rooms.any { it.unreadMentions > 0uL },
    space = space,
)

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

    val rooms = listOf(
        summary(WITH_CAROL, RoomKind.DIRECT, Membership.INVITED, directUserId = "@carol:x"),
        summary(WITH_BOB, RoomKind.DIRECT, unread = 2, directUserId = "@bob:x"),
        summary(OFFER, membership = Membership.INVITED),
        summary(LAB, unread = 1),
        summary(DEEP),
        summary(PARTY, membership = Membership.INVITED),
        summary(NOTES, RoomKind.DIRECT, directUserId = FakeSession.USER_ID),
        summary(LOOSE),
        summary(SUB, RoomKind.SPACE),
        summary(OTHER, RoomKind.SPACE),
        summary(TEAM, RoomKind.SPACE),
    )

    fun room(id: String) = rooms.first { it.id == id }

    /** The lists as the library gives them: Other and Team, then Home; [drafts] and [joining] room IDs. */
    fun lists(loaded: Boolean = true, drafts: List<String> = emptyList(), joining: List<String> = emptyList()) = FakeRoomLists(
        rooms = rooms,
        loaded = loaded,
        chats = listOf(room(WITH_CAROL), room(WITH_BOB)),
        notes = room(NOTES),
        spaces = listOf(
            SpaceEntry(SpaceKey.Space(OTHER), room(OTHER), false),
            SpaceEntry(SpaceKey.Space(TEAM), room(TEAM), true),
            SpaceEntry(SpaceKey.Outside, null, false),
        ),
        drafts = drafts,
        joining = joining,
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
    val posted = mutableListOf<Notification>()
    val fallbacks = mutableListOf<String>()
    val cleared = mutableListOf<String>()
    var clearedAll = 0

    override suspend fun post(content: Notification) {
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

/** An account on fakes, with push registration and notifications that tests may look at. */
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
): Account = Account(mtrx, scope, spaces, sort, push, notifications, sharePresence, sections, log)

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
