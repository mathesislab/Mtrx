package org.mtrx.app.rooms

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material3.AssistChip
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.InputChip
import androidx.compose.material3.InputChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import org.mtrx.app.model.Images
import org.mtrx.app.ui.Avatar
import org.mtrx.app.ui.ChoiceRow
import org.mtrx.app.ui.ConfirmDialog
import org.mtrx.app.ui.SectionHeading
import org.mtrx.app.ui.SwitchRow
import uniffi.mtrx.LevelConfirmation
import uniffi.mtrx.Member
import uniffi.mtrx.Membership
import uniffi.mtrx.RoomSettings
import uniffi.mtrx.Topic
import uniffi.mtrx.levelConfirmation
import uniffi.mtrx.typedLevel

/** What Room info shows of a room or space, besides its settings and members. */
data class RoomInfoHeader(
    val roomId: String,
    val name: String,
    val avatarUrl: String?,
    val topic: Topic,
    val space: Boolean,
)

/**
 * Room info, full screen: the picture, name (renamed in place where we
 * may) and address; Invite… and Leave…; the topic in a box with Edit; the
 * settings we may change; the members. [spaceNames] names the spaces of
 * the room's settings. [onLeave] asks first.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RoomInfoContent(
    header: RoomInfoHeader,
    settings: RoomSettings?,
    members: List<Member>?,
    busy: String?,
    error: String?,
    ownUserId: String,
    spaceNames: Map<String, String>,
    images: Images,
    actions: RoomInfoActions,
    onBack: () -> Unit,
    onLeave: () -> Unit,
    onOpenLink: (String) -> Unit,
) {
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(error) {
        val message = error ?: return@LaunchedEffect
        snackbar.showSnackbar(message)
        actions.clearError()
    }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(if (header.space) "Space info" else "Room info") },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        LazyColumn(
            Modifier
                .padding(padding)
                .fillMaxSize(),
        ) {
            item(key = "header") {
                InfoHeader(header, settings, images, busy, actions, onLeave)
            }
            item(key = "topic") {
                TopicBox(header.topic, settings?.canChangeTopic == true, actions::setTopic, onOpenLink)
            }
            if (settings == null) {
                item(key = "loading") { Loading() }
            } else {
                settingsSection(header, settings, members?.count { it.membership == Membership.JOINED }, spaceNames, ownUserId, actions)
            }
            membersSection(members, images, actions)
        }
    }
}

/** The picture, the name (renamed in place where we may), the address, and Invite… and Leave…. */
@Composable
fun InfoHeader(
    header: RoomInfoHeader,
    settings: RoomSettings?,
    images: Images,
    busy: String?,
    actions: RoomInfoActions,
    onLeave: () -> Unit,
) {
    var renaming by remember { mutableStateOf<String?>(null) }
    var inviting by remember { mutableStateOf(false) }
    val shape = if (header.space) RoundedCornerShape(14.dp) else androidx.compose.foundation.shape.CircleShape
    Column(
        Modifier
            .fillMaxWidth()
            .padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Avatar(header.name, header.avatarUrl, header.roomId, images, size = 72.dp, shape = shape)
        val editing = renaming
        if (editing != null) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(
                    value = editing,
                    onValueChange = { renaming = it },
                    modifier = Modifier.weight(1f),
                    singleLine = true,
                    label = { Text("Name") },
                )
                IconButton(onClick = { renaming = null }) { Icon(Icons.Filled.Close, contentDescription = "Cancel") }
                IconButton(
                    onClick = {
                        actions.rename(editing)
                        renaming = null
                    },
                    enabled = editing.isNotBlank(),
                ) { Icon(Icons.Filled.Check, contentDescription = "Save") }
            }
        } else {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(header.name, style = MaterialTheme.typography.titleLarge, maxLines = 2, overflow = TextOverflow.Ellipsis)
                if (settings?.canRename == true) {
                    IconButton(onClick = { renaming = header.name }) {
                        Icon(Icons.Filled.Edit, contentDescription = "Rename", modifier = Modifier.size(18.dp))
                    }
                }
            }
        }
        settings?.address?.let { address ->
            SelectionContainer {
                Text(address, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), modifier = Modifier.padding(top = 6.dp)) {
            if (settings?.canInvite == true) {
                FilledTonalButton(onClick = { inviting = true }) { Text("Invite…") }
            }
            OutlinedButton(
                onClick = onLeave,
                colors = ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.onSurfaceVariant),
            ) { Text("Leave…") }
        }
        if (busy != null) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp)
                Text(busy, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
    if (inviting) {
        InviteDialog(header.name, onInvite = actions::invite, onDismiss = { inviting = false })
    }
}

