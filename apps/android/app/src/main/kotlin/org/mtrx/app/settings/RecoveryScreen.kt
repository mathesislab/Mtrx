package org.mtrx.app.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.flow.first
import org.mtrx.app.model.INVALID_RECOVERY_KEY
import org.mtrx.app.model.Recovery
import org.mtrx.app.model.SignedInSession
import uniffi.mtrx.RecoveryStep

/**
 * Unlock encryption with the recovery key. The recovery runs in the
 * [SignedInSession]: leaving the screen does not stop it, and coming back
 * shows it. Done goes back ([onDone]).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RecoveryScreen(session: SignedInSession, onBack: () -> Unit, onDone: () -> Unit) {
    val recovery by session.recovery.collectAsStateWithLifecycle()
    // Not saved with the screen's state: the key is a secret.
    var key by remember { mutableStateOf("") }
    val running = recovery is Recovery.Running
    val invalidKey = recovery == Recovery.InvalidKey

    val currentOnDone by rememberUpdatedState(onDone)
    LaunchedEffect(session) {
        // A previous attempt's result is not this one's (one still running is).
        session.resetRecovery()
        session.recovery.first { it == Recovery.Done }
        session.resetRecovery()
        currentOnDone()
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Recovery key") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier
                .padding(padding)
                .fillMaxSize()
                .imePadding()
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                "Enter the recovery key of this account to read its encrypted messages on this device.",
                style = MaterialTheme.typography.bodyMedium,
            )
            OutlinedTextField(
                value = key,
                onValueChange = {
                    key = it
                    // Corrected: the error was about the previous key.
                    if (invalidKey) session.resetRecovery()
                },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Recovery key") },
                enabled = !running,
                isError = invalidKey,
                supportingText = if (invalidKey) {
                    { Text(INVALID_RECOVERY_KEY) }
                } else {
                    null
                },
                minLines = 3,
                textStyle = MaterialTheme.typography.bodyLarge.copy(fontFamily = FontFamily.Monospace),
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                    keyboardType = KeyboardType.Ascii,
                ),
            )
            Button(
                onClick = { session.recover(key.trim()) },
                enabled = key.isNotBlank() && !running,
                modifier = Modifier.align(Alignment.End),
            ) { Text("Recover") }

            when (val current = recovery) {
                is Recovery.Running -> Steps(current.step)
                is Recovery.Failed -> {
                    Text("Could not recover.", color = MaterialTheme.colorScheme.error)
                    if (current.reason.isNotEmpty()) {
                        Text(current.reason, style = MaterialTheme.typography.bodySmall)
                    }
                }
                // Under the field.
                Recovery.InvalidKey, Recovery.Idle, Recovery.Done -> {}
            }
        }
    }
}

/** The recovery's steps: done ones checked, the current one spinning. */
@Composable
private fun Steps(step: RecoveryStep?) {
    val index = when (step) {
        null, RecoveryStep.CheckingKey -> 0
        RecoveryStep.ImportingIdentity -> 1
        is RecoveryStep.DownloadingKeys -> 2
    }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        StepLine("Checking the key", done = index > 0, current = index == 0)
        StepLine("Verifying this device", done = index > 1, current = index == 1)
        if (step is RecoveryStep.DownloadingKeys) {
            StepLine("Fetching keys: ${step.done} of ${step.total} rooms", done = false, current = true)
            if (step.total > 0u) {
                LinearProgressIndicator(
                    progress = { step.done.toFloat() / step.total.toFloat() },
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            Text(
                "This can take a minute with many rooms.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun StepLine(text: String, done: Boolean, current: Boolean) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Box(Modifier.size(20.dp), contentAlignment = Alignment.Center) {
            when {
                done -> Icon(Icons.Filled.Check, contentDescription = "Done", tint = MaterialTheme.colorScheme.primary)
                current -> CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
            }
        }
        Text(
            text,
            color = if (done || current) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
