package org.mtrx.app.screenshots

import androidx.activity.OnBackPressedDispatcher
import androidx.activity.OnBackPressedDispatcherOwner
import androidx.activity.compose.LocalOnBackPressedDispatcherOwner
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneOffset
import org.mtrx.app.FakeRoomLists
import org.mtrx.app.FakeSession
import org.mtrx.app.directoryRoom
import org.mtrx.app.listSection
import org.mtrx.app.member
import org.mtrx.app.messageRow
import org.mtrx.app.model.Images
import org.mtrx.app.rights
import org.mtrx.app.roomSettings
import org.mtrx.app.rooms.JoinActions
import org.mtrx.app.rooms.NewRoomActions
import org.mtrx.app.rooms.OverviewRowActions
import org.mtrx.app.rooms.RoomInfoActions
import org.mtrx.app.rooms.RoomInfoHeader
import org.mtrx.app.summary
import org.mtrx.app.user
import org.mtrx.app.timeline.FileUse
import org.mtrx.app.timeline.TimelineActions
import uniffi.mtrx.Badges
import uniffi.mtrx.Banned
import uniffi.mtrx.EventContent
import uniffi.mtrx.History
import uniffi.mtrx.JoinRule
import uniffi.mtrx.Membership
import uniffi.mtrx.Message
import uniffi.mtrx.Media
import uniffi.mtrx.MessageKind
import uniffi.mtrx.NotifyMode
import uniffi.mtrx.QuoteState
import uniffi.mtrx.Permissions
import uniffi.mtrx.Reaction
import uniffi.mtrx.ReplyQuote
import uniffi.mtrx.Role
import uniffi.mtrx.RoomKind
import uniffi.mtrx.RoomLink
import uniffi.mtrx.SendState
import uniffi.mtrx.SpaceLink
import uniffi.mtrx.SectionChoice
import uniffi.mtrx.SpaceEntry
import uniffi.mtrx.SpaceKey
import uniffi.mtrx.TimelineEvent
import uniffi.mtrx.TimelineRow
import uniffi.mtrx.Topic
import uniffi.mtrx.User

// Fake data for the screenshot tests: a team working on ice sheets. No
// call reaches the native library (Images never loads without a picture).

/** Avatars and previews, never loaded: the fakes have no pictures. */
val images = Images(FakeSession())

val today: LocalDate = LocalDate.of(2026, 10, 28)

fun at(hour: Int, minute: Int = 0, daysAgo: Long = 0): Instant =
    today.minusDays(daysAgo).atTime(hour, minute).toInstant(ZoneOffset.UTC)

const val ME = FakeSession.USER_ID
const val TEAM = "!team:example.org"
const val OUTREACH = "!outreach:example.org"
const val ICE = "!ice:example.org"
const val SEA = "!sea:example.org"
const val PAPERS = "!papers:example.org"
const val TEACHING = "!teaching:example.org"
const val LOOSE = "!loose:example.org"
const val WORKSHOP = "!workshop:example.org"
const val RADAR = "!radar:example.org"
const val CODE = "!code:example.org"
const val ARCHIVE = "!archive:example.org"
const val SUMMIT = "!summit:example.org"

val iceTopic = Topic(
    text = "Ice sheet modelling: runs, papers and the weekly call.\nNotes from each call go to the drive.",
    links = listOf(
        RoomLink(call = true, title = "Weekly", url = "https://meet.example.org/ice", passcode = "4242", name = "Weekly"),
        RoomLink(call = false, title = "Drive", url = "https://drive.example.org/ice", passcode = null, name = "Drive"),
    ),
)

