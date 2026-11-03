package org.mtrx.app.rooms

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.mtrx.app.model.Images
import org.mtrx.app.ui.Avatar
import uniffi.mtrx.RoomKind
import uniffi.mtrx.RoomSummary

/** An invited room or space: its picture, name and topic, with Accept and Decline. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun InviteContent(room: RoomSummary, images: Images, busy: Boolean, onAccept: () -> Unit, onDecline: () -> Unit, onBack: () -> Unit) {
    val space = room.kind == RoomKind.SPACE
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Invite") },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier
                .padding(padding)
                .fillMaxSize()
                .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Avatar(
                room.name,
                room.avatarUrl,
                room.directUser?.userId ?: room.id,
                images,
                size = 80.dp,
                shape = if (space) RoundedCornerShape(16.dp) else CircleShape,
            )
            Text(room.name, style = MaterialTheme.typography.titleLarge, textAlign = TextAlign.Center)
            Text(
                if (space) "You are invited to this space." else "You are invited to this room.",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            room.topic?.takeIf { it.isNotBlank() }?.let {
                Text(it, style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.Center, maxLines = 6)
            }
            if (busy) {
                CircularProgressIndicator(Modifier.size(32.dp))
            } else {
                Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                    OutlinedButton(onClick = onDecline) { Text("Decline") }
                    Button(onClick = onAccept) { Text("Accept") }
                }
            }
        }
    }
}
