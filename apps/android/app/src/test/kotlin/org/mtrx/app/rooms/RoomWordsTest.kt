package org.mtrx.app.rooms

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mtrx.app.FakeSession
import org.mtrx.app.directoryRoom
import org.mtrx.app.roomLink
import org.mtrx.app.roomSettings
import org.mtrx.app.summary
import org.mtrx.app.timeline.roomHeader
import org.mtrx.app.topicOf
import uniffi.mtrx.JoinRule
import uniffi.mtrx.LinkDraft
import uniffi.mtrx.Role

// Topics, levels, settings and listed rooms, as the screens word them.
class RoomWordsTest {
    private val call = roomLink("https://meet.example.org/ice", title = "Weekly", passcode = "42")
    private val drive = roomLink("https://example.org/drive", name = "example.org")

    @Test
    fun theTopicLineIsThePreviewElseAnOffer() {
        assertEquals("Ice group", topicPreview(topicOf("\n Ice group \nmore", calls = listOf(call)), canEdit = false))
        assertEquals("Weekly · example.org", topicPreview(topicOf(calls = listOf(call), links = listOf(drive)), canEdit = false))
        assertEquals(ADD_A_TOPIC, topicPreview(NO_TOPIC, canEdit = true))
        assertNull(topicPreview(NO_TOPIC, canEdit = false))
    }

    @Test
    fun theHeaderHasTheListedTopicAsItIs() {
        val topic = topicOf("Ice", calls = listOf(call))
        assertEquals(topic, roomHeader("!ice:x", summary("!ice:x", topic = topic), FakeSession.USER_ID).topic)
        assertEquals(NO_TOPIC, roomHeader("!gone:x", null, FakeSession.USER_ID).topic)
    }

    @Test
    fun theHeaderJoinsOnlyWithACall() {
        assertNull(HeaderLinks.of(topicOf("About."), canEdit = true))

        val links = HeaderLinks.of(topicOf(links = listOf(drive)), canEdit = false)!!
        assertNull("links only: the menu, no Join", links.join)
        assertEquals(listOf("Links"), links.groups.map { it.title })
        assertFalse(links.canEdit)

        val daily = roomLink("https://zoom.us/j/1", name = "Zoom")
        val header = HeaderLinks.of(topicOf(calls = listOf(call, daily), links = listOf(drive)), canEdit = true)!!
        assertEquals("the first call is the room's", call, header.join)
        assertEquals("Join Weekly, passcode 42", header.joinLabel)
        assertEquals("calls first", listOf("Calls", "Links"), header.groups.map { it.title })
        assertEquals(listOf(listOf(call, daily), listOf(drive)), header.groups.map { it.links })
        assertTrue("Edit links… last", header.canEdit)
    }

    @Test
    fun theEditorComposesTheCallsFirstEachKindInItsOrder() {
        var draft = TopicDraft.of(topicOf("Ice", calls = listOf(call), links = listOf(drive))).copy(text = " Ice group ")
        draft = draft.add(call = true)
        draft = draft.change(call = true, index = 1, link = draft.calls[1].copy(url = "zoom.us/j/1"))
        // Dragged above the other: the room's call now.
        draft = draft.move(call = true, from = 1, to = 0)
        draft = draft.change(call = false, index = 0, link = draft.links[0].copy(passcode = "nope"))
        var sent: Pair<String, List<LinkDraft>>? = null
        assertEquals("composed", draft.compose { text, links -> "composed".also { sent = text to links } })
        val (text, links) = sent!!
        assertEquals(" Ice group ", text)
        assertEquals(listOf(true, true, false), links.map { it.call })
        assertEquals(listOf("zoom.us/j/1", "https://meet.example.org/ice", "https://example.org/drive"), links.map { it.url })
        assertEquals(listOf("", "42", ""), links.map { it.passcode })
    }

    @Test
    fun addressesThatAreNotWebAddressesAreCounted() {
        val linkUrl = { url: String -> url.takeIf { it.startsWith("https://") } }
        var draft = TopicDraft.of(topicOf(links = listOf(drive))).add(call = false)
        assertEquals("nothing typed in yet", 0, draft.dropped(linkUrl))
        draft = draft.change(call = false, index = 1, link = draft.links[1].copy(url = "mailto:ann@example.org"))
        assertEquals(1, draft.dropped(linkUrl))
        draft = draft.remove(call = false, index = 1)
        assertEquals(0, draft.dropped(linkUrl))
    }

    @Test
    fun settingsWords() {
        val settings = roomSettings(joinRule = JoinRule.PUBLIC, encrypted = true)
        assertEquals("Room · Public: anyone can join · 4 members · encrypted · room version 12", factsLine(false, settings, 4))
        assertEquals("Space · Private: by invitation · 1 member · room version 12", factsLine(true, roomSettings(), 1))
        assertTrue(showsListed(settings))
        assertFalse(showsListed(roomSettings(listed = null)))
        assertEquals("ice", addressName("#ice:example.org"))
        assertEquals("example.org", serverOf(FakeSession.USER_ID))
    }

    @Test
    fun listedRowsSayTheirSizeTopicAndRole() {
        assertEquals("3 members · Sea ice", rowDetails(directoryRoom("!a:x", topic = topicOf("\nSea ice\nmore"))))
        assertEquals("Space · 4 rooms", rowDetails(directoryRoom("!a:x", space = true)))
        assertEquals("Admin", roleTag(directoryRoom("!a:x", role = Role.ADMIN)))
        assertNull(roleTag(directoryRoom("!a:x", role = Role.MEMBER)))
        val rooms = listOf(directoryRoom("!ice:x"), directoryRoom("!sea:x", topic = topicOf("About ICE")), directoryRoom("!lab:x"))
        assertEquals(listOf("!ice:x", "!sea:x"), filtered(rooms, " ice").map { it.id })
    }

    @Test
    fun thePillShowsWhileUpdatingAShownListOrAfterNoAnswer() {
        fun pill(rooms: List<uniffi.mtrx.DirectoryRoom>?, updating: Boolean, failed: Boolean) =
            statusPill(OverviewRooms(rooms, updating, failed, emptySet(), { org.mtrx.app.joinButton(it) }))
        val list = listOf(directoryRoom("!a:x"))
        assertEquals(OverviewPill.UPDATING, pill(list, updating = true, failed = false))
        // The first listing shows a spinner instead.
        assertNull(pill(null, updating = true, failed = false))
        assertEquals(OverviewPill.NO_ANSWER, pill(list, updating = false, failed = true))
        assertEquals(OverviewPill.NO_ANSWER, pill(null, updating = false, failed = true))
        assertNull(pill(list, updating = false, failed = false))
    }

    @Test
    fun userIdsAreAtNameColonServer() {
        assertEquals("@b:x", userIdOf(" @b:x "))
        assertNull(userIdOf("bob"))
        assertNull(userIdOf("@b x:y"))
    }
}
