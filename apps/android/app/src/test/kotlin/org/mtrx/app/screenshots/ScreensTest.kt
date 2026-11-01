package org.mtrx.app.screenshots

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import app.cash.paparazzi.DeviceConfig
import com.android.resources.NightMode
import app.cash.paparazzi.Paparazzi
import kotlinx.coroutines.flow.emptyFlow
import org.junit.Rule
import org.junit.Test
import org.mtrx.app.member
import org.mtrx.app.model.SpaceEntry
import org.mtrx.app.rooms.InviteContent
import org.mtrx.app.rooms.JoinContent
import org.mtrx.app.rooms.JoinResults
import org.mtrx.app.rooms.JoinState
import org.mtrx.app.rooms.MemberMenu
import org.mtrx.app.rooms.NewRoomContent
import org.mtrx.app.rooms.NewRoomForm
import org.mtrx.app.rooms.OverviewRooms
import org.mtrx.app.rooms.RoomInfoContent
import org.mtrx.app.rooms.SpaceOverviewContent
import org.mtrx.app.rooms.RoomGroup
import org.mtrx.app.rooms.chatsUnread
import org.mtrx.app.rooms.roomGroups
import org.mtrx.app.rooms.sharedSpaces
import org.mtrx.app.rooms.roomsUnread
import org.mtrx.app.rooms.SectionPrefs
import org.mtrx.app.rooms.chatList
import org.mtrx.app.rooms.sectionChoices
import org.mtrx.app.rooms.spaceList
import org.mtrx.app.ui.RoomListSettings
import org.mtrx.app.ui.SectionHeading
import org.mtrx.app.ui.ShowActions
import org.mtrx.app.ui.ShowSheetContent
import org.mtrx.app.rooms.spaceRows
import org.mtrx.app.rooms.spacesUnread
import org.mtrx.app.rooms.unreadElsewhere
import org.mtrx.app.model.RoomSort
import org.mtrx.app.timeline.RoomHeader
import org.mtrx.app.timeline.TimelineContent
import org.mtrx.app.timeline.TimelineState
import org.mtrx.app.timeline.shownRows
import org.mtrx.app.ui.ChatList
import org.mtrx.app.ui.HomeFrame
import org.mtrx.app.ui.HomeTab
import org.mtrx.app.ui.MtrxTheme
import org.mtrx.app.ui.RoomGroupsList
import org.mtrx.app.ui.SpaceList
import org.mtrx.app.ui.SpaceRoomsList
import org.mtrx.app.ui.SpaceTitle
import uniffi.mtrx.DirectoryKind
import uniffi.mtrx.Membership

/**
 * The key screens with fake data, as PNGs (recordPaparazziDebug; CI
 * uploads them as the android-screenshots artifact).
 */
open class ScreensTest {
    /** The system's text size. */
    protected open val fontScale: Float get() = 1f

    @get:Rule
    val paparazzi = Paparazzi(deviceConfig = DeviceConfig.PIXEL_5.copy(fontScale = fontScale))

    private val drafts = rooms.drafts.toSet()

    private fun snap(name: String, content: @Composable () -> Unit) {
        paparazzi.snapshot(name) { MtrxTheme { WithBack(content) } }
    }

    @Test
    fun chats() = snap("chats-notes-at-top") {
        val (notes, chats) = chatList(rooms)
        HomeFrame(HomeTab.Chats, badges, onTab = {}) {
            ChatList(notes, chats, loaded = true, images, today, onOpenRoom = {}, presenceOf = { null }, spaces = sharedSpaces(rooms), drafts = drafts)
        }
    }

    @Test
    fun settingsSwitch() = snap("settings-notes-at-top") {
        HomeFrame(HomeTab.Settings, badges, onTab = {}) {
            SectionHeading("Room list")
            RoomListSettings(notesAtTop = true, onNotesAtTop = {})
        }
    }

    @Test
    fun showSheet() = snap("show-sheet") {
        Surface(Modifier.fillMaxWidth()) {
            ShowSheetContent(
                "Ice team",
                sectionChoices(rooms, SpaceEntry.Space(TEAM), SectionPrefs(hidden = setOf("low_priority"))),
                ShowActions(setShown = { _, _ -> }, showAll = {}, hideAll = {}, move = { _, _ -> }),
            )
        }
    }

    @Test
    fun spaces() = snap("spaces") {
        HomeFrame(HomeTab.Spaces, badges, onTab = {}) {
            SpaceList(spaceRows(rooms), SpaceEntry.Space(TEAM), loaded = true, images, onSelect = {})
        }
    }

    // All sections: Invites, Direct messages, Favourites, a category, Rooms
    // folded with its unread count, Low priority; the Notes to self on top.
    @Test
    fun spaceRooms() = snap("space-sections") {
        HomeFrame(HomeTab.Spaces, badges, onTab = {}, title = { SpaceTitle("Ice team", onClick = {}) }) {
            SpaceRoomsList(
                spaceList(rooms, SpaceEntry.Space(TEAM), SectionPrefs(folded = setOf("rooms")), RoomSort.DEFAULT),
                loaded = true,
                images,
                today,
                onOpenRoom = {},
                onFold = {},
                drafts = drafts,
            )
        }
    }

