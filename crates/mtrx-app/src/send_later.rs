//! Send later: the bar of scheduled messages above the composer, and the
//! picker for a message's send time: quick picks, a calendar and hour and
//! minute wheels, floating above the clock next to Send (or above the
//! scheduled message getting a new time).

use std::time::{Duration, SystemTime};

use chrono::{DateTime, Datelike, Local, Months, NaiveDate, NaiveTime, TimeZone, Timelike};
use iced::{
    Alignment, Background, Border, Element, Length, Task, Theme,
    widget::{
        Id, button, column, container, operation, row, scrollable, space, text, text::Wrapping,
    },
};
use mtrx_core::{OwnedRoomId, ScheduledMessage, describe_send_time, parse_send_time};

use crate::{
    actions::{Editing, InlineEdit},
    app, fonts,
    popover::{Edge, Side, popover},
    room, style,
};

/// The wheels' scrollables, to bring the picked hour and minute into view.
const HOURS: &str = "send-later-hours";
const MINUTES: &str = "send-later-minutes";
const MINUTE_STEP: u32 = 5;
/// Height of a wheel entry, and how many show.
const WHEEL_ROW: f32 = 26.0;
const WHEEL_ROWS: f32 = 7.0;
const DAY_CELL: f32 = 32.0;

#[derive(Debug, Clone)]
pub enum Message {
    /// Pick a time for the composer's text.
    Open,
    Close,
    /// Show the month this many months away.
    Month(i32),
    Day(NaiveDate),
    Hour(u32),
    Minute(u32),
    /// Schedule for the picked time.
    Schedule,
    /// Schedule the composer's text for this time, or move the message whose
    /// time is being changed to it.
    At(SystemTime),
    /// Edit a scheduled message's text in its row.
    Edit(ScheduledMessage),
    /// Pick a new time for a scheduled message.
    ChangeTime(ScheduledMessage),
    SendNow(String),
    Cancel(String),
    /// The composer's text was scheduled: no longer the room's draft, also
    /// once the room was left.
    Scheduled(Result<(), String>),
    /// A scheduled message got its new time.
    Saved(Result<(), String>),
    /// Sending now or cancelling finished.
    Done(Result<(), String>),
}

/// A send time being picked, in the window's popup.
pub struct SendLater {
    pub picker: Picker,
    /// The scheduled message the picker picks a new time for; otherwise it
    /// schedules the composer's text.
    pub rescheduling: Option<ScheduledMessage>,
}

/// A send time being picked on the calendar and wheels.
pub struct Picker {
    /// The month shown, as its first day.
    month: NaiveDate,
    date: NaiveDate,
    hour: u32,
    minute: u32,
}

impl Picker {
    /// Starting at `at`, or at the next full hour.
    pub fn new(now: SystemTime, at: Option<SystemTime>) -> Self {
        let start = at
            .filter(|at| *at > now)
            .map(DateTime::<Local>::from)
            .unwrap_or_else(|| next_hour(DateTime::from(now)));
        Self {
            month: first_of_month(start.date_naive()),
            date: start.date_naive(),
            hour: start.hour(),
            minute: start.minute(),
        }
    }

