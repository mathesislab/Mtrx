package org.mtrx.app.timeline

/** What the composer's text becomes, besides a new message: a reply or an edit. */
sealed interface Composing {
    /**
     * The row replied to (its `TimelineRow.id`); its sender and first line
     * in the bar; its event's ID, kept with the draft.
     */
    data class Replying(val itemId: String, val name: String, val preview: String, val eventId: String) : Composing

    /**
     * The row edited, with its text as we wrote it ([original]) and the
     * composer's text before the edit started ([draft]), back after it.
     */
    data class Editing(val itemId: String, val original: String, val draft: String) : Composing
}

/** The line above the composer: who else is typing; empty for nobody. */
fun typingText(names: List<String>): String = when (names.size) {
    0 -> ""
    1 -> "${names[0]} is typing…"
    2 -> "${names[0]} and ${names[1]} are typing…"
    3 -> "${names[0]}, ${names[1]} and ${names[2]} are typing…"
    else -> "${names[0]}, ${names[1]} and ${names.size - 2} others are typing…"
}