    // Home, then the spaces, each newest first; Outreach folded, with its count.
    @Test
    fun roomsTab() = snap("rooms-groups") {
        HomeFrame(HomeTab.Rooms, badges, onTab = {}) {
            RoomGroupsList(roomGroups(rooms, folded = setOf(OUTREACH)), loaded = true, images, today, onOpenRoom = {}, onOpenSpace = {}, onFold = {}, drafts = drafts)
        }
    }

    @Test
    fun roomsTabEmpty() = snap("rooms-empty") {
        HomeFrame(HomeTab.Rooms, emptyMap(), onTab = {}) {
            RoomGroupsList(emptyList(), loaded = true, images, today, onOpenRoom = {}, onOpenSpace = {})
        }
    }

    @Test
    fun room() = snap("room") { Room() }

    @Test
    fun topicPanel() = snap("topic-panel") { Room(topicOpen = true) }

    @Test
    fun topicEditor() = snap("topic-editor") { Room(editingTopic = true) }

    // Coffee at Home, three rooms in Ice team, Papers again in Outreach.
    @Test
    fun sharedRooms() = snap("shared-rooms") { SharedRooms(roomGroups(rooms, only = setOf(ICE, SEA, PAPERS, LOOSE, CODE))) }

    @Test
    fun noSharedRooms() = snap("shared-rooms-none") { SharedRooms(emptyList()) }

    @Test
    fun memberMenu() = snap("member-menu") {
        Surface(Modifier.fillMaxWidth()) {
            MemberMenu(member("@bob:example.org", "Bob Berg", level = 50, upTo = 100, remove = true, ban = true), own = false, onChoose = {})
        }
    }

    @Test
    fun ownMemberMenu() = snap("member-menu-own") {
        Surface(Modifier.fillMaxWidth()) {
            MemberMenu(member(ME, "Me", level = 100, upTo = 99), own = true, onChoose = {})
        }
    }

    @Test
    fun spaceOverview() = snap("space-overview") {
        SpaceOverviewContent(
            teamHeader,
            OverviewRooms(teamChildren, updating = true, failed = false, pending = emptySet(), joined = setOf(ICE)),
            iceSettings,
            iceMembers,
            busy = null,
            error = null,
            ownUserId = ME,
            spaceNames = mapOf(TEAM to "Ice team", OUTREACH to "Outreach"),
            images = images,
            rowActions = noRowActions,
            infoActions = NoRoomInfoActions,
            onBack = {},
            onLeave = {},
            onOpenLink = {},
        )
    }

    // A list long enough to scroll, at its end: the last row clear of the pill.
    @Composable
    private fun OverviewAtEnd(updating: Boolean, failed: Boolean) {
        val long = teamChildren + (1..12).map { org.mtrx.app.directoryRoom("!extra$it:example.org", "Project $it", members = it.toLong()) } +
            org.mtrx.app.directoryRoom("!last:example.org", "Last room", members = 2, topic = "The list's last row")
        SpaceOverviewContent(
            teamHeader,
            OverviewRooms(long, updating = updating, failed = failed, pending = emptySet(), joined = setOf(ICE)),
            iceSettings,
            iceMembers,
            busy = null,
            error = null,
            ownUserId = ME,
            spaceNames = mapOf(TEAM to "Ice team", OUTREACH to "Outreach"),
            images = images,
            rowActions = noRowActions,
            infoActions = NoRoomInfoActions,
            onBack = {},
            onLeave = {},
            onOpenLink = {},
            // Past the end: the list stops at its last item.
            listState = rememberLazyListState(initialFirstVisibleItemIndex = long.size + 10),
        )
    }

    @Test
    fun spaceOverviewUpdatingAtEnd() = snap("space-overview-updating-end") { OverviewAtEnd(updating = true, failed = false) }

    @Test
    fun spaceOverviewNoAnswerAtEnd() = snap("space-overview-no-answer-end") { OverviewAtEnd(updating = false, failed = true) }

    @Test
    fun join() = snap("join") {
        JoinContent(
            JoinState(
                ownServer = "example.org",
                servers = listOf("matrix.org", "uni-hamburg.de"),
                server = null,
                query = "ice",
                kind = DirectoryKind.ALL,
                results = JoinResults.Listed(
                    listOf(
                        org.mtrx.app.directoryRoom("!glacio:example.org", "Glaciology", members = 240, topic = "Everything about glaciers"),
                        org.mtrx.app.directoryRoom("!ice:example.org", "Ice sheets", membership = Membership.JOINED, members = 12),
                        org.mtrx.app.directoryRoom("!polar:example.org", "Polar science", space = true, members = 80),
                        org.mtrx.app.directoryRoom("!icecore:example.org", "Ice cores", access = uniffi.mtrx.JoinAccess.KNOCK, members = 18),
                    ),
                    next = "2",
                ),
                pending = emptySet(),
                joined = setOf(ICE),
            ),
            images,
            noJoinActions,
            onBack = {},
        )
    }