    /// The picked time, unless it has passed.
    pub fn time(&self, now: SystemTime) -> Result<SystemTime, &'static str> {
        let time = NaiveTime::from_hms_opt(self.hour, self.minute, 0).ok_or("Not a time")?;
        let at: SystemTime = Local
            .from_local_datetime(&self.date.and_time(time))
            .earliest()
            .ok_or("Not a time here (clock change)")?
            .into();
        if at <= now {
            Err("That time has passed")
        } else {
            Ok(at)
        }
    }

    pub fn update(&mut self, message: &Message, now: SystemTime) {
        let today = DateTime::<Local>::from(now).date_naive();
        match message {
            Message::Month(by) => {
                let month = if *by < 0 {
                    self.month
                        .checked_sub_months(Months::new(by.unsigned_abs()))
                } else {
                    self.month.checked_add_months(Months::new(*by as u32))
                };
                if let Some(month) = month.filter(|m| *m >= first_of_month(today)) {
                    self.month = month;
                }
            }
            Message::Day(date) => self.date = *date,
            Message::Hour(hour) => self.hour = *hour,
            Message::Minute(minute) => self.minute = *minute,
            _ => {}
        }
    }

    /// Bring the picked hour and minute into view.
    pub fn scroll(&self) -> Task<app::Message> {
        let offset = |index: u32| operation::AbsoluteOffset {
            x: None,
            y: Some((index as f32 - (WHEEL_ROWS - 1.0) / 2.0).max(0.0) * WHEEL_ROW),
        };
        Task::batch([
            operation::scroll_to(HOURS, offset(self.hour)),
            operation::scroll_to(MINUTES, offset(self.minute / MINUTE_STEP)),
        ])
    }

    /// A month of days, Monday first; past days are disabled.
    fn calendar<'a>(&self, room: &OwnedRoomId, today: NaiveDate) -> Element<'a, app::Message> {
        let nav = |label: &'static str, by: i32, enabled: bool| {
            button(text(label).size(14))
                .padding([0, 8])
                .style(style::text_button)
                .on_press_maybe(enabled.then(|| later(room, Message::Month(by))))
        };
        let header = row![
            nav("‹", -1, self.month > first_of_month(today)),
            text(self.month.format("%B %Y").to_string())
                .size(13)
                .font(fonts::SEMIBOLD)
                .width(Length::Fill)
                .center(),
            nav("›", 1, true),
        ]
        .align_y(Alignment::Center);
        let weekdays = row(["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].map(|day| {
            text(day)
                .size(11)
                .style(text::secondary)
                .width(DAY_CELL)
                .center()
                .into()
        }));
        let mut weeks = column![header, weekdays].spacing(2).width(DAY_CELL * 7.0);
        let lead = self.month.weekday().num_days_from_monday();
        let mut day = self.month - chrono::Days::new(lead.into());
        while day.month() == self.month.month() || day < self.month {
            let mut week = row![];
            for _ in 0..7 {
                week = week.push(if day.month() == self.month.month() {
                    self.day_button(room, day, today)
                } else {
                    space().width(DAY_CELL).into()
                });
                day = day.succ_opt().unwrap_or(day);
            }
            weeks = weeks.push(week);
        }
        weeks.into()
    }

    fn day_button<'a>(
        &self,
        room: &OwnedRoomId,
        day: NaiveDate,
        today: NaiveDate,
    ) -> Element<'a, app::Message> {
        let label = text(day.day().to_string()).size(12).center();
        let label = if day == today {
            label.font(fonts::SEMIBOLD)
        } else {
            label
        };
        let selected = day == self.date;
        button(label.width(Length::Fill))
            .width(DAY_CELL)
            .padding([4, 0])
            .style(move |theme, status| style::wheel_item(theme, status, selected))
            .on_press_maybe((day >= today).then(|| later(room, Message::Day(day))))
            .into()
    }

    /// A scrolling column of hours or minutes; past ones are disabled.
    fn wheel<'a>(
        &self,
        room: &OwnedRoomId,
        id: &'static str,
        values: impl Iterator<Item = u32>,
        picked: u32,
        allowed: impl Fn(u32) -> bool,
        message: fn(u32) -> Message,
    ) -> Element<'a, app::Message> {
        let entries = values.map(|value| {
            let selected = value == picked;
            button(
                text(format!("{value:02}"))
                    .size(13)
                    .center()
                    .width(Length::Fill),
            )
            .height(WHEEL_ROW)
            .padding([4, 0])
            .style(move |theme, status| style::wheel_item(theme, status, selected))
            .on_press_maybe(allowed(value).then(|| later(room, message(value))))
            .into()
        });
        scrollable(column(entries).width(48))
            .id(Id::new(id))
            .height(WHEEL_ROW * WHEEL_ROWS)
            .direction(scrollable::Direction::Vertical(
                scrollable::Scrollbar::new().width(0).scroller_width(0),
            ))
            .into()
    }
}

