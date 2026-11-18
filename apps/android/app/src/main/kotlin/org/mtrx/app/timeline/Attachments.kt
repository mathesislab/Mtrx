package org.mtrx.app.timeline

import android.content.ActivityNotFoundException
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.ThumbnailUtils
import android.os.Build
import android.util.Size
import android.webkit.MimeTypeMap
import androidx.activity.compose.LocalActivityResultRegistryOwner
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.io.File
import java.io.IOException
import kotlin.math.roundToInt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.mtrx.FileToSend

/**
 * The paperclip left of the message field: a menu of "Photos and videos"
 * (the system's photo picker, several at once, no permission), "Files" (any,
 * several) and the camera, for a photo or a video (the camera app writes into
 * our cache; no permission either). Each picked file goes to [onAttach] with
 * how it is prepared ([prepareToSend]); what cannot start says why in
 * [onError].
 */
@Composable
internal fun AttachButton(
    enabled: Boolean,
    onAttach: (prepare: suspend () -> FileToSend) -> Unit,
    onError: (String) -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    // No activity in the screenshots: nothing to pick with.
    val pickers = if (LocalActivityResultRegistryOwner.current != null) rememberPickers(onAttach, onError) else null
    Box {
        IconButton(onClick = { open = true }, enabled = enabled) {
            Icon(PaperclipIcon, contentDescription = "Attach")
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(
                text = { Text("Photos and videos") },
                onClick = {
                    open = false
                    pickers?.media?.invoke()
                },
            )
            DropdownMenuItem(
                text = { Text("Files") },
                onClick = {
                    open = false
                    pickers?.files?.invoke()
                },
            )
            pickers?.camera?.let { camera ->
                DropdownMenuItem(
                    text = { Text("Take a photo") },
                    onClick = {
                        open = false
                        camera(false)
                    },
                )
                DropdownMenuItem(
                    text = { Text("Record a video") },
                    onClick = {
                        open = false
                        camera(true)
                    },
                )
            }
        }
    }
}

/** What the menu starts; [camera] (for a video or not) only on a device with one. */
private class Pickers(val media: () -> Unit, val files: () -> Unit, val camera: ((video: Boolean) -> Unit)?)

@Composable
private fun rememberPickers(onAttach: (prepare: suspend () -> FileToSend) -> Unit, onError: (String) -> Unit): Pickers {
    val app = LocalContext.current.applicationContext
    val attach by rememberUpdatedState<(Picked) -> Unit>({ picked -> onAttach { prepareToSend(app, picked) } })
    val media = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia()) { uris ->
        uris.forEach { attach(Picked.Media(it)) }
    }
    val documents = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        uris.forEach { attach(Picked.Document(it)) }
    }
    // The camera's file while the camera is out; kept if the app is ended meanwhile.
    var shooting by rememberSaveable { mutableStateOf<String?>(null) }
    val taken = { done: Boolean, video: Boolean ->
        val path = shooting
        shooting = null
        if (path != null) {
            if (done) attach(Picked.Camera(File(path), video)) else File(path).delete()
        }
    }
    val photo = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { taken(it, false) }
    val video = rememberLauncherForActivityResult(ActivityResultContracts.CaptureVideo()) { taken(it, true) }
    val hasCamera = remember { app.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY) }
    val camera = { isVideo: Boolean ->
        val file = cameraFile(app, isVideo)
        shooting = file.path
        try {
            (if (isVideo) video else photo).launch(uri(app, file.path))
        } catch (e: ActivityNotFoundException) {
            shooting = null
            file.delete()
            onError("No camera app")
        }
    }
    return Pickers(
        media = { media.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo)) },
        files = { documents.launch(arrayOf("*/*")) },
        camera = camera.takeIf { hasCamera },
    )
}

/**
 * The files picked to send, above the message field, like the desktop's
 * chips: a card each with its picture (or a file's symbol), its name, and ✕
 * ([onRemove]); "Preparing…" until its copy is made.
 */
