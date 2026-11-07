package org.mtrx.app.timeline

import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.BottomSheetDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch
import org.mtrx.app.model.Images
import org.mtrx.app.ui.Avatar
import org.mtrx.app.ui.MtrxIcons
import org.mtrx.app.ui.PresenceLine
import org.mtrx.app.ui.WithPresenceDot
import uniffi.mtrx.Presence
import uniffi.mtrx.RoomSummary

/**
 * A person (a direct chat's, or one mentioned): avatar and name with their
 * [presence], user ID with a copy button, and "Add to space…", which lists
 * [spaces] (the top-level ones) to invite them to. Messages show in the
 * sheet, over its bottom.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PersonInfoSheet(
    model: PersonInfoModel,
    presence: Presence?,
    spaces: List<RoomSummary>,
    images: Images,
    onDismiss: () -> Unit,
) {
    val profile by model.profile.collectAsStateWithLifecycle()
    val choosingSpace by model.choosingSpace.collectAsStateWithLifecycle()
    val inviting by model.inviting.collectAsStateWithLifecycle()
    val message by model.message.collectAsStateWithLifecycle()
    val card = personCard(model.shown, profile)
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    @Suppress("DEPRECATION") // LocalClipboard's API is suspending; this is a plain copy.
    val clipboard = LocalClipboardManager.current

    LaunchedEffect(message) {
        val text = message ?: return@LaunchedEffect
        snackbar.showSnackbar(text)
        model.clearMessage()
    }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        // In the sheet's window: back from the spaces to the person first.
        BackHandler(enabled = choosingSpace, onBack = model::closeSpaces)
        Box(Modifier.fillMaxWidth()) {
            if (choosingSpace && spaces.isNotEmpty()) {
                SpaceChoices(spaces, inviting, images, onBack = model::closeSpaces, onChoose = model::invite)
            } else {
                Person(
                    card,
                    presence,
                    images,
                    canInvite = spaces.isNotEmpty(),
                    onCopy = {
                        clipboard.setText(AnnotatedString(card.userId))
                        // From Android 13 the system shows its own notice.
                        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
                            scope.launch { snackbar.showSnackbar("Copied") }
                        }
                    },
                    onAddToSpace = model::chooseSpace,
                )
            }
            SnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter))
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Person(
    card: PersonCard,
    presence: Presence?,
    images: Images,
    canInvite: Boolean,
    onCopy: () -> Unit,
    onAddToSpace: () -> Unit,
) {
    Column(Modifier.fillMaxWidth().padding(bottom = 16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        WithPresenceDot(presence, avatarSize = 80.dp, dotSize = 20.dp, ring = BottomSheetDefaults.ContainerColor) {
            Avatar(card.name, card.avatarUrl, seed = card.userId, images = images, size = 80.dp)
        }
        Spacer(Modifier.height(12.dp))
        Text(
            card.name,
            Modifier.padding(horizontal = 24.dp),
            style = MaterialTheme.typography.titleLarge,
            textAlign = TextAlign.Center,
        )
        PresenceLine(
            presence,
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(horizontal = 24.dp),
            textAlign = TextAlign.Center,
        )
        Row(Modifier.padding(start = 24.dp, end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            SelectionContainer(Modifier.weight(1f, fill = false)) {
                Text(
                    card.userId,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            IconButton(onClick = onCopy) {
                Icon(MtrxIcons.Copy, contentDescription = "Copy user ID")
            }
        }
        if (canInvite) {
            HorizontalDivider(Modifier.padding(top = 8.dp))
            ListItem(
                modifier = Modifier.clickable(onClick = onAddToSpace),
                leadingContent = { Icon(Icons.Filled.Add, contentDescription = null) },
                headlineContent = { Text("Add to space…") },
            )
        }
    }
}

/** The spaces to invite to; the one being invited to spins, the others wait. */
@Composable
private fun SpaceChoices(
    spaces: List<RoomSummary>,
    inviting: String?,
    images: Images,
    onBack: () -> Unit,
    onChoose: (RoomSummary) -> Unit,
) {
    Column(Modifier.fillMaxWidth().padding(bottom = 16.dp)) {
        Row(Modifier.padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
            Text("Add to space", style = MaterialTheme.typography.titleMedium)
        }
        LazyColumn {
            items(spaces, key = { it.id }) { space ->
                ListItem(
                    modifier = Modifier.clickable(enabled = inviting == null) { onChoose(space) },
                    leadingContent = {
                        // Spaces are squares, as on the desktop.
                        Avatar(space.name, space.avatarUrl, space.id, images, size = 40.dp, shape = RoundedCornerShape(8.dp))
                    },
                    headlineContent = { Text(space.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    trailingContent = if (inviting == space.id) {
                        { CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp) }
                    } else {
                        null
                    },
                )
            }
        }
    }
}