fn next_hour(now: DateTime<Local>) -> DateTime<Local> {
    let hour = now + chrono::Duration::hours(1);
    let start = NaiveTime::from_hms_opt(hour.hour(), 0, 0).unwrap_or_default();
    Local
        .from_local_datetime(&hour.date_naive().and_time(start))
        .earliest()
        .unwrap_or(hour)
}

fn first_of_month(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

/// A room's scheduled messages, earliest first, as of `now`.
pub struct Pending<'a> {
    pub messages: Vec<&'a ScheduledMessage>,
    pub now: SystemTime,
}

/// The bar of a room's scheduled messages; the one being edited shows its
/// editor (`popup`: the window's, which may be the editor's menu or emoji),
/// the one `rescheduling` its time picker.
pub fn queue<'a>(
    room: &OwnedRoomId,
    pending: &Pending<'a>,
    rescheduling: Option<&'a SendLater>,
    editing: Option<&'a InlineEdit>,
    frequent_emoji: &[String],
    popup: Option<&'a app::Popup>,
) -> Element<'a, app::Message> {
    let (messages, now) = (&pending.messages, pending.now);
    column(messages.iter().map(|message| {
        let edit = editing.filter(|e| e.target == Editing::Scheduled(message.id.clone()));
        let item = item(room, message, edit, frequent_emoji, now, popup);
        let picker = rescheduling
            .filter(|later| {
                later
                    .rescheduling
                    .as_ref()
                    .is_some_and(|m| m.id == message.id)
            })
            .map(|later| picker(room, later, true, now));
        popover(item, picker, Side::Above, Edge::Right)
            .on_dismiss(self::later(room, Message::Close))
            .into()
    }))
    .spacing(4)
    .into()
}

fn item<'a>(
    room: &OwnedRoomId,
    message: &'a ScheduledMessage,
    edit: Option<&'a InlineEdit>,
    frequent_emoji: &[String],
    now: SystemTime,
    popup: Option<&'a app::Popup>,
) -> Element<'a, app::Message> {
    let editing = edit.is_some();
    let when = describe_send_time(message.send_at, now);
    let status = if message.overdue {
        text(format!("Overdue · was due {when}")).font(crate::fonts::SEMIBOLD)
    } else if editing {
        text(format!("Editing · {when}"))
    } else {
        text(format!("{when} · {}", until(message.send_at, now)))
    };
    let preview = message.text.lines().next().unwrap_or_default().to_owned();
    let files = match message.files.as_slice() {
        [] => None,
        [file] => Some(file.filename.clone()),
        files => Some(format!("{} files", files.len())),
    };

    let action = |label, on_press: Option<Message>, primary: bool| {
        button(text(label).size(12))
            .padding([2, 8])
            .style(if primary {
                style::primary_button
            } else {
                style::text_button
            })
            .on_press_maybe(on_press.map(|message| later(room, message)))
    };
    let time_label = if message.overdue {
        "Reschedule"
    } else {
        "Change time"
    };
    let content: Element<'a, app::Message> = match edit {
        Some(edit) => column![status.size(12), edit.view(room, frequent_emoji, popup)]
            .spacing(6)
            .into(),
        None => row![
            status.size(12).width(Length::Shrink),
            crate::ellipsis::ellipsis(preview)
                .size(13)
                .width(Length::Fill),
            files.map(|files| {
                row![
                    crate::icons::paperclip(14.0, |theme| {
                        theme
                            .extended_palette()
                            .background
                            .base
                            .text
                            .scale_alpha(0.55)
                    }),
                    text(files).size(12).style(text::secondary),
                ]
                .spacing(2)
                .align_y(Alignment::Center)
            }),
            row![
                action("Edit", Some(Message::Edit(message.clone())), false),
                action(
                    time_label,
                    Some(Message::ChangeTime(message.clone())),
                    false
                ),
                action(
                    "Send now",
                    Some(Message::SendNow(message.id.clone())),
                    message.overdue,
                ),
                action("Cancel", Some(Message::Cancel(message.id.clone())), false),
            ]
            .spacing(2)
            .align_y(Alignment::Center),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into(),
    };

    let kind = match (message.overdue, editing) {
        (true, _) => Kind::Overdue,
        (false, true) => Kind::Editing,
        (false, false) => Kind::Waiting,
    };
    container(content)
        .padding([4, 10])
        .width(Length::Fill)
        .clip(true)
        .style(move |theme| item_style(theme, kind))
        .into()
}