/** The rooms, most recently active first. */
val roomList = listOf(
    summary("!with-bob:example.org", RoomKind.DIRECT, unread = 2, directUserId = "@bob:example.org", name = "Bob Berg", lastActivity = at(10, 42)),
    summary(ICE, unread = 3, name = "Ice sheets", lastActivity = at(10, 30), topic = iceTopic.text, pinned = listOf("\$p1", "\$p2")),
    summary("!with-carol:example.org", RoomKind.DIRECT, Membership.INVITED, directUserId = "@carol:example.org", name = "Carol Chen"),
    summary(SEA, name = "Sea ice", lastActivity = at(9, 5), topic = "Sea ice and the ocean"),
    summary(PAPERS, unread = 1, mentions = 1, name = "Papers", lastActivity = at(17, 20, daysAgo = 1)),
    summary(WORKSHOP, membership = Membership.INVITED, name = "Workshop 2027"),
    summary("!notes:example.org", RoomKind.DIRECT, directUserId = ME, name = "Notes to self", lastActivity = at(8, 0, daysAgo = 2)),
    summary(TEACHING, unread = 5, name = "Teaching", lastActivity = at(14, 0, daysAgo = 3)),
    summary(LOOSE, name = "Coffee", lastActivity = at(12, 0, daysAgo = 9)),
    summary(TEAM, RoomKind.SPACE, name = "Ice team"),
    summary(OUTREACH, RoomKind.SPACE, name = "Outreach"),
    summary("!with-dan:example.org", RoomKind.DIRECT, directUserId = "@dan:example.org", name = "Dan Dahl", lastActivity = at(16, 0, daysAgo = 4)),
    summary(RADAR, name = "Radar data", lastActivity = at(15, 10, daysAgo = 1), topic = "Radargrams from the traverse"),
    summary(CODE, unread = 4, name = "Model code", lastActivity = at(11, 0, daysAgo = 1)),
    summary(ARCHIVE, name = "Archive 2025", lastActivity = at(9, 0, daysAgo = 40)),
    summary(SUMMIT, membership = Membership.INVITED, name = "Ice summit 2027"),
)

fun listed(id: String) = roomList.first { it.id == id }

const val BOB_CHAT = "!with-bob:example.org"
const val DAN_CHAT = "!with-dan:example.org"
const val NOTES = "!notes:example.org"

/** Ice team's sections as the library gives them, Rooms folded. */
val teamSections = listOf(
    listSection("invites", "Invites", listOf(listed(SUMMIT))),
    listSection("direct_messages", "Direct messages", listOf(listed(BOB_CHAT), listed(DAN_CHAT))),
    listSection("favourites", "Favourites", listOf(listed(ICE))),
    listSection("category:field", "Field work", listOf(listed(SEA), listed(RADAR))),
    listSection("rooms", "Rooms", listOf(listed(PAPERS), listed(CODE)), folded = true),
    listSection("low_priority", "Low priority", listOf(listed(ARCHIVE))),
)

/**
 * The lists as the library gives them: the Notes to self at the top, Bob
 * in both spaces and Dan in Ice team; the Rooms tab's groups newest first,
 * Outreach folded.
 */
val rooms = FakeRoomLists(
    rooms = roomList,
    chats = listOf(listed(BOB_CHAT), listed("!with-carol:example.org"), listed(DAN_CHAT)),
    notes = listed(NOTES),
    spaces = listOf(
        SpaceEntry(SpaceKey.Space(TEAM), listed(TEAM), true),
        SpaceEntry(SpaceKey.Space(OUTREACH), listed(OUTREACH), true),
        SpaceEntry(SpaceKey.Outside, null, false),
    ),
    sections = teamSections,
    // Low priority hidden.
    choices = teamSections.map { SectionChoice(it.key, it.title, it.key != "low_priority", it.rooms.size.toULong()) },
    groups = listOf(
        listSection("home", "Home", listOf(listed(LOOSE), listed(WORKSHOP))),
        listSection(TEAM, "Ice team", listOf(ICE, SEA, PAPERS, RADAR, CODE, ARCHIVE, SUMMIT).map(::listed), space = TEAM),
        listSection(OUTREACH, "Outreach", listOf(listed(PAPERS), listed(TEACHING)), folded = true, space = OUTREACH),
    ),
    chatSpaces = mapOf(BOB_CHAT to listOf("Ice team", "Outreach"), DAN_CHAT to listOf("Ice team")),
    // A pencil on their rows: with and without unread messages.
    drafts = listOf(BOB_CHAT, DAN_CHAT, ICE, SEA, LOOSE),
    // Bob's 2; Ice sheets' 3, Papers' 1, Teaching's 5 and Model code's 4, all in spaces.
    badges = Badges(chats = 2uL, rooms = 13uL, spaces = 13uL),
)

