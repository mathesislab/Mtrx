package org.mtrx.app.timeline

import java.time.Duration
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mtrx.app.FakeAudioOutput
import org.mtrx.app.FakeSession
import uniffi.mtrx.Media
import uniffi.mtrx.MediaKind
import uniffi.mtrx.MtrxException

@OptIn(ExperimentalCoroutinesApi::class)
class AudioPlayerTest {
    private val session = FakeSession().apply {
        audioFiles["a"] = "/cache/audio/a.wav"
        audioFiles["b"] = "/cache/audio/b.wav"
    }
    private val output = FakeAudioOutput()
    private val players = mutableListOf<FakePlayer>()

    // Unconfined: fetches run at once; the ticks wait on a scheduler that no test advances.
    private val scope = CoroutineScope(UnconfinedTestDispatcher())
    private val audio = AudioPlayer(session, scope, output) { FakePlayer().also { players += it } }

    private val a = voiceMessage("a")
    private val b = voiceMessage("b")

    private val playback: Playback get() = audio.playback.value

    @After
    fun stop() {
        scope.cancel()
    }

    @Test
    fun playingAnotherPausesTheFirstWhichKeepsItsPosition() {
        audio.toggle(a)
        val first = players.single()
        assertEquals("/cache/audio/a.wav", first.path)
        first.ready()
        assertEquals(PlayState.Playing, playback.stateOf(a))
        assertEquals(Duration.ofSeconds(42), playback.durationOf(a))
        assertTrue(output.held)

        first.position = 12_000
        audio.toggle(b)
        assertEquals(listOf("open", "start", "release"), first.calls)
        assertNull(playback.stateOf(a))
        assertEquals(Duration.ofSeconds(12), playback.positionOf(a))
        players[1].ready()
        assertEquals(PlayState.Playing, playback.stateOf(b))

        // Back to A: fetched again (at once, from the library's cache), from where it stopped.
        audio.toggle(a)
        val again = players[2]
        again.ready()
        assertEquals(listOf("open", "seek 12000", "start"), again.calls)
        assertEquals(PlayState.Playing, playback.stateOf(a))
        assertEquals(listOf("a", "b", "a"), session.audioFetches)
    }

    @Test
    fun theEndGoesBackToTheStart() {
        audio.toggle(a)
        val player = players.single()
        player.ready()
        player.end()
        assertEquals(PlayState.Paused, playback.stateOf(a))
        assertEquals(Duration.ZERO, playback.positionOf(a))
        assertEquals("seek 0", player.calls.last())
        assertFalse(output.held)

        // Again from the start, without fetching again.
        audio.toggle(a)
        assertEquals(PlayState.Playing, playback.stateOf(a))
        assertEquals("start", player.calls.last())
        assertEquals(listOf("a"), session.audioFetches)
    }

    @Test
    fun releaseForgetsThePositionsAndCancelsAFetch() {
        audio.seek(b, Duration.ofSeconds(20))
        audio.toggle(a)
        val player = players.single()
        player.ready()
        player.position = 5_000
        // No file: it waits.
        val slow = voiceMessage("slow")
        audio.toggle(slow)
        assertEquals(PlayState.Fetching(null), playback.stateOf(slow))
        assertEquals(Duration.ofSeconds(5), playback.positionOf(a))

        audio.release()
        assertEquals(listOf("slow"), session.audioCancelled)
        assertEquals(Playback(), playback)
        assertEquals("release", player.calls.last())
        assertFalse(output.held)
    }

    @Test
    fun seekingAMessageNotPlayingSetsWhereItStarts() {
        audio.seek(a, Duration.ofSeconds(20))
        assertNull(playback.stateOf(a))
        assertEquals(Duration.ofSeconds(20), playback.positionOf(a))
        assertTrue(session.audioFetches.isEmpty())

        audio.toggle(a)
        players.single().ready()
        assertEquals(listOf("open", "seek 20000", "start"), players.single().calls)
        assertEquals(Duration.ofSeconds(20), playback.positionOf(a))
    }

    @Test
    fun aFetchShowsHowFarAndAFailureWhy() {
        session.audioProgress = listOf(0.4f)
        val slow = voiceMessage("slow")
        audio.toggle(slow)
        assertEquals(PlayState.Fetching(0.4f), playback.stateOf(slow))
        // A tap cancels it.
        audio.toggle(slow)
        assertNull(playback.source)
        assertEquals(listOf("slow"), session.audioCancelled)

        val broken = voiceMessage("broken")
        session.audioErrors["broken"] = MtrxException.Other("This audio can't be played")
        audio.toggle(broken)
        assertEquals(PlayState.Failed("This audio can't be played"), playback.stateOf(broken))
        // A tap tries again; the player can't read it.
        session.audioErrors.clear()
        session.audioFiles["broken"] = "/cache/audio/broken.ogg"
        audio.toggle(broken)
        players.single().fail()
        assertEquals(PlayState.Failed(AudioPlayer.CANT_PLAY), playback.stateOf(broken))
        assertEquals("release", players.single().calls.last())
    }

    @Test
    fun anInterruptionPauses() {
        audio.toggle(a)
        val player = players.single()
        player.ready()
        player.position = 7_000
        // A call, or the headphones going.
        output.interrupt()
        assertEquals(PlayState.Paused, playback.stateOf(a))
        assertEquals(Duration.ofSeconds(7), playback.positionOf(a))
        assertEquals("pause", player.calls.last())
        assertFalse(output.held)

        // Refused while a call goes on: it stays paused.
        output.refuse = true
        audio.toggle(a)
        assertEquals(PlayState.Paused, playback.stateOf(a))
        assertEquals("pause", player.calls.last())
    }

    /** A voice message, 42 s long as its event says. */
    private fun voiceMessage(source: String) = Media(
        kind = MediaKind.AUDIO, filename = "Voice message.ogg", size = 24_000u, mimetype = "audio/ogg",
        width = null, height = null, source = source,
        duration = Duration.ofSeconds(42), durationLabel = "0:42", voice = true,
    )
}

/** A player the test drives, calling back as `MediaPlayer` would ([ready], [end], [fail]); its calls in order. */
private class FakePlayer : Player {
    val calls = mutableListOf<String>()
    var path: String? = null
    override var position: Long = 0
    override val duration: Long? = 42_000
    private var onReady: () -> Unit = {}
    private var onEnd: () -> Unit = {}
    private var onError: () -> Unit = {}

    override fun open(path: String, voice: Boolean, onReady: () -> Unit, onEnd: () -> Unit, onError: () -> Unit) {
        calls += "open"
        this.path = path
        this.onReady = onReady
        this.onEnd = onEnd
        this.onError = onError
    }

    override fun start() {
        calls += "start"
    }

    override fun pause() {
        calls += "pause"
    }

    override fun seekTo(ms: Long) {
        calls += "seek $ms"
        position = ms
    }

    override fun release() {
        calls += "release"
    }

    fun ready() = onReady()

    /** Plays to its end. */
    fun end() {
        position = duration ?: 0
        onEnd()
    }

    fun fail() = onError()
}