/** Invite someone by their user ID. */
@Composable
fun InviteDialog(name: String, onInvite: (String) -> Unit, onDismiss: () -> Unit) {
    var person by remember { mutableStateOf("") }
    val userId = userIdOf(person)
    ConfirmDialog(
        title = "Invite to $name",
        message = null,
        button = "Invite",
        confirmEnabled = userId != null,
        onConfirm = {
            userId?.let(onInvite)
            onDismiss()
        },
        onDismiss = onDismiss,
    ) {
        OutlinedTextField(
            value = person,
            onValueChange = { person = it },
            singleLine = true,
            label = { Text("User ID") },
            placeholder = { Text("@name:server") },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email),
        )
    }
}

/** The topic in a box, with Edit where we may; the editor in its place while editing. */
@Composable
fun TopicBox(topic: Topic, canEdit: Boolean, onSave: (Topic) -> Unit, onOpenLink: (String) -> Unit, initiallyEditing: Boolean = false) {
    var editing by remember { mutableStateOf(initiallyEditing) }
    Surface(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 4.dp),
        shape = RoundedCornerShape(12.dp),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
    ) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (editing) {
                TopicEditor(
                    topic,
                    onSave = {
                        onSave(it)
                        editing = false
                    },
                    onCancel = { editing = false },
                )
                return@Column
            }
            Row(verticalAlignment = Alignment.Top) {
                Column(Modifier.weight(1f)) {
                    Text("Topic", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    if (topic.text.isBlank() && topic.links.isEmpty()) {
                        Text(
                            if (canEdit) ADD_A_TOPIC else "No topic",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    TopicText(topic)
                }
                if (canEdit) TextButton(onClick = { editing = true }) { Text("Edit") }
            }
            TopicLinks(topic.links, onOpen = onOpenLink)
        }
    }
}

/**
 * The settings we may change, each with its current value; the spaces the
 * room is in as chips, the facts line, and the people banned.
 */