    @Test
    fun newRoom() = snap("new-room") {
        NewRoomContent(
            space = false,
            parentName = "Ice team",
            form = NewRoomForm(
                name = "Field work 2027",
                topic = "Planning the Greenland traverse",
                invite = listOf("@ana:example.org", "@bob:example.org"),
                coOwners = setOf("@ana:example.org"),
            ),
            offersOwners = true,
            creating = false,
            error = null,
            actions = noNewRoomActions,
            onBack = {},
        )
    }

    @Test
    fun invite() = snap("invite") {
        InviteContent(rooms.rooms.first { it.id == WORKSHOP }.copy(topic = "Planning the 2027 ice sheet workshop"), images, busy = false, onAccept = {}, onDecline = {}, onBack = {})
    }
}

/** The same screens at the largest text size the system offers. */
class LargeTextScreensTest : ScreensTest() {
    override val fontScale: Float get() = 2f
}

/** Room info is long: on a tall screen, to show its settings and members. */
open class TallScreensTest {
    /** The system's text size. */
    protected open val fontScale: Float get() = 1f

    @get:Rule
    val paparazzi = Paparazzi(deviceConfig = DeviceConfig.PIXEL_5.copy(screenHeight = 2340 * 2, fontScale = fontScale))

    @Test
    fun roomInfo() {
        paparazzi.snapshot("room-info") {
            MtrxTheme {
                WithBack {
                    RoomInfoContent(
                        iceHeader,
                        iceSettings,
                        iceMembers,
                        busy = null,
                        error = null,
                        ownUserId = ME,
                        spaceNames = mapOf(TEAM to "Ice team", OUTREACH to "Outreach"),
                        images = images,
                        actions = NoRoomInfoActions,
                        onBack = {},
                        onLeave = {},
                        onOpenLink = {},
                    )
                }
            }
        }
    }
}

/** The tabs' unread counts. */
private val badges = mapOf(HomeTab.Chats to chatsUnread(rooms), HomeTab.Rooms to roomsUnread(rooms), HomeTab.Spaces to spacesUnread(rooms))

private val iceRoomHeader = RoomHeader(
    name = "Ice sheets",
    avatarUrl = null,
    seed = ICE,
    person = null,
    topic = iceTopic,
    pinned = listOf("\$p1", "\$p2"),
    space = rooms.rooms.first { it.id == TEAM },
)

/** The Ice sheets room, as on the Rooms tab. */
@Composable
private fun Room(topicOpen: Boolean = false, editingTopic: Boolean = false) {
    HomeFrame(HomeTab.Rooms, badges, onTab = {}, showTopBar = false) {
        TimelineContent(
            iceRoomHeader,
            TimelineState(
                rows = shownRows(conversation),
                draft = "Thanks Ana, looking at them now",
                typing = listOf("Bob Berg"),
                canEditTopic = true,
                pinned = shownRows(pinnedMessages).filterIsInstance<org.mtrx.app.timeline.EventRow>(),
            ),
            NoTimelineActions,
            images,
            presence = null,
            backCount = unreadElsewhere(rooms, except = ICE),
            scrollTo = emptyFlow(),
            onBack = {},
            onRoomInfo = {},
            onOpenLink = {},
            onOpenRoom = {},
            onOpenSpace = {},
            topicOpen = topicOpen,
            editingTopic = editingTopic,
        )
    }
}

/** A direct chat with Bob, its panel of the rooms shared with him open ([shared] by space). */
@Composable
private fun SharedRooms(shared: List<RoomGroup>) {
    HomeFrame(HomeTab.Chats, badges, onTab = {}, showTopBar = false) {
        TimelineContent(
            RoomHeader(name = "Bob Berg", avatarUrl = null, seed = bob, person = bob),
            TimelineState(rows = shownRows(conversation), shared = shared),
            NoTimelineActions,
            images,
            presence = null,
            backCount = unreadElsewhere(rooms, except = "!with-bob:example.org"),
            scrollTo = emptyFlow(),
            onBack = {},
            onRoomInfo = {},
            onOpenLink = {},
            onOpenRoom = {},
            onOpenSpace = {},
            sharedOpen = true,
        )
    }
}

/** Room info at the largest text size. */
class LargeTextTallScreensTest : TallScreensTest() {
    override val fontScale: Float get() = 2f
}

/** Our bubbles in the dark theme. */
class DarkScreensTest {
    @get:Rule
    val paparazzi = Paparazzi(deviceConfig = DeviceConfig.PIXEL_5.copy(nightMode = NightMode.NIGHT))

    @Test
    fun room() {
        paparazzi.snapshot("room-dark") { MtrxTheme { WithBack { Room() } } }
    }
}
