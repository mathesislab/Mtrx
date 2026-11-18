package org.mtrx.app.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.DatePicker
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SelectableDates
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TimePicker
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.material3.rememberTimePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZoneOffset
import org.mtrx.app.model.QuietModel
import org.mtrx.app.ui.SectionHeading
import org.mtrx.app.ui.SwitchRow
import uniffi.mtrx.PauseEntry
import uniffi.mtrx.PauseFor
import uniffi.mtrx.QuietState
import uniffi.mtrx.Weekday

/** The days a quiet period may start on, Monday first, with their letters. */
private val DAYS = listOf(
    Weekday.MON to "M",
    Weekday.TUE to "T",
    Weekday.WED to "W",
    Weekday.THU to "T",
    Weekday.FRI to "F",
    Weekday.SAT to "S",
    Weekday.SUN to "S",
)

/** Quiet hours' start or end, being picked. */
private enum class End { FROM, TO }

/**
 * Pausing notifications and quiet hours, on all our devices with Mtrx:
 * the pause (for an hour, until tomorrow morning, until a date and time
 * picked, until turned back on), quiet hours, and whether mentions get
 * through. Opened from Settings → Notifications and from the Chats tab's
 * bell while quiet.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun QuietScreen(quiet: QuietModel, onBack: () -> Unit) {
    val state by quiet.state.collectAsStateWithLifecycle()
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Notifications") },
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
                .verticalScroll(rememberScrollState()),
        ) {
            state?.let { QuietContent(it, quiet) }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun QuietContent(state: QuietState, quiet: QuietModel) {
    var pickingDate by remember { mutableStateOf(false) }
    var pickedDate by remember { mutableStateOf<LocalDate?>(null) }
    var pickingHours by remember { mutableStateOf<End?>(null) }
    val hours = state.hours

    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(state.line, Modifier.weight(1f), style = MaterialTheme.typography.titleMedium)
        if (state.paused) {
            Button(onClick = { quiet.resume() }) { Text("Resume") }
        }
    }
    SectionHeading("Pause notifications")
    for (item in quiet.pauseMenu()) {
        ListItem(
            headlineContent = { Text(item.title) },
            modifier = Modifier.clickable {
                when (val entry = item.entry) {
                    is PauseEntry.Pause -> quiet.pause(entry.pause)
                    PauseEntry.PickTime -> pickingDate = true
                }
            },
        )
    }
    HorizontalDivider(Modifier.padding(vertical = 8.dp))
    SectionHeading("Quiet hours")
    SwitchRow("Quiet hours", hours.on, state.hoursLine) { on -> quiet.setHours(hours.copy(on = on)) }
    ListItem(
        headlineContent = { Text("From") },
        trailingContent = { Text(clock(hours.fromMinute)) },
        modifier = Modifier.clickable { pickingHours = End.FROM },
    )
    ListItem(
        headlineContent = { Text("To") },
        trailingContent = { Text(clock(hours.toMinute)) },
        modifier = Modifier.clickable { pickingHours = End.TO },
    )
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        for ((day, letter) in DAYS) {
            val on = day in hours.days
            FilterChip(
                selected = on,
                onClick = {
                    val days = if (on) hours.days - day else hours.days + day
                    quiet.setHours(hours.copy(days = DAYS.map { it.first }.filter { it in days }))
                },
                label = { Text(letter) },
                modifier = Modifier.weight(1f),
            )
        }
    }
    HorizontalDivider(Modifier.padding(vertical = 8.dp))
    SwitchRow("Let mentions and keywords through", state.mentionsThrough, null) { quiet.setMentionsThrough(it) }
    Text(
        "Mtrx on all your devices. Other Matrix apps aren't paused.",
        Modifier.padding(16.dp),
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )

    if (pickingDate) {
        val today = LocalDate.now()
        val dates = rememberDatePickerState(
            initialSelectedDateMillis = today.atStartOfDay().toInstant(ZoneOffset.UTC).toEpochMilli(),
            selectableDates = object : SelectableDates {
                override fun isSelectableDate(utcTimeMillis: Long): Boolean =
                    !utcDate(utcTimeMillis).isBefore(today)
            },
        )
        DatePickerDialog(
            onDismissRequest = { pickingDate = false },
            confirmButton = {
                TextButton(onClick = {
                    pickingDate = false
                    pickedDate = dates.selectedDateMillis?.let(::utcDate)
                }) { Text("Next") }
            },
            dismissButton = { TextButton(onClick = { pickingDate = false }) { Text("Cancel") } },
        ) { DatePicker(dates) }
    }
    pickedDate?.let { date ->
        val next = LocalTime.now().plusHours(1).withMinute(0)
        TimeDialog(next.hour, next.minute, onDismiss = { pickedDate = null }) { hour, minute ->
            pickedDate = null
            val at = LocalDateTime.of(date, LocalTime.of(hour, minute)).atZone(ZoneId.systemDefault()).toInstant()
            if (at.isAfter(Instant.now())) quiet.pause(PauseFor.At(at))
        }
    }
    pickingHours?.let { end ->
        val minute = if (end == End.FROM) hours.fromMinute else hours.toMinute
        TimeDialog(minute.toInt() / 60, minute.toInt() % 60, onDismiss = { pickingHours = null }) { hour, min ->
            pickingHours = null
            val picked = (hour * 60 + min).toUShort()
            quiet.setHours(if (end == End.FROM) hours.copy(fromMinute = picked) else hours.copy(toMinute = picked))
        }
    }
}

/** A time picked on Material's clock, starting at [hour]:[minute]. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TimeDialog(hour: Int, minute: Int, onDismiss: () -> Unit, onPick: (Int, Int) -> Unit) {
    val time = rememberTimePickerState(initialHour = hour, initialMinute = minute, is24Hour = true)
    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = { TextButton(onClick = { onPick(time.hour, time.minute) }) { Text("OK") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
        text = { TimePicker(time) },
    )
}

/** Minutes after midnight as "22:00". */
fun clock(minute: UShort): String = "%02d:%02d".format(minute.toInt() / 60, minute.toInt() % 60)

/** The date picker's day: midnight UTC of it. */
private fun utcDate(millis: Long): LocalDate = Instant.ofEpochMilli(millis).atZone(ZoneOffset.UTC).toLocalDate()
