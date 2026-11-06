package org.mtrx.app.rooms

import uniffi.mtrx.History
import uniffi.mtrx.JoinRule
import uniffi.mtrx.NotifyMode
import uniffi.mtrx.Permissions
import uniffi.mtrx.RoomSettings

// The words of Room info's settings, as the desktop's (crates/mtrx-app/src/info.rs).

/** An option of a setting: its value, label and one-line explanation. */
data class Option<T>(val value: T, val label: String, val explanation: String)

val NOTIFY_OPTIONS = listOf(
    Option(NotifyMode.ALL, "All messages", "Every message notifies you."),
    Option(NotifyMode.MENTIONS, "Mentions", "Only mentions of you and your keywords notify you."),
    Option(NotifyMode.MUTE, "Mute", "Nothing here notifies you."),
)

val HISTORY_OPTIONS = listOf(
    Option(History.JOINED, "Since joining", "Members, from when they joined."),
    Option(History.INVITED, "Since invited", "Members, from when they were invited."),
    Option(History.MEMBERS, "Members", "Members, all of it, also from before they joined."),
    Option(History.ANYONE, "Anyone", "Anyone, without joining."),
)

val PERMISSION_OPTIONS = listOf(
    Option(Permissions.EVERYONE, "Everyone", "Everyone can invite, rename and change the topic."),
    Option(Permissions.MODERATORS, "Moderators", "Invite, rename, change the topic and picture, and pin messages."),
)

/** The label of [value] among [options]. */
fun <T> labelOf(options: List<Option<T>>, value: T): String = options.firstOrNull { it.value == value }?.label.orEmpty()

/** "Room · Public: anyone can join · 4 members · encrypted · room version 12". */
fun factsLine(space: Boolean, settings: RoomSettings, members: Int?): String = listOfNotNull(
    if (space) "Space" else "Room",
    settings.joinRuleFact,
    members?.let { if (it == 1) "1 member" else "$it members" },
    "encrypted".takeIf { settings.encrypted },
    settings.version?.let { "room version $it" },
).joinToString(" · ")

/** Listed in the directory shows only for a public or listed room. */
fun showsListed(settings: RoomSettings): Boolean = settings.joinRule == JoinRule.PUBLIC || settings.listed == true

/** The server of a user ID or room ID, after its first colon. */
fun serverOf(id: String): String = id.substringAfter(':', "")

/** The name part of an address `#name:server`. */
fun addressName(address: String?): String = address?.removePrefix("#")?.substringBefore(':').orEmpty()
