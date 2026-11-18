package org.mtrx.app.timeline

import android.content.ContentResolver
import android.content.Context
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import java.io.File
import java.io.IOException
import java.time.LocalDateTime
import java.time.format.DateTimeFormatter
import java.util.UUID
import kotlin.math.sqrt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.mtrx.FileToSend
import uniffi.mtrx.MediaQuality
import uniffi.mtrx.Session

/** A photo, video or file picked to send, as the system hands it over. */
sealed interface Picked {
    /** From the photo picker: a photo or video. */
    data class Media(val uri: Uri) : Picked

    /** From Files: any document. */
    data class Document(val uri: Uri) : Picked

    /** Taken with the camera into [file] ([cameraFile]): a photo, or a [video]. */
    data class Camera(val file: File, val video: Boolean) : Picked
}

/**
 * A file picked to send, as its card above the field shows it: ready once
 * prepared ([file]); a video being converted says how far ([progress], 0 to 1).
 */
data class PickedFile(val id: String = UUID.randomUUID().toString(), val file: FileToSend? = null, val progress: Float? = null)

/**
 * What preparing a picked file takes besides it: how this phone sends
 * photos and videos ([quality]), the session that plans a video's
 * conversion (`Session.videoPlan`), and where a conversion says how far it
 * is ([progress], 0 to 1, from any thread).
 */
class Preparation(val quality: MediaQuality, val session: Session, val progress: (Float) -> Unit = {})

/** How a picked file is made ready to send ([prepareToSend]), with what the timeline gives it. */
typealias Prepare = suspend (Preparation) -> FileToSend

/** Send can go: there is text, or there are files picked, all prepared (an edit needs its text, and sends no files). */
internal fun canSend(draft: String, composing: Composing?, picked: List<PickedFile>): Boolean =
    if (composing is Composing.Editing) {
        draft.isNotBlank()
    } else {
        (draft.isNotBlank() || picked.isNotEmpty()) && picked.all { it.file != null }
    }

/**
 * [picked] as a file to send: a copy in [outgoingFolder] (a camera's file is
 * there already), named as it is sent: a picked one by its display name, a
 * camera's from the date ("Photo 2026-11-18 14.03.jpg"). A photo or video
 * (not one from Files) is made ready as [preparation] says
 * (docs/reducing-media.md): reduced, a HEIF photo becomes a JPEG the
 * library can reduce, and a video is converted to the library's plan
 * ([prepareVideo]); every video carries its still and length.
 */
suspend fun prepareToSend(
    context: Context,
    picked: Picked,
    preparation: Preparation,
    now: LocalDateTime = LocalDateTime.now(),
): FileToSend {
    val copy = withContext(Dispatchers.IO) {
        when (picked) {
            is Picked.Media -> copyContent(context, picked.uri, now)
            is Picked.Document -> copyContent(context, picked.uri, now)
            is Picked.Camera -> FileToSend(picked.file.path, datedName(picked.video, now, picked.file.extension))
        }
    }
    if (picked is Picked.Document) return copy
    val type = typeOf(copy.filename)
    return try {
        when {
            type.startsWith("video/") -> prepareVideo(context, copy, preparation)
            type in HEIF && preparation.quality == MediaQuality.REDUCED -> withContext(Dispatchers.IO) { heifAsJpeg(context, copy) }
            else -> copy
        }
    } catch (e: Exception) {
        deleteCopy(copy)
        throw e
    }
}

/** The copy is no longer needed: sent (the library has read it), or taken away. A video's still goes with it. */
fun deleteCopy(file: FileToSend) {
    File(file.path).delete()
    file.video?.thumbnailPath?.let { File(it).delete() }
}

private val HEIF = setOf("image/heic", "image/heif")

/** The type of a file named [name], from its extension; empty if not known. */
private fun typeOf(name: String): String =
    MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase()).orEmpty()

/** Decoded HEIF photos are at most this large, for memory (the library reduces them further). */
private const val MAX_DECODED_PIXELS = 16_000_000

/**
 * A HEIF photo as a JPEG at quality 90, upright (the library cannot read
 * HEIF, but reduces JPEGs), its copy replacing [heif]'s; before Android 9,
 * which cannot decode HEIF, as it is.
 */
