package org.mtrx.app.timeline

import android.content.Context
import android.graphics.Bitmap
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Build
import androidx.annotation.OptIn
import androidx.media3.common.MediaItem
import androidx.media3.common.Metadata
import androidx.media3.common.MimeTypes
import androidx.media3.common.util.UnstableApi
import androidx.media3.container.Mp4LocationData
import androidx.media3.effect.Presentation
import androidx.media3.muxer.Muxer
import androidx.media3.transformer.AudioEncoderSettings
import androidx.media3.transformer.Composition
import androidx.media3.transformer.DefaultEncoderFactory
import androidx.media3.transformer.DefaultMuxer
import androidx.media3.transformer.EditedMediaItem
import androidx.media3.transformer.Effects
import androidx.media3.transformer.ExportException
import androidx.media3.transformer.ExportResult
import androidx.media3.transformer.ProgressHolder
import androidx.media3.transformer.Transformer
import androidx.media3.transformer.VideoEncoderSettings
import java.io.File
import java.io.IOException
import kotlin.math.roundToInt
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import uniffi.mtrx.FileToSend
import uniffi.mtrx.MediaQuality
import uniffi.mtrx.Session
import uniffi.mtrx.VideoInfo
import uniffi.mtrx.VideoPlan
import uniffi.mtrx.videoThumbnailEdge

/**
 * A video as the library plans for it: shown [width]×[height] px (turned
 * upright), [fps] frames a second (0 if not known), [durationMs] long, its
 * file [size] bytes.
 */
data class VideoSource(val width: Int, val height: Int, val fps: Float, val durationMs: Long, val size: Long)

/**
 * [file], a video, ready to send (docs/reducing-media.md): converted to the
 * library's plan with Media3 Transformer when [Preparation.quality] is
 * reduced and the library plans so (the converted copy replaces it; how far
 * goes to [Preparation.progress]); with its still and length ([VideoInfo]),
 * so that apps show them before it is opened. A file without pictures goes
 * as it is.
 */
suspend fun prepareVideo(context: Context, file: FileToSend, preparation: Preparation): FileToSend {
    val source = withContext(Dispatchers.IO) { readVideo(File(file.path)) } ?: return file
    val plan = if (preparation.quality == MediaQuality.REDUCED) planVideo(source, preparation.session) else null
    val sent = if (plan == null) {
        file
    } else {
        val converted = convert(context, File(file.path), plan, preparation.progress)
        File(file.path).delete()
        FileToSend(converted.path, file.filename.substringBeforeLast('.') + ".mp4")
    }
    val still = withContext(Dispatchers.IO) { still(context, File(sent.path)) }
    val (width, height) = plan?.let { it.width.toInt() to it.height.toInt() } ?: (source.width to source.height)
    val info = VideoInfo(width.toUInt(), height.toUInt(), source.durationMs.toULong(), still?.path)
    return sent.copy(video = info)
}

/** The library's plan for [source], fitted to the server's limit; null: it is sent as it is. */
suspend fun planVideo(source: VideoSource, session: Session): VideoPlan? =
    session.videoPlan(
        source.width.toUInt(),
        source.height.toUInt(),
        source.fps,
        source.durationMs.toULong(),
        source.size.toULong(),
    )

/** The H.264 encoder's bit rate in [plan]. */
@OptIn(UnstableApi::class)
fun videoEncoderSettings(plan: VideoPlan): VideoEncoderSettings =
    VideoEncoderSettings.Builder().setBitrate(plan.videoBitrate.toInt()).build()

/** The AAC encoder's bit rate in [plan]. */
@OptIn(UnstableApi::class)
fun audioEncoderSettings(plan: VideoPlan): AudioEncoderSettings =
    AudioEncoderSettings.Builder().setBitrate(plan.audioBitrate.toInt()).build()

/** How often a conversion says how far it is. */
private const val PROGRESS_POLL_MS = 200L

/**
 * [source] converted to [plan] (H.264 at its size and bit rate, AAC, at
 * most its frame rate) into a new copy in the outgoing folder, without
 * where it was taken; how far goes to [progress]. Stops when the coroutine
 * is cancelled (its card taken away).
 */