/** The rooms shared with Bob: Coffee at Home, four rooms in Ice team, Papers again in Outreach. */
val sharedWithBob = FakeRoomLists(
    rooms = roomList,
    groups = listOf(
        listSection("home", "Home", listOf(listed(LOOSE))),
        listSection(TEAM, "Ice team", listOf(ICE, SEA, PAPERS, CODE).map(::listed), space = TEAM),
        listSection(OUTREACH, "Outreach", listOf(listed(PAPERS)), space = OUTREACH),
    ),
)

/** A message by [sender] ([name]) at [time]; the [first] of a group unless said otherwise. */
fun say(
    id: String,
    sender: String,
    name: String?,
    body: String,
    time: Instant,
    own: Boolean = false,
    reactions: List<Reaction> = emptyList(),
    readBy: List<User> = emptyList(),
    replyTo: ReplyQuote? = null,
    first: Boolean = true,
): TimelineRow = event(
    id,
    sender,
    name,
    time,
    own,
    EventContent.Message(Message(kind = MessageKind.TEXT, body = body, edited = false, media = null, reactions = reactions, replyTo = replyTo)),
    readBy,
    first,
)

/** The row of an event with [content] by [sender] ([name]) at [time]. */
fun event(
    id: String,
    sender: String,
    name: String?,
    time: Instant,
    own: Boolean,
    content: EventContent,
    readBy: List<User> = emptyList(),
    first: Boolean = true,
): TimelineRow = messageRow(
    id,
    TimelineEvent(
        eventId = "\$$id",
        sender = user(sender, name),
        timestamp = time,
        own = own,
        content = content,
        sendState = SendState.Sent,
        editable = own,
        readBy = readBy,
    ),
    firstInGroup = first,
)

/** A loaded quote of [sender]'s message [id]. */
fun quoting(id: String, sender: String, name: String, body: String) =
    ReplyQuote(eventId = "\$$id", state = QuoteState.LOADED, sender = user(sender, name), kind = MessageKind.TEXT, body = body)

val bob = "@bob:example.org"
val ana = "@ana:example.org"

val conversation = listOf(
    say("1", ana, "Ana Alves", "The new run with the finer grid finished overnight.", at(9, 12)),
    say("2", ana, "Ana Alves", "Grounding line retreat is about 20% slower than before.", at(9, 13), first = false),
    say(
        "3",
        bob,
        "Bob Berg",
        "Nice! Can you put the plots in https://drive.example.org/ice before the call?",
        at(9, 40),
        reactions = listOf(Reaction(key = "👍", count = 2uL, byUs = true, senders = emptyList())),
    ),
    say("4", ME, "Me", "I'll add them to the paper draft too: www.overleaf.com/project/ice", at(10, 2), own = true),
    say("5", ana, "Ana Alves", "Done, they're in the Figures folder.", at(10, 30)),
    // Ours: a reply with a reaction, and a deleted one.
    say(
        "6",
        ME,
        "Me",
        "Then the retreat is within the observed range.",
        at(10, 34),
        own = true,
        reactions = listOf(Reaction(key = "🎉", count = 1uL, byUs = false, senders = emptyList())),
        replyTo = quoting("2", ana, "Ana Alves", "Grounding line retreat is about 20% slower than before."),
    ),
    event("7", ME, "Me", at(10, 35), own = true, EventContent.Redacted, first = false),
)

val pinnedMessages = listOf(
    say("p1", bob, "Bob Berg", "Paper draft deadline: 15 November", at(9, 0, daysAgo = 6)),
    say("p2", ana, "Ana Alves", "Model setup notes are in the drive under Setup", at(11, 0, daysAgo = 12)),
)

val iceSettings = roomSettings(
    joinRule = JoinRule.SPACE_MEMBERS,
    encrypted = true,
    address = "#ice:example.org",
    listed = false,
    spaces = listOf(SpaceLink(spaceId = TEAM, suggested = true, canChange = true)),
    canAddTo = listOf(OUTREACH),
    banned = listOf(Banned(user = user("@spam:example.net", "Spammer"), reason = "Spam")),
).copy(notify = NotifyMode.MENTIONS, history = History.INVITED, permissions = Permissions.MODERATORS)

