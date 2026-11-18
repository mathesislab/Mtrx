package org.mtrx.app.timeline

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.time.Duration
import org.mtrx.app.ui.MtrxIcons
import uniffi.mtrx.DisappearChoice

/** Who else is typing, in a line whose height stays while nobody is. */
@Composable
internal fun TypingLine(text: String) {
    Text(
        text,
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 2.dp),
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        maxLines = 1,
        minLines = 1,
        overflow = TextOverflow.Ellipsis,
    )
}

/**
 * The composer: the bar of a reply or an edit, the files [picked] to send
 * (cards; ✕ calls [onDetach]), the paperclip (what is picked goes to
 * [onAttach]; not during an edit), the text, the hourglass (what is sent
 * disappears [disappearAfterRead] after everyone has read it, said in
 * [disappearWords], one of [disappearChoices], chosen with [onDisappear])
 * and Send, with text or files alone.
 */
@Composable
internal fun Composer(
    draft: String,
    composing: Composing?,
    picked: List<PickedFile>,
    disappearAfterRead: Duration?,
    disappearWords: String,
    disappearChoices: List<DisappearChoice>,
    onDraft: (String) -> Unit,
    onCancel: () -> Unit,
    onDisappear: (Duration?) -> Unit,
    onAttach: (prepare: Prepare) -> Unit,
    onDetach: (String) -> Unit,
    onError: (String) -> Unit,
    onSend: () -> Unit,
) {
    // Typed here; set from outside (an edit starts, a send clears), the cursor goes to the end.
    var field by remember { mutableStateOf(TextFieldValue(draft, TextRange(draft.length))) }
    val shown = if (field.text == draft) field else TextFieldValue(draft, TextRange(draft.length))
    val focus = remember { FocusRequester() }
    LaunchedEffect(composing) {
        if (composing != null) focus.requestFocus()
    }
    Surface(tonalElevation = 2.dp) {
        Column(Modifier.fillMaxWidth()) {
            if (composing != null) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(start = 16.dp, end = 4.dp, top = 4.dp)
                        .height(IntrinsicSize.Min),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(
                        Modifier
                            .width(3.dp)
                            .fillMaxHeight()
                            .background(MaterialTheme.colorScheme.primary),
                    )
                    Column(
                        Modifier
                            .weight(1f)
                            .padding(start = 8.dp),
                    ) {
                        val heading = when (composing) {
                            is Composing.Replying -> "Replying to ${composing.name}"
                            is Composing.Editing -> "Editing"
                        }
                        Text(heading, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
                        if (composing is Composing.Replying && composing.preview.isNotEmpty()) {
                            Text(
                                composing.preview,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                    IconButton(onClick = onCancel) {
                        Icon(Icons.Filled.Close, contentDescription = "Cancel")
                    }
                }
            }
            if (picked.isNotEmpty()) PickedFileCards(picked, onDetach)
            Row(
                Modifier
                    .fillMaxWidth()
                    .navigationBarsPadding()
                    .imePadding()
                    .padding(horizontal = 8.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                AttachButton(enabled = composing !is Composing.Editing, onAttach = onAttach, onError = onError)
                OutlinedTextField(
                    value = shown,
                    onValueChange = {
                        field = it
                        if (it.text != draft) onDraft(it.text)
                    },
                    modifier = Modifier
                        .weight(1f)
                        .focusRequester(focus),
                    placeholder = { Text("Message") },
                    maxLines = 6,
                    keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                )
                DisappearButton(disappearAfterRead, disappearWords, disappearChoices, onDisappear)
                IconButton(onClick = onSend, enabled = canSend(draft, composing, picked)) {
                    Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "Send")
                }
            }
        }
    }
}

/**
 * The hourglass left of Send, in the accent colour while what is sent
 * disappears ([afterRead] after everyone has read it), muted while not;
 * [words] say which. A tap opens the menu of [choices], the current one
 * ticked.
 */
@Composable
private fun DisappearButton(
    afterRead: Duration?,
    words: String,
    choices: List<DisappearChoice>,
    onChoose: (Duration?) -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) {
            Icon(
                MtrxIcons.Hourglass,
                contentDescription = words,
                tint = if (afterRead != null) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            Text(
                "Disappear after everyone read it",
                Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            for (choice in choices) {
                DropdownMenuItem(
                    text = { Text(choice.label) },
                    onClick = {
                        open = false
                        onChoose(choice.afterRead)
                    },
                    trailingIcon = if (choice.afterRead == afterRead) {
                        { Icon(Icons.Filled.Check, contentDescription = "Selected") }
                    } else {
                        null
                    },
                )
            }
        }
    }
}
