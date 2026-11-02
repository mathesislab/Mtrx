package org.mtrx.app.ui

import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

private val time = DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT)
private val weekday = DateTimeFormatter.ofPattern("EEE")
private val dayMonth = DateTimeFormatter.ofPattern("d MMM")
private val shortDate = DateTimeFormatter.ofLocalizedDate(FormatStyle.SHORT)
private val longDay = DateTimeFormatter.ofPattern("EEEE d MMMM")
private val longDate = DateTimeFormatter.ofPattern("d MMMM yyyy")

/** A message's time, e.g. 14:05. */
fun timeOfDay(instant: Instant, zone: ZoneId = ZoneId.systemDefault()): String =
    instant.atZone(zone).format(time)

/** When a room was last active: the time today, else the weekday, day or date. */
fun lastActivity(instant: Instant, today: LocalDate, zone: ZoneId = ZoneId.systemDefault()): String {
    val at = instant.atZone(zone)
    val date = at.toLocalDate()
    return when {
        date == today -> at.format(time)
        date.isAfter(today.minusDays(7)) && !date.isAfter(today) -> date.format(weekday)
        date.year == today.year -> date.format(dayMonth)
        else -> date.format(shortDate)
    }
}

/** A day divider: Today, Yesterday, or the date. */
fun dayLabel(date: LocalDate, today: LocalDate): String = when {
    date == today -> "Today"
    date == today.minusDays(1) -> "Yesterday"
    date.year == today.year -> date.format(longDay)
    else -> date.format(longDate)
}
