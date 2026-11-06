package org.mtrx.app.rooms

import uniffi.mtrx.Badges
import uniffi.mtrx.DirectoryRoom
import uniffi.mtrx.JoinButton
import uniffi.mtrx.ListSection
import uniffi.mtrx.Membership
import uniffi.mtrx.RoomKind
import uniffi.mtrx.RoomLists
import uniffi.mtrx.RoomListsInterface
import uniffi.mtrx.RoomSort
import uniffi.mtrx.RoomSummary
import uniffi.mtrx.SectionChoice
import uniffi.mtrx.SectionLayout
import uniffi.mtrx.SpaceEntry
import uniffi.mtrx.SpaceKey
import uniffi.mtrx.TreeEntry

// The lists of the Chats, Rooms and Spaces tabs come from the library
// (`RoomLists`); how they open and show is the app's.

/** No rooms: the lists before the library's first. */
object NoRooms : RoomListsInterface {
    override fun ancestors(roomId: String): List<RoomSummary> = emptyList()

    override fun badges() = Badges(0uL, 0uL, 0uL)

    override fun chatSpaces(chatId: String): List<String> = emptyList()

    override fun chats(): List<RoomSummary> = emptyList()

    override fun drafts(): List<String> = emptyList()

    override fun firstSpaceOf(roomId: String): RoomSummary? = null

    override fun joining(): List<String> = emptyList()

    override fun membership(roomId: String): Membership? = null

    /** As the library's empty lists give it: not a member. */
    override fun joinButton(room: DirectoryRoom): JoinButton = empty.joinButton(room)

    private val empty by lazy { RoomLists.empty() }

    override fun loaded() = false

    override fun notes(): RoomSummary? = null

    override fun notesAtTop() = true

    override fun parents(roomId: String): List<RoomSummary> = emptyList()

    override fun room(roomId: String): RoomSummary? = null

    override fun roomGroups(among: List<String>?, folded: List<String>): List<ListSection> = emptyList()

    override fun rooms(): List<RoomSummary> = emptyList()

    override fun search(key: SpaceKey?, filter: String): List<RoomSummary> = emptyList()

    override fun searchSpaces(filter: String): List<SpaceEntry> = emptyList()

    override fun sectionChoices(key: SpaceKey, layout: SectionLayout): List<SectionChoice> = emptyList()

    override fun space(key: SpaceKey): SpaceEntry? = null

    override fun spaceList(key: SpaceKey, layout: SectionLayout, sort: RoomSort, filter: String): List<ListSection> =
        emptyList()

    override fun spaces(): List<SpaceEntry> = emptyList()

    override fun topLevelSpaces(): List<RoomSummary> = emptyList()

    override fun tree(spaceId: String): List<TreeEntry> = emptyList()

    override fun unreadBesides(roomId: String) = 0uL
}

/**
 * A group of the Rooms tab (the library's [section]): Home's ([space]
 * null) or a top-level space's, with the space for its picture.
 */
data class RoomGroup(val section: ListSection, val space: RoomSummary?) {
    val key: String get() = section.key
    val name: String get() = section.title
    val rooms: List<RoomSummary> get() = section.rooms
    val folded: Boolean get() = section.folded
    val unread: Long get() = section.unread.toLong()
    val mentions: Boolean get() = section.mentions
}

/**
 * The Rooms tab's groups in [lists]: only of the rooms [among], if given
 * (a direct chat's rooms shared with the person); those whose key is in
 * [folded] folded.
 */
fun roomGroups(lists: RoomListsInterface, among: List<String>? = null, folded: List<String> = emptyList()): List<RoomGroup> =
    lists.roomGroups(among, folded).map { group -> RoomGroup(group, group.space?.let(lists::room)) }

/** An entry of the spaces page's name: a space's, or Home. */
val SpaceEntry.name: String get() = space?.name ?: HOME_NAME

const val HOME_NAME = "Home"

/**
 * The room open on the Rooms tab, [roomId], while it is listed there; also
 * before the list is loaded, and while it is coming (just joined or made
 * here, the library's [RoomListsInterface.joining]).
 */
fun shownRoom(lists: RoomListsInterface, roomId: String?): String? {
    if (roomId == null) return null
    val room = lists.room(roomId)
    return when {
        room != null -> roomId.takeIf { opensOnRoomsTab(room) }
        roomId in lists.joining() || !lists.loaded() -> roomId
        else -> null
    }
}

/** Rooms and invites open on the Rooms tab; direct chats on Chats. */
fun opensOnRoomsTab(room: RoomSummary): Boolean = room.kind != RoomKind.DIRECT && (room.kind != RoomKind.SPACE || room.membership == Membership.INVITED)

/** The unread count to show, and whether it counts mentions; null if none. */
fun unreadBadge(room: RoomSummary): Pair<Long, Boolean>? = when {
    room.unreadMentions > 0uL -> room.unreadMentions.toLong() to true
    room.unreadMessages > 0uL -> room.unreadMessages.toLong() to false
    else -> null
}
