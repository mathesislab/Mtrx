package org.mtrx.app.rooms

import uniffi.mtrx.JoinRule
import uniffi.mtrx.RoomSettings

// The words of Room info's settings, as the desktop's (crates/mtrx-app/src/info.rs);
// the choices' words are the library's (RoomSettings).

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
