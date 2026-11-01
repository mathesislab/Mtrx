package org.mtrx.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import java.time.Instant
import kotlin.math.sqrt
import kotlinx.coroutines.delay
import uniffi.mtrx.Presence
import uniffi.mtrx.PresenceDot
import uniffi.mtrx.presenceDotLabel
import uniffi.mtrx.presenceLine

private val Online = Color(0xFF34C759)
private val Idle = Color(0xFFFFA000)

/**
 * A round [avatar] of [avatarSize] with [presence]'s dot of [dotSize] on its
 * bottom right, ringed 2 dp in [ring] (the background's colour) and centred
 * on the avatar's circle at 45°; no dot offline or unknown.
 */
@Composable
fun WithPresenceDot(
    presence: Presence?,
    avatarSize: Dp,
    dotSize: Dp,
    ring: Color,
    modifier: Modifier = Modifier,
    avatar: @Composable () -> Unit,
) {
    Box(modifier) {
        avatar()
        val dot = presence?.dot ?: return@Box
        val outer = dotSize + 4.dp
        val centre = avatarSize * ((1f + 1f / sqrt(2f)) / 2f)
        Box(
            Modifier
                .offset(x = centre - outer / 2, y = centre - outer / 2)
                .size(outer)
                .clip(CircleShape)
                .background(ring)
                .padding(2.dp)
                .clip(CircleShape)
                .background(if (dot == PresenceDot.ONLINE) Online else Idle)
                .semantics { contentDescription = presenceDotLabel(dot) },
        )
    }
}

/**
 * The line under a person's name: their status message, "Active now",
 * "Idle" or when they were last active (updated each minute); nothing
 * without.
 */
@Composable
fun PresenceLine(
    presence: Presence?,
    style: TextStyle,
    modifier: Modifier = Modifier,
    textAlign: TextAlign? = null,
) {
    val now by produceState(Instant.now(), presence) {
        while (true) {
            value = Instant.now()
            delay(60_000)
        }
    }
    val text = presence?.let { presenceLine(it, now) } ?: return
    Text(
        text,
        modifier,
        style = style,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        textAlign = textAlign,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
    )
}
