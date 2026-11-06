package org.mtrx.app.rooms

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import org.mtrx.app.model.Images
import org.mtrx.app.ui.ConfirmDialog
import uniffi.mtrx.DirectoryKind
import uniffi.mtrx.DirectoryRoom
import uniffi.mtrx.JoinButton

/** The join screen's state, from [JoinModel]. */
data class JoinState(
    /** Ours, for the picker. */
    val ownServer: String,
    /** Browsed before, most recent first. */
    val servers: List<String>,
    /** Null for ours. */
    val server: String?,
    val query: String,
    val kind: DirectoryKind,
    val results: JoinResults,
    val pending: Set<String>,
    /** What a row offers, with our membership as the room lists have it (the library's joinButton). */
    val buttonOf: (DirectoryRoom) -> JoinButton,
    val error: String? = null,
)

/** What the join screen does. */
class JoinActions(
    val setServer: (String?) -> Unit,
    val setQuery: (String) -> Unit,
    val setKind: (DirectoryKind) -> Unit,
    val retry: () -> Unit,
    val more: () -> Unit,
    val clearError: () -> Unit,
    val rows: OverviewRowActions,
)

/**
 * Join a space or room: a server picker (ours, those browsed before,
 * Other server…), the search field (also for a pasted address or link),
 * All, Spaces or Rooms, and the results with Join or Ask to join.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun JoinContent(state: JoinState, images: Images, actions: JoinActions, onBack: () -> Unit) {
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(state.error) {
        val message = state.error ?: return@LaunchedEffect
        snackbar.showSnackbar(message)
        actions.clearError()
    }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Join a space or room") },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        Column(
            Modifier
                .padding(padding)
                .fillMaxSize(),
        ) {
            ServerPicker(state, actions.setServer)
            OutlinedTextField(
                value = state.query,
                onValueChange = actions.setQuery,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 4.dp),
                singleLine = true,
                placeholder = { Text("Search, or paste #room:server or a link") },
                leadingIcon = { Icon(Icons.Filled.Search, contentDescription = null) },
            )
            if (state.results !is JoinResults.Preview) {
                val kinds = listOf(DirectoryKind.ALL to "All", DirectoryKind.SPACES to "Spaces", DirectoryKind.ROOMS to "Rooms")
                SingleChoiceSegmentedButtonRow(
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 6.dp),
                ) {
                    kinds.forEachIndexed { index, (kind, label) ->
                        SegmentedButton(
                            selected = state.kind == kind,
                            onClick = { actions.setKind(kind) },
                            shape = SegmentedButtonDefaults.itemShape(index, kinds.size),
                        ) { Text(label) }
                    }
                }
            }
            when (val results = state.results) {
                JoinResults.Loading -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                is JoinResults.Failed -> Column(
                    Modifier
                        .fillMaxWidth()
                        .padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(results.message, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    TextButton(onClick = actions.retry) { Text("Try again") }
                }
                is JoinResults.Preview -> DirectoryRow(
                    results.room,
                    state.buttonOf(results.room),
                    results.room.id in state.pending,
                    images,
                    actions.rows,
                )
                is JoinResults.Listed -> LazyColumn(Modifier.fillMaxSize()) {
                    if (results.rooms.isEmpty()) {
                        item(key = "none") {
                            Text("Nothing found.", Modifier.padding(16.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                    items(results.rooms, key = { it.id }) { room ->
                        DirectoryRow(room, state.buttonOf(room), room.id in state.pending, images, actions.rows)
                    }
                    if (results.next != null) {
                        item(key = "more") {
                            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                                TextButton(onClick = actions.more) { Text("More") }
                            }
                        }
                    }
                }
            }
        }
    }
}

/** The server whose directory shows: ours, one browsed before, or another typed. */
@Composable
private fun ServerPicker(state: JoinState, onServer: (String?) -> Unit) {
    var open by remember { mutableStateOf(false) }
    var typing by remember { mutableStateOf(false) }
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text("Server", Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
        Box {
            TextButton(onClick = { open = true }) {
                Text(state.server ?: state.ownServer)
                Icon(Icons.Filled.ArrowDropDown, contentDescription = null)
            }
            DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                val choices = listOf<String?>(null) + state.servers.filter { it != state.ownServer }
                for (server in choices) {
                    DropdownMenuItem(
                        text = { Text(server ?: state.ownServer) },
                        onClick = {
                            open = false
                            onServer(server)
                        },
                        trailingIcon = if (server == state.server) {
                            { Icon(Icons.Filled.Check, contentDescription = "Selected") }
                        } else {
                            null
                        },
                    )
                }
                DropdownMenuItem(
                    text = { Text("Other server…") },
                    onClick = {
                        open = false
                        typing = true
                    },
                )
            }
        }
    }
    if (typing) {
        var name by remember { mutableStateOf("") }
        ConfirmDialog(
            title = "Other server",
            message = null,
            button = "Browse",
            confirmEnabled = name.isNotBlank(),
            onConfirm = {
                typing = false
                onServer(name)
            },
            onDismiss = { typing = false },
        ) {
            OutlinedTextField(
                value = name,
                onValueChange = { name = it },
                singleLine = true,
                placeholder = { Text("Server, such as uni-hamburg.de") },
            )
        }
    }
}