fun LazyListScope.settingsSection(
    header: RoomInfoHeader,
    settings: RoomSettings,
    memberCount: Int?,
    spaceNames: Map<String, String>,
    ownUserId: String,
    actions: RoomInfoActions,
) {
    item(key = "settings") {
        SettingsColumn(header, settings, memberCount, spaceNames, ownUserId, actions)
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun SettingsColumn(
    header: RoomInfoHeader,
    settings: RoomSettings,
    memberCount: Int?,
    spaceNames: Map<String, String>,
    ownUserId: String,
    actions: RoomInfoActions,
) {
    var encrypting by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxWidth()) {
        SectionHeading("Settings")
        ChoiceRow(
            "Notifications",
            settings.notify,
            NOTIFY_OPTIONS.map { it.value to it.label },
            NOTIFY_OPTIONS.firstOrNull { it.value == settings.notify }?.explanation,
            enabled = true,
            onChoose = actions::setNotify,
        )
        if (settings.canChangeAccess) {
            ChoiceRow(
                "Who can join",
                settings.joinRule,
                settings.joinRuleChoices.map { it.rule to it.title },
                settings.joinRuleFact,
                enabled = true,
                onChoose = actions::setJoinRule,
            )
        }
        if (settings.canChangeAccess && showsListed(settings)) {
            SwitchRow(
                "Listed in the directory",
                checked = settings.listed == true,
                explanation = "Found by people browsing the rooms of ${serverOf(ownUserId)}.",
                onChange = actions::setListed,
            )
        }
        for (link in settings.spaces.filter { it.canChange }) {
            val space = spaceNames[link.spaceId] ?: link.spaceId
            SwitchRow(
                "Suggested in $space",
                checked = link.suggested,
                explanation = "Offered first to the space's members, and to people who join it.",
                onChange = { actions.setSuggested(link.spaceId, it) },
            )
        }
        if (settings.canChangeHistory) {
            ChoiceRow(
                "Who can read the history",
                settings.history,
                HISTORY_OPTIONS.map { it.value to it.label },
                HISTORY_OPTIONS.firstOrNull { it.value == settings.history }?.explanation,
                enabled = true,
                onChoose = actions::setHistory,
            )
        }
        if (settings.canChangePermissions) {
            ChoiceRow(
                "Who can edit",
                settings.permissions,
                PERMISSION_OPTIONS.map { it.value to it.label },
                PERMISSION_OPTIONS.firstOrNull { it.value == settings.permissions }?.explanation,
                enabled = true,
                onChoose = actions::setPermissions,
            )
        }
        if (!header.space && (settings.encrypted || settings.canEncrypt)) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text("Encryption", style = MaterialTheme.typography.bodyLarge)
                    Text(
                        if (settings.encrypted) "Only members' devices can read the messages." else "Messages are stored readable on the servers.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (settings.encrypted) {
                    Icon(Icons.Filled.Lock, contentDescription = null, modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.primary)
                    Spacer(Modifier.width(4.dp))
                    Text("On", color = MaterialTheme.colorScheme.primary)
                } else {
                    TextButton(onClick = { encrypting = true }) { Text("Turn on…") }
                }
            }
        }
        if (settings.canChangeAddress) AddressRow(settings.address, serverOf(ownUserId), actions::setAddress)
        if (settings.spaces.isNotEmpty() || settings.canAddTo.isNotEmpty()) {
            Text(
                "Spaces",
                Modifier.padding(start = 16.dp, end = 16.dp, top = 8.dp),
                style = MaterialTheme.typography.bodyLarge,
            )
            FlowRow(
                Modifier.padding(horizontal = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                for (link in settings.spaces) {
                    InputChip(
                        selected = false,
                        onClick = { if (link.canChange) actions.removeFromSpace(link.spaceId) },
                        label = { Text(spaceNames[link.spaceId] ?: link.spaceId) },
                        trailingIcon = if (link.canChange) {
                            { Icon(Icons.Filled.Close, contentDescription = "Take it out of this space", Modifier.size(InputChipDefaults.IconSize)) }
                        } else {
                            null
                        },
                    )
                }
                if (settings.canAddTo.isNotEmpty()) AddToSpace(settings.canAddTo, spaceNames, actions::addToSpace)
            }
        }
        Text(
            factsLine(header.space, settings, memberCount),
            Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (settings.banned.isNotEmpty()) {
            SectionHeading("Banned")
            for (person in settings.banned) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 2.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(Modifier.weight(1f)) {
                        Text(person.user.displayName, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(
                            listOfNotNull(person.user.userId, person.reason).joinToString(" · "),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    if (settings.canUnban) TextButton(onClick = { actions.unban(person.user.userId) }) { Text("Unban") }
                }
            }
        }
        HorizontalDivider(Modifier.padding(top = 8.dp))
    }
    if (encrypting) {
        ConfirmDialog(
            title = "Encrypt ${header.name}?",
            message = "Only members can read messages. Cannot be turned off later.",
            button = "Encrypt",
            onConfirm = {
                encrypting = false
                actions.encrypt()
            },
            onDismiss = { encrypting = false },
        )
    }
}

/** The address: `#` [name] `:server`, and Save once changed. */
@Composable
private fun AddressRow(address: String?, server: String, onSave: (String) -> Unit) {
    val saved = addressName(address)
    var name by remember(saved) { mutableStateOf(saved) }
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text("Address", Modifier.widthIn(min = 80.dp).padding(end = 8.dp), style = MaterialTheme.typography.bodyLarge, maxLines = 1)
        OutlinedTextField(
            value = name,
            onValueChange = { name = it.trim().removePrefix("#") },
            modifier = Modifier.weight(1f),
            singleLine = true,
            prefix = { Text("#") },
            suffix = { Text(":$server", maxLines = 1) },
        )
        TextButton(onClick = { onSave(name) }, enabled = name != saved) { Text("Save") }
    }
}

/** Add to space…: the joined spaces whose rooms we may change. */
@Composable
private fun AddToSpace(spaces: List<String>, spaceNames: Map<String, String>, onAdd: (String) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        AssistChip(
            onClick = { open = true },
            label = { Text("Add to space…") },
            leadingIcon = { Icon(Icons.Filled.Add, contentDescription = null, Modifier.size(18.dp)) },
        )
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            for (space in spaces) {
                DropdownMenuItem(
                    text = { Text(spaceNames[space] ?: space) },
                    onClick = {
                        open = false
                        onAdd(space)
                    },
                )
            }
        }
    }
}

