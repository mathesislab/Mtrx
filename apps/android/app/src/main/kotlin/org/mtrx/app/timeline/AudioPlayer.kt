package org.mtrx.app.timeline

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.media.MediaPlayer
import android.os.Handler
import android.os.Looper
import androidx.core.content.ContextCompat
import java.io.IOException
import java.time.Duration
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.job
import kotlinx.coroutines.launch
import org.mtrx.app.model.describe
import uniffi.mtrx.AudioProgress
import uniffi.mtrx.Media
import uniffi.mtrx.MtrxException
import uniffi.mtrx.Session

/**
 * A room's audio messages as played: the current one ([source]; playing,
 * paused, being fetched or failed) at [position] of [duration] (the
 * player's once loaded, else the event's), and where the others stopped
 * or were moved to ([kept], by source).
 */
data class Playback(
    val source: String? = null,
    val state: PlayState? = null,
    val position: Duration = Duration.ZERO,
    val duration: Duration? = null,
    val kept: Map<String, Duration> = emptyMap(),
) {
    /** What [media] is doing, if it is the current message. */
    fun stateOf(media: Media): PlayState? = if (media.source == source) state else null

    /** Where [media] is: the current message's position, else where it stopped or was moved to. */
    fun positionOf(media: Media): Duration = if (media.source == source) position else kept[media.source] ?: Duration.ZERO

    /** [media]'s length: the player's once loaded, else the event's. */
    fun durationOf(media: Media): Duration? = if (media.source == source) duration else media.duration
}

/** How the current audio message goes. */
sealed interface PlayState {
    /** Its file is fetched: how far, null while unknown or converting. */
    data class Fetching(val fraction: Float?) : PlayState

    data object Playing : PlayState

    data object Paused : PlayState

    /** It can't play: why. */
    data class Failed(val message: String) : PlayState
}

/**
 * Plays a message's file: [SystemPlayer] (`MediaPlayer`), behind an
 * interface so the tests use a fake. Called on the main thread, and calls
 * back on it; times in milliseconds.
 */
interface Player {
    /** Load [path] ([voice]: speech); then [onReady], or [onError] if it can't play; [onEnd] once played to its end. */
    fun open(path: String, voice: Boolean, onReady: () -> Unit, onEnd: () -> Unit, onError: () -> Unit)

    fun start()

    fun pause()

    fun seekTo(ms: Long)

    /** Where it is, once ready. */
    val position: Long

    /** The file's length, once ready; null if unknown. */
    val duration: Long?

    fun release()
}

/**
 * The device's side of playing ([SystemAudioOutput]): the audio focus and
 * the headphones, behind an interface so the JVM tests leave them out.
 * Called on the main thread.
 */
interface AudioOutput {
    /**
     * Playing starts: take the focus (other audio pauses) and watch the
     * headphones; losing the focus, or the headphones going, calls
     * [interrupted] on the main thread. False if the focus is refused (a call).
     */
    fun acquire(voice: Boolean, interrupted: () -> Unit): Boolean

    /** Playing stops: give the focus back. */
    fun release()
}

/**
 * A room's audio messages, played in their bubbles one at a time: the file
 * from the library ([Session.audioFile]), played by a [Player] from
 * [newPlayer] while [output] holds the focus. Starting one pauses the one
 * playing, which keeps its position; each keeps its position while the
 * room is open, and goes back to the start at its end. The position is
 * read ten times a second while playing. [release] when the room is left.
 * Methods are called on the main thread, [scope]'s.
 */
