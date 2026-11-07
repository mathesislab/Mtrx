package org.mtrx.app.timeline

import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import androidx.annotation.RequiresApi
import androidx.core.content.FileProvider
import java.io.File
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.mtrx.Download
import uniffi.mtrx.DownloadState
import uniffi.mtrx.Media

/** What to do with a message's file once downloaded (to the cache). */
enum class FileUse { OPEN, SHARE, SAVE }

/** A message's [media], downloaded to [path], for [use]. */
data class FileRequest(val path: String, val media: Media, val use: FileUse)

/** Failed or cancelled: it can go again. */
val Download.stopped: Boolean
    get() = state is DownloadState.Failed || state is DownloadState.Cancelled

/** Newest first, as the library lists them: each message's file's newest download, by its source. */
fun downloadsBySource(downloads: List<Download>): Map<String, Download> = buildMap {
    for (download in downloads) putIfAbsent(download.source, download)
}

/** The app's FileProvider (AndroidManifest.xml), which shares the cache's downloads. */
private fun uri(context: Context, path: String): Uri =
    FileProvider.getUriForFile(context, "${context.packageName}.files", File(path))

private fun mimeType(media: Media): String = media.mimetype
    ?: MimeTypeMap.getSingleton().getMimeTypeFromExtension(File(media.filename).extension.lowercase())
    ?: "application/octet-stream"

/** Open the file with an app that can (the system asks which); false if none can. */
fun openFile(context: Context, request: FileRequest): Boolean {
    val view = Intent(Intent.ACTION_VIEW)
        .setDataAndType(uri(context, request.path), mimeType(request.media))
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    return try {
        context.startActivity(view)
        true
    } catch (e: ActivityNotFoundException) {
        false
    }
}

/** Share the file with the app the user picks. */
fun shareFile(context: Context, request: FileRequest) {
    val file = uri(context, request.path)
    val send = Intent(Intent.ACTION_SEND)
        .setType(mimeType(request.media))
        .putExtra(Intent.EXTRA_STREAM, file)
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    // The chooser passes the grant on to the app picked.
    send.clipData = ClipData.newRawUri(null, file)
    context.startActivity(Intent.createChooser(send, null))
}

/** Copy the file into the Downloads collection (API 29+); false if it failed. */
@RequiresApi(Build.VERSION_CODES.Q)
suspend fun saveToDownloads(context: Context, request: FileRequest): Boolean = withContext(Dispatchers.IO) {
    val resolver = context.contentResolver
    val values = ContentValues().apply {
        put(MediaStore.Downloads.DISPLAY_NAME, request.media.filename.substringAfterLast('/'))
        put(MediaStore.Downloads.MIME_TYPE, mimeType(request.media))
        put(MediaStore.Downloads.IS_PENDING, 1)
    }
    val item = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: return@withContext false
    try {
        val out = resolver.openOutputStream(item) ?: throw IOException("no output")
        out.use { File(request.path).inputStream().use { input -> input.copyTo(it) } }
        resolver.update(item, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
        true
    } catch (e: IOException) {
        resolver.delete(item, null, null)
        false
    }
}

/** Copy the file to [target], a document the user created (API 26–28); false if it failed. */
suspend fun copyFile(context: Context, request: FileRequest, target: Uri): Boolean = withContext(Dispatchers.IO) {
    try {
        val out = context.contentResolver.openOutputStream(target) ?: throw IOException("no output")
        out.use { File(request.path).inputStream().use { input -> input.copyTo(it) } }
        true
    } catch (e: IOException) {
        false
    }
}