/** The members: each with their role; tapped, the menu of what we may do to them. */
fun LazyListScope.membersSection(members: List<Member>?, images: Images, actions: RoomInfoActions) {
    item(key = "members-heading") {
        SectionHeading(if (members == null) "Members" else "Members · ${members.count { it.membership == Membership.JOINED }}")
    }
    if (members == null) {
        item(key = "members-loading") { Loading() }
        return
    }
    items(members, key = { "member:" + it.user.userId }) { member ->
        MemberRow(member, images, actions)
    }
}

/** A member: avatar, name and role; tapped, [MemberMenu] where we may manage them. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MemberRow(member: Member, images: Images, actions: RoomInfoActions) {
    var menu by remember { mutableStateOf(false) }
    var asking by remember { mutableStateOf<MemberQuestion?>(null) }
    val name = member.user.displayName
    org.mtrx.app.ui.CompactRow(
        title = if (member.own) "$name (you)" else name,
        subtitle = if (member.membership == Membership.INVITED) "Invited" else member.user.userId,
        onClick = if (member.manageable) ({ menu = true }) else null,
        leading = { Avatar(name, member.user.avatarUrl, member.user.userId, images, size = 36.dp) },
        trailing = {
            member.label?.let {
                Text(it, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        },
    )
    if (menu) {
        ModalBottomSheet(onDismissRequest = { menu = false }, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
            MemberMenu(
                member,
                onChoose = { question ->
                    menu = false
                    if (question is MemberQuestion.Level && question.confirmation == null) {
                        actions.setLevel(member.user.userId, question.level)
                    } else {
                        asking = question
                    }
                },
            )
        }
    }
    when (val question = asking) {
        null -> {}
        is MemberQuestion.Level -> {
            val confirmation = question.confirmation
            ConfirmDialog(
                title = confirmation?.title.orEmpty(),
                message = confirmation?.message,
                button = confirmation?.button ?: "Change",
                onConfirm = {
                    asking = null
                    actions.setLevel(member.user.userId, question.level)
                },
                onDismiss = { asking = null },
            )
        }
        MemberQuestion.CustomLevel -> CustomLevelDialog(
            member,
            name,
            onSet = { level ->
                asking = null
                actions.setLevel(member.user.userId, level)
            },
            onDismiss = { asking = null },
        )
        is MemberQuestion.Remove -> RemoveDialog(
            name,
            ban = question.ban,
            onRemove = { reason ->
                asking = null
                actions.remove(member.user.userId, reason, question.ban)
            },
            onDismiss = { asking = null },
        )
    }
}

/** What a member's menu asks next. */
sealed interface MemberQuestion {
    /** [level], asking [confirmation] first if it can't be undone. */
    data class Level(val level: Long, val confirmation: LevelConfirmation?) : MemberQuestion

    data object CustomLevel : MemberQuestion

    data class Remove(val ban: Boolean) : MemberQuestion
}

/**
 * A member's menu: Admin 100, Moderator 50, Member (theirs ticked) where we
 * may change their level, Custom level…, Remove… and Ban….
 */