class AudioPlayer(
    private val session: Session,
    private val scope: CoroutineScope,
    private val output: AudioOutput,
    private val newPlayer: () -> Player,
) {
    private val _playback = MutableStateFlow(Playback())
    val playback: StateFlow<Playback> = _playback.asStateFlow()

    /** The current message's, once its file is here. */
    private var player: Player? = null

    /** The current message is a voice message. */
    private var voice = false

    /** The current message's fetch, and the ticks while it plays. */
    private var fetch: Job? = null
    private var ticks: Job? = null

    /** The play button: play, pause, resume, cancel the fetch or try again. */
    fun toggle(media: Media) {
        when (_playback.value.stateOf(media)) {
            PlayState.Playing -> pause()
            PlayState.Paused -> resume()
            is PlayState.Fetching -> leave()
            is PlayState.Failed, null -> start(media)
        }
    }

    /** Move [media] to [to]: the player, if it is the current message (it starts there if still fetched); else where it starts. */
    fun seek(media: Media, to: Duration) {
        val now = _playback.value
        if (media.source != now.source) {
            _playback.value = now.copy(kept = now.kept + (media.source to to))
            return
        }
        if (now.state == PlayState.Playing || now.state == PlayState.Paused) player?.seekTo(to.toMillis())
        _playback.value = now.copy(position = to)
    }

    /**
     * Pause the message playing, if any (the app goes to the background, a
     * call, the headphones go); a fetch is cancelled, so that it doesn't
     * start there.
     */
    fun pause() {
        val now = _playback.value
        when (now.state) {
            is PlayState.Fetching -> leave()
            PlayState.Playing -> {
                val player = player ?: return
                player.pause()
                stopTicks()
                output.release()
                _playback.value = now.copy(state = PlayState.Paused, position = Duration.ofMillis(player.position))
            }
            else -> {}
        }
    }

    /** Stop, and forget the positions (the room is left). */
    fun release() {
        leave()
        output.release()
        _playback.value = Playback()
    }

    /** Fetch [media]'s file and play it from its position, after letting go of the current message. */
    private fun start(media: Media) {
        leave()
        val before = _playback.value
        voice = media.voice
        _playback.value = Playback(
            source = media.source,
            state = PlayState.Fetching(null),
            position = before.kept[media.source] ?: Duration.ZERO,
            duration = media.duration,
            kept = before.kept - media.source,
        )
        // Started once it is `fetch`: it may end without suspending.
        val job = scope.launch(start = CoroutineStart.LAZY) {
            val me = coroutineContext.job
            val progress = object : AudioProgress {
                // On a library thread: applied on the main thread, while this fetch is the current one.
                override fun onProgress(fraction: Float?) {
                    scope.launch {
                        val now = _playback.value
                        if (fetch === me && now.state is PlayState.Fetching) _playback.value = now.copy(state = PlayState.Fetching(fraction))
                    }
                }
            }
            val path = try {
                session.audioFile(media, progress)
            } catch (e: MtrxException) {
                if (fetch === me) failed(describe(e) ?: CANT_PLAY)
                return@launch
            }
            if (fetch === me) open(path)
        }
        fetch = job
        job.start()
    }

    /** The current message's file is here: load it, then play it from its position. */
    private fun open(path: String) {
        fetch = null
        val player = newPlayer()
        this.player = player
        player.open(
            path,
            voice,
            onReady = { if (this.player === player) ready(player) },
            onEnd = { if (this.player === player) ended(player) },
            onError = { if (this.player === player) failed(CANT_PLAY) },
        )
    }

    private fun ready(player: Player) {
        val now = _playback.value
        if (now.position > Duration.ZERO) player.seekTo(now.position.toMillis())
        _playback.value = now.copy(duration = player.duration?.let(Duration::ofMillis) ?: now.duration)
        resume()
    }

    /** Play the current message's player from where it is, holding the focus; paused if it is refused. */
    private fun resume() {
        val player = player ?: return
        if (!output.acquire(voice, this::pause)) {
            _playback.value = _playback.value.copy(state = PlayState.Paused)
            return
        }
        player.start()
        _playback.value = _playback.value.copy(state = PlayState.Playing)
        ticks = scope.launch {
            while (true) {
                delay(TICK_MILLIS)
                _playback.value = _playback.value.copy(position = Duration.ofMillis(player.position))
            }
        }
    }

    /** Played to its end: back to the start, paused. */
    private fun ended(player: Player) {
        stopTicks()
        output.release()
        player.seekTo(0)
        _playback.value = _playback.value.copy(state = PlayState.Paused, position = Duration.ZERO)
    }

    /** The current message can't play, as [message] says; its position stays, to try again from. */
    private fun failed(message: String) {
        fetch = null
        stopTicks()
        output.release()
        player?.release()
        player = null
        _playback.value = _playback.value.copy(state = PlayState.Failed(message))
    }

    /** Let go of the current message, keeping its position: release its player, or cancel its fetch. */
    private fun leave() {
        val now = _playback.value
        val source = now.source ?: return
        val fetching = fetch
        fetch = null
        fetching?.cancel()
        stopTicks()
        output.release()
        val position = player?.takeIf { now.state == PlayState.Playing }?.let { Duration.ofMillis(it.position) } ?: now.position
        player?.release()
        player = null
        _playback.value = Playback(kept = if (position > Duration.ZERO) now.kept + (source to position) else now.kept - source)
    }

    private fun stopTicks() {
        ticks?.cancel()
        ticks = null
    }

    companion object {
        /** The position is read this often while playing. */
        const val TICK_MILLIS = 100L

        /** When the player can't read or decode the file. */
        const val CANT_PLAY = "Can't play this file"
    }
}