private fun heifAsJpeg(context: Context, heif: FileToSend): FileToSend {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P) return heif
    val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(File(heif.path))) { decoder, info, _ ->
        decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
        val pixels = info.size.width.toLong() * info.size.height
        if (pixels > MAX_DECODED_PIXELS) {
            decoder.setTargetSampleSize(sqrt(pixels.toDouble() / MAX_DECODED_PIXELS).toInt() + 1)
        }
    }
    val jpeg = newCopy(context, "jpg")
    try {
        jpeg.outputStream().use { out ->
            if (!bitmap.compress(Bitmap.CompressFormat.JPEG, 90, out)) throw IOException("${heif.filename} could not be read")
        }
    } catch (e: IOException) {
        jpeg.delete()
        throw e
    } finally {
        bitmap.recycle()
    }
    deleteCopy(heif)
    return FileToSend(jpeg.path, heif.filename.substringBeforeLast('.') + ".jpg")
}

/** A new file in [outgoingFolder] for the camera to write a photo or a [video] into. */
fun cameraFile(context: Context, video: Boolean): File = newCopy(context, if (video) "mp4" else "jpg")

private val DATED: DateTimeFormatter = DateTimeFormatter.ofPattern("yyyy-MM-dd HH.mm")

/** "Photo 2026-11-18 14.03.jpg", "Video 2026-11-18 14.03.mp4": the extension lowercased, JPEG's as "jpg". */
internal fun datedName(video: Boolean, at: LocalDateTime, extension: String): String {
    val ext = extension.lowercase().let { if (it == "jpeg") "jpg" else it }.ifEmpty { if (video) "mp4" else "jpg" }
    return "${if (video) "Video" else "Photo"} ${at.format(DATED)}.$ext"
}

/** [uri]'s content copied into [outgoingFolder], under its display name (with an extension for its type). */
private fun copyContent(context: Context, uri: Uri, now: LocalDateTime): FileToSend {
    val resolver = context.contentResolver
    val type = resolver.getType(uri)
    val name = displayName(resolver, uri)?.let { withExtension(it, type) }
        ?: datedName(type?.startsWith("video/") == true, now, extensionOf(type))
    val copy = newCopy(context, name.substringAfterLast('.', ""))
    try {
        val input = resolver.openInputStream(uri) ?: throw IOException("$name could not be read")
        input.use { source -> copy.outputStream().use { source.copyTo(it) } }
    } catch (e: IOException) {
        copy.delete()
        throw e
    }
    return FileToSend(copy.path, name)
}

/** The name the provider gives [uri] (`OpenableColumns.DISPLAY_NAME`), without any folder. */
private fun displayName(resolver: ContentResolver, uri: Uri): String? =
    resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
        if (cursor.moveToFirst()) cursor.getString(0) else null
    }?.substringAfterLast('/')?.takeIf { it.isNotBlank() }

/** [name], with the extension of [type] if it has none: the library tells the type from the name. */
private fun withExtension(name: String, type: String?): String {
    val ext = extensionOf(type)
    return if ('.' in name || ext.isEmpty()) name else "$name.$ext"
}

private fun extensionOf(type: String?): String =
    type?.let { MimeTypeMap.getSingleton().getExtensionFromMimeType(it) }.orEmpty()

/** A new place in [outgoingFolder], with [extension] so other apps know the type. */
internal fun newCopy(context: Context, extension: String): File {
    val name = UUID.randomUUID().toString()
    return File(outgoingFolder(context), if (extension.isEmpty()) name else "$name.$extension")
}

/**
 * `outgoing` in the cache, shared with camera apps through the FileProvider
 * (`file_paths.xml`). Copies a run left there (it ended before sending them)
 * go once a day old, the first time it is used in a run: a camera may still
 * be writing a newer one for an app it outlived.
 */
private fun outgoingFolder(context: Context): File = synchronized(Swept) {
    val folder = File(context.cacheDir, "outgoing")
    if (!Swept.done) {
        val dayAgo = System.currentTimeMillis() - 24 * 60 * 60 * 1000L
        folder.listFiles()?.filter { it.lastModified() < dayAgo }?.forEach { it.delete() }
        Swept.done = true
    }
    folder.mkdirs()
    folder
}

private object Swept {
    var done = false
}