@Composable
fun MemberMenu(member: Member, onChoose: (MemberQuestion) -> Unit) {
    val name = member.user.displayName
    Column(Modifier.padding(bottom = 16.dp)) {
        Text(
            if (member.own) "$name (you)" else name,
            Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
            style = MaterialTheme.typography.titleMedium,
        )
        val choices = member.choices
        for (choice in choices) {
            MenuLine(
                choice.title,
                trailing = choice.level.toString(),
                checked = choice.current,
                enabled = !choice.current,
                onClick = { onChoose(MemberQuestion.Level(choice.level, choice.confirmation)) },
            )
        }
        if (member.manage.upTo != null) MenuLine("Custom level…", onClick = { onChoose(MemberQuestion.CustomLevel) })
        if ((member.manage.remove || member.manage.ban) && choices.isNotEmpty()) HorizontalDivider(Modifier.padding(vertical = 4.dp))
        if (member.manage.remove) MenuLine("Remove…", destructive = true, onClick = { onChoose(MemberQuestion.Remove(ban = false)) })
        if (member.manage.ban) MenuLine("Ban…", destructive = true, onClick = { onChoose(MemberQuestion.Remove(ban = true)) })
    }
}

@Composable
private fun MenuLine(
    text: String,
    trailing: String? = null,
    checked: Boolean = false,
    enabled: Boolean = true,
    destructive: Boolean = false,
    onClick: () -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 24.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.width(32.dp)) {
            if (checked) Icon(Icons.Filled.Check, contentDescription = "Current", tint = MaterialTheme.colorScheme.primary)
        }
        Text(
            text,
            color = when {
                destructive -> MaterialTheme.colorScheme.error
                enabled || checked -> MaterialTheme.colorScheme.onSurface
                else -> MaterialTheme.colorScheme.onSurfaceVariant
            },
            modifier = Modifier.weight(1f),
        )
        trailing?.let { Text(it, color = MaterialTheme.colorScheme.onSurfaceVariant) }
    }
}

/** A level of our choosing, up to what we may give; warns where it can't be undone. */
@Composable
private fun CustomLevelDialog(member: Member, name: String, onSet: (Long) -> Unit, onDismiss: () -> Unit) {
    var text by remember { mutableStateOf(member.level?.toString().orEmpty()) }
    val upTo = member.manage.upTo ?: return
    val level = typedLevel(text, upTo)
    val warning = level?.let { levelConfirmation(member, it) }
    ConfirmDialog(
        title = if (member.own) "Your power level" else "Power level for $name",
        message = warning?.message,
        button = "Set",
        confirmEnabled = level != null && level != member.level,
        onConfirm = { level?.let(onSet) },
        onDismiss = onDismiss,
    ) {
        OutlinedTextField(
            value = text,
            onValueChange = { text = it },
            singleLine = true,
            label = { Text("Level") },
            supportingText = { Text("Admin 100 · Moderator 50 · Member ${member.manage.defaultLevel}; up to $upTo") },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
        )
    }
}

/** Remove or ban someone, with an optional reason they see. */
@Composable
private fun RemoveDialog(name: String, ban: Boolean, onRemove: (String) -> Unit, onDismiss: () -> Unit) {
    var reason by remember { mutableStateOf("") }
    ConfirmDialog(
        title = if (ban) "Ban $name?" else "Remove $name?",
        message = if (ban) {
            "They can't come back until someone lifts the ban."
        } else {
            "They can come back if the room lets them join or someone invites them."
        },
        button = if (ban) "Ban" else "Remove",
        destructive = true,
        onConfirm = { onRemove(reason) },
        onDismiss = onDismiss,
    ) {
        OutlinedTextField(
            value = reason,
            onValueChange = { reason = it },
            label = { Text("Reason (optional, they see it)") },
        )
    }
}

/** Asks before leaving [name]. */
@Composable
fun LeaveDialog(name: String, onLeave: () -> Unit, onDismiss: () -> Unit) {
    ConfirmDialog(
        title = "Leave $name?",
        message = "You can only come back if the room is public or someone invites you.",
        button = "Leave",
        destructive = true,
        onConfirm = onLeave,
        onDismiss = onDismiss,
    )
}

@Composable
private fun Loading() {
    Box(
        Modifier
            .fillMaxWidth()
            .padding(24.dp),
        contentAlignment = Alignment.Center,
    ) { CircularProgressIndicator() }
}