/** As media: speech for a [voice] message, else music. */
private fun audioAttributes(voice: Boolean): AudioAttributes = AudioAttributes.Builder()
    .setUsage(AudioAttributes.USAGE_MEDIA)
    .setContentType(if (voice) AudioAttributes.CONTENT_TYPE_SPEECH else AudioAttributes.CONTENT_TYPE_MUSIC)
    .build()

/** [Player] by the framework's [MediaPlayer]; made on the main thread, it calls back there. */
class SystemPlayer : Player {
    private val player = MediaPlayer()

    override fun open(path: String, voice: Boolean, onReady: () -> Unit, onEnd: () -> Unit, onError: () -> Unit) {
        player.setAudioAttributes(audioAttributes(voice))
        player.setOnPreparedListener { onReady() }
        player.setOnCompletionListener { onEnd() }
        // Handled: no completion follows.
        player.setOnErrorListener { _, _, _ ->
            onError()
            true
        }
        try {
            player.setDataSource(path)
            player.prepareAsync()
        } catch (e: IOException) {
            onError()
        } catch (e: IllegalArgumentException) {
            onError()
        }
    }

    override fun start() = player.start()

    override fun pause() = player.pause()

    override fun seekTo(ms: Long) = player.seekTo(ms, MediaPlayer.SEEK_CLOSEST)

    override val position: Long get() = player.currentPosition.toLong()

    override val duration: Long? get() = player.duration.takeIf { it > 0 }?.toLong()

    override fun release() = player.release()
}

/**
 * The audio focus, held while playing (transient: other audio pauses, and
 * may resume after), and the headphones going (`ACTION_AUDIO_BECOMING_NOISY`),
 * watched while playing. Every loss of the focus interrupts, ducking
 * included: speech under other audio is lost.
 */
class SystemAudioOutput(private val context: Context) : AudioOutput {
    private val audio = context.getSystemService(AudioManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private var focus: AudioFocusRequest? = null
    private var noisy: BroadcastReceiver? = null

    override fun acquire(voice: Boolean, interrupted: () -> Unit): Boolean {
        release()
        val listener = AudioManager.OnAudioFocusChangeListener { change ->
            when (change) {
                AudioManager.AUDIOFOCUS_LOSS,
                AudioManager.AUDIOFOCUS_LOSS_TRANSIENT,
                AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK,
                -> interrupted()
            }
        }
        val request = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
            .setAudioAttributes(audioAttributes(voice))
            .setWillPauseWhenDucked(true)
            .setOnAudioFocusChangeListener(listener, main)
            .build()
        if (audio.requestAudioFocus(request) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED) return false
        focus = request
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) = interrupted()
        }
        // A system broadcast: received although not exported.
        ContextCompat.registerReceiver(
            context,
            receiver,
            IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY),
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
        noisy = receiver
        return true
    }

    override fun release() {
        focus?.let { audio.abandonAudioFocusRequest(it) }
        focus = null
        noisy?.let { context.unregisterReceiver(it) }
        noisy = null
    }
}
