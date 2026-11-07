package org.mtrx.app.rooms

import java.util.concurrent.atomic.AtomicLong
import uniffi.mtrx.LinkDraft
import uniffi.mtrx.RoomLink
import uniffi.mtrx.Topic

// A room's topic comes parsed from the library (`RoomSummary.topic`): its
// text, its calls (the first is the room's, which the header joins) and its
// other links, as the desktop shows and edits them.

/** No topic. */
val NO_TOPIC = Topic(text = "", calls = emptyList(), links = emptyList(), firstLine = null, preview = null)

/** The topic line under a room's name: the topic's preview, else "Add a topic" where we may edit it; null for none. */
fun topicPreview(topic: Topic, canEdit: Boolean): String? = topic.preview ?: ADD_A_TOPIC.takeIf { canEdit }

const val ADD_A_TOPIC = "Add a topic"

/** The topic has calls or links. */
val Topic.hasLinks: Boolean get() = calls.isNotEmpty() || links.isNotEmpty()

/** A group of a topic's links in the header's menu: the calls, or the other links. */
data class LinkGroup(val title: String, val call: Boolean, val links: List<RoomLink>)

/**
 * What the room header offers of a topic's calls and links: Join for the
 * room's call ([join]), and a menu with the calls, then the links
 * ([groups]), then Edit links… where we may edit the topic ([canEdit]).
 */
data class HeaderLinks(val join: RoomLink?, val groups: List<LinkGroup>, val canEdit: Boolean) {
    /** "Join Weekly, passcode 123". */
    val joinLabel: String? get() = join?.let { call -> "Join ${call.name}" + (call.passcode?.let { ", passcode $it" } ?: "") }

    companion object {
        /** None without calls or links. */
        fun of(topic: Topic, canEdit: Boolean): HeaderLinks? {
            if (!topic.hasLinks) return null
            val groups = listOf(LinkGroup("Calls", true, topic.calls), LinkGroup("Links", false, topic.links))
            return HeaderLinks(topic.calls.firstOrNull(), groups.filter { it.links.isNotEmpty() }, canEdit)
        }
    }
}

private val linkIds = AtomicLong()

/**
 * The topic editor's text, calls and links as typed, each kind in its
 * order: the first call is the room's.
 */
data class TopicDraft(val text: String = "", val calls: List<Link> = emptyList(), val links: List<Link> = emptyList()) {
    /** A call or link as typed; [id] keeps it apart while it is dragged. */
    data class Link(
        val title: String = "",
        val url: String = "",
        val passcode: String = "",
        val id: Long = linkIds.incrementAndGet(),
    ) {
        /** Nothing typed in: left out when saved, without a word. */
        fun blank(call: Boolean): Boolean = listOf(title, url, if (call) passcode else "").all { it.isBlank() }

        fun draft(call: Boolean) = LinkDraft(call = call, title = title, url = url, passcode = if (call) passcode else "")

        companion object {
            fun of(link: RoomLink) = Link(link.title.orEmpty(), link.url, link.passcode.orEmpty())
        }
    }

    /** The calls, or the other links. */
    fun kind(call: Boolean): List<Link> = if (call) calls else links

    private fun with(call: Boolean, change: (MutableList<Link>) -> Unit): TopicDraft {
        val changed = kind(call).toMutableList().also(change)
        return if (call) copy(calls = changed) else copy(links = changed)
    }

    fun add(call: Boolean) = with(call) { it.add(Link()) }

    fun change(call: Boolean, index: Int, link: Link) = with(call) { it[index] = link }

    fun remove(call: Boolean, index: Int) = with(call) { it.removeAt(index) }

    /** With the call or link at [from] moved to [to] among its kind. */
    fun move(call: Boolean, from: Int, to: Int) = with(call) { it.add(to, it.removeAt(from)) }

    /** Calls and links typed in without a web address (none from [linkUrl]): left out when saved. */
    fun dropped(linkUrl: (String) -> String?): Int =
        calls.count { !it.blank(call = true) && linkUrl(it.url) == null } +
            links.count { !it.blank(call = false) && linkUrl(it.url) == null }

    /** The topic to save, as the library composes it ([composeTopic]): calls first, each kind in its order. */
    fun compose(composeTopic: (String, List<LinkDraft>) -> String): String =
        composeTopic(text, calls.map { it.draft(call = true) } + links.map { it.draft(call = false) })

    companion object {
        fun of(topic: Topic) = TopicDraft(topic.text, topic.calls.map(Link::of), topic.links.map(Link::of))
    }
}
