package org.mtrx.app.settings

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import org.mtrx.app.model.SignedInSession
import org.mtrx.app.model.VerificationModel
import uniffi.mtrx.VerificationEmoji
import uniffi.mtrx.VerificationState

/**
 * The session's verification ([SignedInSession.verification]) when the
 * screen opens, outgoing or accepted, until it is done or cancelled;
 * [onClose] leaves the screen.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun VerificationScreen(session: SignedInSession, onClose: () -> Unit) {
    // The one it was opened for, also once closed (while the screen leaves).
    val current = remember(session) { session.verification.value }
    if (current == null) {
        // None (the screen restored after the process was stopped).
        val currentOnClose by rememberUpdatedState(onClose)
        LaunchedEffect(Unit) { currentOnClose() }
        return
    }
    val state by current.state.collectAsStateWithLifecycle()
    val busy by current.busy.collectAsStateWithLifecycle()
    val error by current.error.collectAsStateWithLifecycle()
    val close = {
        session.closeVerification(current)
        onClose()
    }
    // Back is Cancel while it runs (not while keys are shared), Close after.
    BackHandler {
        when {
            VerificationModel.isEnded(state) -> close()
            state !is VerificationState.SharingKeys -> current.cancel()
        }
    }

    Scaffold(topBar = { TopAppBar(title = { Text("Verify this device") }) }) { padding ->
        Column(
            Modifier
                .padding(padding)
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            when (val shown = state) {
                VerificationState.Waiting -> {
                    Message(
                        if (current.outgoing) "Accept the request on your other device." else "Waiting for the other device…",
                    )
                    CircularProgressIndicator()
                    OutlinedButton(onClick = current::cancel, enabled = !busy) { Text("Cancel") }
                }
                is VerificationState.Emojis -> {
                    EmojiGrid(shown.emojis)
                    Message("Do they match the other device?")
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        OutlinedButton(onClick = current::mismatch, enabled = !busy) { Text("They do not match") }
                        Button(onClick = current::confirm, enabled = !busy) { Text("They match") }
                    }
                }
                VerificationState.Confirmed -> {
                    Message("Waiting for the other device to confirm…")
                    CircularProgressIndicator()
                    OutlinedButton(onClick = current::cancel, enabled = !busy) { Text("Cancel") }
                }
                VerificationState.SharingKeys -> {
                    Message("Finishing… keep the app open.")
                    CircularProgressIndicator()
                }
                VerificationState.Done -> {
                    Message("This device is verified.")
                    Button(onClick = close) { Text("Done") }
                }
                is VerificationState.Cancelled -> {
                    Message("Verification was cancelled.")
                    if (shown.reason.isNotEmpty()) {
                        Text(
                            shown.reason,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = TextAlign.Center,
                        )
                    }
                    Button(onClick = close) { Text("Close") }
                }
            }
            error?.let { Text(it, color = MaterialTheme.colorScheme.error, textAlign = TextAlign.Center) }
        }
    }
}

@Composable
private fun Message(text: String) {
    Text(text, style = MaterialTheme.typography.titleMedium, textAlign = TextAlign.Center)
}

/** The emoji in order, four to a row: the symbol large, its name below. */
@Composable
private fun EmojiGrid(emojis: List<VerificationEmoji>) {
    Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
        emojis.chunked(4).forEach { row ->
            Row(Modifier.fillMaxWidth()) {
                row.forEach { emoji ->
                    Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(emoji.symbol, fontSize = 40.sp)
                        Text(
                            emoji.description,
                            style = MaterialTheme.typography.labelMedium,
                            textAlign = TextAlign.Center,
                        )
                    }
                }
                // Keep a short last row's cells as wide as the others.
                repeat(4 - row.size) { Spacer(Modifier.weight(1f)) }
            }
        }
    }
}

/** Another of our devices asks to verify this one. */
@Composable
fun VerificationRequestDialog(onAccept: () -> Unit, onDecline: () -> Unit) {
    AlertDialog(
        // Answered with the buttons only.
        onDismissRequest = {},
        title = { Text("Verify this device?") },
        text = { Text("Another device wants to verify this one.") },
        confirmButton = { TextButton(onClick = onAccept) { Text("Accept") } },
        dismissButton = { TextButton(onClick = onDecline) { Text("Decline") } },
    )
}