/// The time picker: quick picks on the left; the calendar and wheels on
/// the right, with the picked time and Schedule below them. Only as wide
/// as needed: it floats over the room.
pub fn picker<'a>(
    room: &OwnedRoomId,
    state: &'a SendLater,
    can_schedule: bool,
    now: SystemTime,
) -> Element<'a, app::Message> {
    let picker = &state.picker;
    let can_schedule = can_schedule || state.rescheduling.is_some();
    let local = DateTime::<Local>::from(now);
    let today = local.date_naive();
    let quick = |label: &'a str, at: Option<SystemTime>| {
        button(text(label).size(13).width(Length::Fill).center())
            .width(150)
            .padding([5, 10])
            .style(style::secondary_button)
            .on_press_maybe(
                at.filter(|_| can_schedule)
                    .map(|at| later(room, Message::At(at))),
            )
    };
    let title = match &state.rescheduling {
        Some(message) => format!(
            "New time for “{}”",
            message.text.lines().next().unwrap_or_default()
        ),
        None => "Send".into(),
    };
    let quick_picks = column![
        text(title)
            .size(12)
            .font(fonts::SEMIBOLD)
            .width(150)
            .wrapping(Wrapping::WordOrGlyph),
        quick("In 1 hour", Some(now + Duration::from_secs(3600))),
        quick("In 3 hours", Some(now + Duration::from_secs(3 * 3600))),
        quick("Tomorrow 09:00", parse_send_time("tomorrow", now)),
        quick("Monday 09:00", parse_send_time("mon", now)),
    ]
    .spacing(6);

    // Past hours and minutes of today are not offered.
    let on_today = picker.date == today;
    let hours = picker.wheel(
        room,
        HOURS,
        0..24,
        picker.hour,
        move |hour| !on_today || hour >= local.hour(),
        Message::Hour,
    );
    let this_hour = on_today && picker.hour == local.hour();
    let minutes = picker.wheel(
        room,
        MINUTES,
        (0..60).step_by(MINUTE_STEP as usize),
        picker.minute,
        move |minute| !this_hour || minute > local.minute(),
        Message::Minute,
    );
    let wheel = |label, wheel| {
        column![text(label).size(11).style(text::secondary), wheel]
            .spacing(4)
            .align_x(Alignment::Center)
    };

    let time = picker.time(now);
    let summary = match time {
        Ok(at) => text(format!(
            "{} · {}",
            describe_send_time(at, now),
            until(at, now)
        ))
        .size(13),
        Err(problem) => text(problem).size(13).style(text::danger),
    };
    let schedule = (can_schedule && time.is_ok()).then(|| later(room, Message::Schedule));
    let chosen = column![
        row![
            picker.calendar(room, today),
            wheel("Hour", hours),
            wheel("Min", minutes)
        ]
        .spacing(16),
        row![
            summary,
            space::horizontal(),
            button(text("Schedule").size(13))
                .padding([5, 12])
                .style(style::primary_button)
                .on_press_maybe(schedule),
            button(text("Close").size(13))
                .padding([5, 12])
                .style(style::text_button)
                .on_press(later(room, Message::Close)),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(10)
    .width(Length::Shrink);

    let content = row![container(quick_picks).center_y(Length::Shrink), chosen]
        .spacing(24)
        .align_y(Alignment::Center);
    container(content).padding(12).style(style::toolbar).into()
}

/// "in 5 min", "in 2 h 10 min", "in 3 days".
fn until(at: SystemTime, now: SystemTime) -> String {
    let minutes = at
        .duration_since(now)
        .unwrap_or_default()
        .as_secs()
        .div_ceil(60);
    match minutes {
        0 => "now".into(),
        1..60 => format!("in {minutes} min"),
        60..1440 if minutes.is_multiple_of(60) => format!("in {} h", minutes / 60),
        60..1440 => format!("in {} h {} min", minutes / 60, minutes % 60),
        _ => match (minutes + 720) / 1440 {
            1 => "in 1 day".into(),
            days => format!("in {days} days"),
        },
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Waiting,
    Editing,
    Overdue,
}

/// Colour hints: neutral while waiting, accent while edited, warning when overdue.
fn item_style(theme: &Theme, kind: Kind) -> container::Style {
    let palette = theme.extended_palette();
    let (background, edge, text_color) = match kind {
        Kind::Waiting => (
            palette.background.weak.color,
            palette.background.strong.color,
            None,
        ),
        Kind::Editing => (
            palette.primary.weak.color.scale_alpha(0.35),
            palette.primary.base.color,
            None,
        ),
        Kind::Overdue => (
            palette.warning.weak.color,
            palette.warning.strong.color,
            Some(palette.warning.weak.text),
        ),
    };
    container::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: Border::default().rounded(6).width(1).color(edge),
        ..Default::default()
    }
}

/// `message` for `room`, as the app gets it.
fn later(room: &OwnedRoomId, message: Message) -> app::Message {
    room::Message::Later(message).to(room)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(date: (i32, u32, u32), hour: u32, minute: u32) -> SystemTime {
        Local
            .with_ymd_and_hms(date.0, date.1, date.2, hour, minute, 0)
            .unwrap()
            .into()
    }

    #[test]
    fn calendar_and_wheels_pick_a_time() {
        let now = local((2026, 10, 20), 14, 23);
        let mut picker = Picker::new(now, None);
        // The next full hour.
        assert_eq!(picker.time(now), Ok(local((2026, 10, 20), 15, 0)));

        picker.update(
            &Message::Day(NaiveDate::from_ymd_opt(2026, 11, 2).unwrap()),
            now,
        );
        picker.update(&Message::Hour(9), now);
        picker.update(&Message::Minute(30), now);
        assert_eq!(picker.time(now), Ok(local((2026, 11, 2), 9, 30)));

        picker.update(
            &Message::Day(NaiveDate::from_ymd_opt(2026, 10, 20).unwrap()),
            now,
        );
        picker.update(&Message::Hour(10), now);
        assert_eq!(picker.time(now), Err("That time has passed"));

        // Starts from a scheduled message's time.
        let later = local((2026, 12, 24), 18, 5);
        let picker_at = Picker::new(now, Some(later));
        assert_eq!(picker_at.time(now), Ok(later));
        assert_eq!(
            picker_at.month,
            NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()
        );

        // No months before this one.
        picker.update(&Message::Month(-1), now);
        assert_eq!(picker.month, NaiveDate::from_ymd_opt(2026, 10, 1).unwrap());
        picker.update(&Message::Month(3), now);
        assert_eq!(picker.month, NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
    }

    #[test]
    fn describes_time_left() {
        let now = SystemTime::UNIX_EPOCH;
        let after = |secs| now + Duration::from_secs(secs);
        assert_eq!(until(after(0), now), "now");
        assert_eq!(until(after(30), now), "in 1 min");
        assert_eq!(until(after(45 * 60), now), "in 45 min");
        assert_eq!(until(after(2 * 3600), now), "in 2 h");
        assert_eq!(until(after(2 * 3600 + 600), now), "in 2 h 10 min");
        assert_eq!(until(after(30 * 3600), now), "in 1 day");
        assert_eq!(until(after(40 * 3600), now), "in 2 days");
    }
}