@OptIn(UnstableApi::class)
private suspend fun convert(context: Context, source: File, plan: VideoPlan, progress: (Float) -> Unit): File {
    val output = newCopy(context, "mp4")
    try {
        // Transformer is used on the thread it was made on, with a looper.
        withContext(Dispatchers.Main) {
            val finished = CompletableDeferred<Unit>()
            val encoders = DefaultEncoderFactory.Builder(context)
                .setRequestedVideoEncoderSettings(videoEncoderSettings(plan))
                .setRequestedAudioEncoderSettings(audioEncoderSettings(plan))
                .build()
            val transformer = Transformer.Builder(context)
                .setVideoMimeType(MimeTypes.VIDEO_H264)
                .setAudioMimeType(MimeTypes.AUDIO_AAC)
                .setEncoderFactory(encoders)
                .setMuxerFactory(WithoutLocation())
                .addListener(object : Transformer.Listener {
                    override fun onCompleted(composition: Composition, exportResult: ExportResult) {
                        finished.complete(Unit)
                    }

                    override fun onError(composition: Composition, exportResult: ExportResult, exportException: ExportException) {
                        finished.completeExceptionally(
                            IOException("The video could not be converted: ${exportException.message}", exportException),
                        )
                    }
                })
                .build()
            val presentation = Presentation.createForWidthAndHeight(
                plan.width.toInt(),
                plan.height.toInt(),
                Presentation.LAYOUT_SCALE_TO_FIT,
            )
            val edited = EditedMediaItem.Builder(MediaItem.fromUri(Uri.fromFile(source)))
                // At most this rate: frames beyond it are dropped.
                .setFrameRate(plan.fps.roundToInt())
                .setEffects(Effects(listOf(), listOf(presentation)))
                .build()
            transformer.start(edited, output.path)
            try {
                val held = ProgressHolder()
                while (!finished.isCompleted) {
                    if (transformer.getProgress(held) == Transformer.PROGRESS_STATE_AVAILABLE) {
                        progress(held.progress / 100f)
                    }
                    withTimeoutOrNull(PROGRESS_POLL_MS) { finished.await() }
                }
                finished.await()
            } finally {
                if (!finished.isCompleted) transformer.cancel()
            }
        }
    } catch (e: Exception) {
        output.delete()
        throw e
    }
    return output
}

/**
 * The default MP4 muxer, without where a video was taken: reduced videos
 * leave location data out, as reduced photos do.
 */
@OptIn(UnstableApi::class)
private class WithoutLocation(private val muxers: Muxer.Factory = DefaultMuxer.Factory()) : Muxer.Factory by muxers {
    override fun create(path: String): Muxer {
        val muxer = muxers.create(path)
        return object : Muxer by muxer {
            override fun addMetadataEntry(metadataEntry: Metadata.Entry) {
                if (metadataEntry !is Mp4LocationData) muxer.addMetadataEntry(metadataEntry)
            }
        }
    }
}

/** What the library plans from, read from [file]; null if it has no pictures or cannot be read. */
private fun readVideo(file: File): VideoSource? {
    val retriever = MediaMetadataRetriever()
    return try {
        retriever.setDataSource(file.path)
        if (retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_HAS_VIDEO) != "yes") return null
        val width = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)?.toIntOrNull() ?: return null
        val height = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)?.toIntOrNull() ?: return null
        val rotation = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION)?.toIntOrNull() ?: 0
        val durationMs = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull() ?: 0L
        val frames = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_FRAME_COUNT)?.toLongOrNull()
        } else {
            null
        }
        val fps = if (frames != null && durationMs > 0) frames * 1000f / durationMs else 0f
        val turned = rotation % 180 != 0
        VideoSource(if (turned) height else width, if (turned) width else height, fps, durationMs, file.length())
    } catch (e: RuntimeException) {
        null
    } finally {
        retriever.release()
    }
}

/**
 * A JPEG of a frame near the start of [video] (a second in: the first is
 * often black), upright, at most [videoThumbnailEdge] px, in the outgoing
 * folder; null if none can be made.
 */
private fun still(context: Context, video: File): File? {
    val edge = videoThumbnailEdge().toInt()
    val retriever = MediaMetadataRetriever()
    val frame: Bitmap? = try {
        retriever.setDataSource(video.path)
        val durationMs = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull() ?: 0L
        val atUs = minOf(1_000L, durationMs / 2) * 1_000
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) {
            retriever.getScaledFrameAtTime(atUs, MediaMetadataRetriever.OPTION_CLOSEST_SYNC, edge, edge)
        } else {
            retriever.getFrameAtTime(atUs, MediaMetadataRetriever.OPTION_CLOSEST_SYNC)?.let { within(it, edge) }
        }
    } catch (e: RuntimeException) {
        null
    } finally {
        retriever.release()
    }
    if (frame == null) return null
    val still = newCopy(context, "jpg")
    return try {
        still.outputStream().use { out ->
            if (!frame.compress(Bitmap.CompressFormat.JPEG, 80, out)) throw IOException("no still")
        }
        still
    } catch (e: IOException) {
        still.delete()
        null
    } finally {
        frame.recycle()
    }
}

/** [bitmap] scaled to at most [edge] on its longer side, if larger. */
private fun within(bitmap: Bitmap, edge: Int): Bitmap {
    val longer = maxOf(bitmap.width, bitmap.height)
    if (longer <= edge) return bitmap
    val scale = edge.toFloat() / longer
    val scaled = Bitmap.createScaledBitmap(
        bitmap,
        (bitmap.width * scale).roundToInt().coerceAtLeast(1),
        (bitmap.height * scale).roundToInt().coerceAtLeast(1),
        true,
    )
    bitmap.recycle()
    return scaled
}