@Composable
internal fun PickedFileCards(files: List<PickedFile>, onRemove: (String) -> Unit) {
    LazyRow(
        Modifier
            .fillMaxWidth()
            .padding(top = 8.dp),
        contentPadding = PaddingValues(horizontal = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        items(files, key = { it.id }) { card -> PickedFileCard(card, onRemove = { onRemove(card.id) }) }
    }
}

@Composable
private fun PickedFileCard(card: PickedFile, onRemove: () -> Unit) {
    val file = card.file
    val size = with(LocalDensity.current) { THUMBNAIL.roundToPx() }
    val picture: Bitmap? by produceState<Bitmap?>(null, file?.path) {
        value = file?.let { withContext(Dispatchers.IO) { thumbnail(File(it.path), size) } }
    }
    Surface(shape = RoundedCornerShape(12.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
        Row(Modifier.padding(6.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(
                Modifier
                    .size(THUMBNAIL)
                    .clip(RoundedCornerShape(8.dp))
                    .background(MaterialTheme.colorScheme.surface),
                contentAlignment = Alignment.Center,
            ) {
                val image = picture
                when {
                    image != null -> Image(
                        remember(image) { image.asImageBitmap() },
                        contentDescription = null,
                        modifier = Modifier.fillMaxSize(),
                        contentScale = ContentScale.Crop,
                    )
                    file == null -> CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                    else -> Icon(FileIcon, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            Text(
                file?.filename ?: "Preparing…",
                Modifier
                    .padding(start = 8.dp)
                    .widthIn(max = 140.dp),
                style = MaterialTheme.typography.bodySmall,
                color = if (file == null) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            IconButton(onClick = onRemove, modifier = Modifier.size(32.dp)) {
                Icon(
                    Icons.Filled.Close,
                    contentDescription = file?.let { "Remove ${it.filename}" } ?: "Stop preparing the file",
                    modifier = Modifier.size(18.dp),
                )
            }
        }
    }
}

/** While a message's file uploads ([fraction] of it sent), a ring that fills as it goes; nothing otherwise. */
@Composable
internal fun UploadRing(fraction: Float?) {
    if (fraction == null) return
    Box(
        Modifier
            .size(44.dp)
            .clip(CircleShape)
            .background(MaterialTheme.colorScheme.surface.copy(alpha = 0.8f))
            .semantics { contentDescription = "Uploading, ${(fraction * 100).roundToInt()} percent" },
        contentAlignment = Alignment.Center,
    ) {
        CircularProgressIndicator(progress = { fraction })
    }
}

private val THUMBNAIL = 40.dp

/** A small picture of a photo or video, about [size] px; null for other files, or one that cannot be read. */
private fun thumbnail(file: File, size: Int): Bitmap? {
    val type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: return null
    return try {
        when {
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && type.startsWith("image/") ->
                ThumbnailUtils.createImageThumbnail(file, Size(size, size), null)
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && type.startsWith("video/") ->
                ThumbnailUtils.createVideoThumbnail(file, Size(size, size), null)
            type.startsWith("image/") -> sampled(file, size)
            // Before Android 10, a video shows the file's symbol.
            else -> null
        }
    } catch (e: IOException) {
        null
    }
}

/** The image at [file], decoded at a power-of-two fraction no smaller than [size] px. */
private fun sampled(file: File, size: Int): Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(file.path, bounds)
    var sample = 1
    while (bounds.outWidth / (sample * 2) >= size && bounds.outHeight / (sample * 2) >= size) sample *= 2
    return BitmapFactory.decodeFile(file.path, BitmapFactory.Options().apply { inSampleSize = sample })
}

/** Material's attach_file. */
private val PaperclipIcon: ImageVector = vector(
    "Paperclip",
    "M16.5,6v11.5c0,2.21 -1.79,4 -4,4s-4,-1.79 -4,-4V5c0,-1.38 1.12,-2.5 2.5,-2.5s2.5,1.12 2.5,2.5v10.5c0,0.55 -0.45,1 -1,1s-1,-0.45 -1,-1V6H10v9.5c0,1.38 1.12,2.5 2.5,2.5s2.5,-1.12 2.5,-2.5V5c0,-2.21 -1.79,-4 -4,-4S7,2.79 7,5v12.5c0,3.04 2.46,5.5 5.5,5.5s5.5,-2.46 5.5,-5.5V6h-1.5z",
)

/** Material's insert_drive_file. */
private val FileIcon: ImageVector = vector(
    "File",
    "M6,2c-1.1,0 -1.99,0.9 -1.99,2L4,20c0,1.1 0.89,2 1.99,2H18c1.1,0 2,-0.9 2,-2V8l-6,-6H6zM13,9V3.5L18.5,9H13z",
)

private fun vector(name: String, path: String): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 24f,
    viewportHeight = 24f,
).addPath(pathData = addPathNodes(path), fill = SolidColor(Color.Black)).build()
