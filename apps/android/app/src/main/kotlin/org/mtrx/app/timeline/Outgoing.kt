package org.mtrx.app.timeline

import android.content.ContentResolver
import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import java.io.File
import java.io.IOException
import java.time.LocalDateTime
import java.time.format.DateTimeFormatter
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.mtrx.FileToSend

/** A photo, video or file picked to send, as the system hands it over. */
sealed interface Picked {
    /** From the photo picker: a photo or video. */
    data class Media(val uri: Uri) : Picked

    /** From Files: any document. */
    data class Document(val uri: Uri) : Picked

    /** Taken with the camera into [file] ([cameraFile]): a photo, or a [video]. */
    data class Camera(val file: File, val video: Boolean) : Picked
}

/** A file picked to send, as its card above the field shows it: ready once prepared ([file]). */
data class PickedFile(val id: String = UUID.randomUUID().toString(), val file: FileToSend? = null)

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
 * camera's from the date ("Photo 2026-11-18 14.03.jpg"). Photos and videos
 * are to be reduced here (docs/reducing-media.md).
 */
suspend fun prepareToSend(context: Context, picked: Picked, now: LocalDateTime = LocalDateTime.now()): FileToSend =
    withContext(Dispatchers.IO) {
        when (picked) {
            is Picked.Media -> copyContent(context, picked.uri, now)
            is Picked.Document -> copyContent(context, picked.uri, now)
            is Picked.Camera -> FileToSend(picked.file.path, datedName(picked.video, now, picked.file.extension))
        }
    }

/** The copy is no longer needed: sent (the library has read it), or taken away. */
fun deleteCopy(file: FileToSend) {
    File(file.path).delete()
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
private fun newCopy(context: Context, extension: String): File {
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