val iceMembers = listOf(
    member(ME, "Me", level = 100, upTo = 99),
    member("@ana:example.org", "Ana Alves", level = null),
    member(bob, "Bob Berg", level = 50, upTo = 100, remove = true, ban = true),
    member("@carol:example.org", "Carol Chen", level = 20, upTo = 100, remove = true, ban = true),
    member("@dan:example.org", "Dan Dahl", level = 0, upTo = 100, remove = true, ban = true),
    member("@eve:example.org", "Eve Ek", level = 0, membership = Membership.INVITED),
)

val iceHeader = RoomInfoHeader(roomId = ICE, name = "Ice sheets", avatarUrl = null, topic = iceTopic, space = false, rights = rights())
val teamHeader = RoomInfoHeader(
    roomId = TEAM,
    name = "Ice team",
    avatarUrl = null,
    topic = Topic("The ice sheet group: modelling, field work and papers.", emptyList()),
    space = true,
    rights = rights(),
)

val teamChildren = listOf(
    directoryRoom("!ice:example.org", "Ice sheets", membership = Membership.JOINED, members = 12, topic = iceTopic.text, role = Role.ADMIN),
    directoryRoom("!field:example.org", "Field work 2027", members = 7, topic = "Planning the Greenland traverse"),
    directoryRoom("!models:example.org", "Model development", joinRule = JoinRule.KNOCK, members = 5),
    directoryRoom("!review:example.org", "Reviews", joinRule = JoinRule.KNOCK, membership = Membership.ASKED, members = 3),
    directoryRoom("!board:example.org", "Board", joinRule = JoinRule.INVITE, members = 4),
    directoryRoom("!sub:example.org", "Students", space = true, members = 9, joinRule = JoinRule.SPACE_MEMBERS),
)

/** Actions that do nothing. */
object NoTimelineActions : TimelineActions {
    override fun onNearTop(value: Boolean) {}
    override fun markRead() {}
    override fun onDraft(value: String) {}
    override fun startReply(itemId: String) {}
    override fun startEdit(itemId: String) {}
    override fun cancelComposing() {}
    override fun send(): Boolean = false
    override fun react(itemId: String, key: String) {}
    override fun delete(itemId: String) {}
    override fun retry(itemId: String) {}
    override fun unsend(itemId: String) {}
    override fun goToOriginal(eventId: String) {}
    override fun showPersonInfo(userId: String) {}
    override fun setPinned(eventId: String, pin: Boolean) {}
    override fun file(media: Media, eventId: String?, use: FileUse) {}
    override fun cancelDownload(id: ULong) {}
}

object NoRoomInfoActions : RoomInfoActions {
    override fun rename(name: String) {}
    override fun setTopic(topic: Topic) {}
    override fun setNotify(mode: NotifyMode) {}
    override fun setJoinRule(rule: JoinRule) {}
    override fun setListed(listed: Boolean) {}
    override fun setSuggested(spaceId: String, suggested: Boolean) {}
    override fun setHistory(history: History) {}
    override fun setPermissions(permissions: Permissions) {}
    override fun encrypt() {}
    override fun setAddress(name: String) {}
    override fun addToSpace(spaceId: String) {}
    override fun removeFromSpace(spaceId: String) {}
    override fun unban(userId: String) {}
    override fun setLevel(userId: String, level: Long) {}
    override fun remove(userId: String, reason: String, ban: Boolean) {}
    override fun invite(userId: String) {}
    override fun clearError() {}
}

val noRowActions = OverviewRowActions(join = {}, knock = {}, open = {}, answer = {}, retry = {})

val noJoinActions = JoinActions(
    setServer = {},
    setQuery = {},
    setKind = {},
    retry = {},
    more = {},
    clearError = {},
    rows = noRowActions,
)

val noNewRoomActions = NewRoomActions(edit = {}, addPerson = { false }, removePerson = {}, setCoOwner = { _, _ -> }, create = {}, clearError = {})

/** Back presses go nowhere: the screens' BackHandlers need a dispatcher. */
@Composable
fun WithBack(content: @Composable () -> Unit) {
    val lifecycle: Lifecycle = LocalLifecycleOwner.current.lifecycle
    val owner = remember {
        object : OnBackPressedDispatcherOwner {
            override val lifecycle: Lifecycle = lifecycle
            override val onBackPressedDispatcher = OnBackPressedDispatcher()
        }
    }
    CompositionLocalProvider(LocalOnBackPressedDispatcherOwner provides owner, content = content)
}
