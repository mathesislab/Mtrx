//! The bell at the bottom of the space bar: pausing notifications and
//! quiet hours, on all our devices with Mtrx (see `docs/quiet.md`). Its
//! popover pauses (for an hour, until tomorrow morning, until a time picked
//! on send later's calendar and wheels, until turned back on), sets the
//! quiet hours and lets mentions through. The library decides; this shows
//! what it says.

use std::{sync::Arc, time::SystemTime};

use chrono::{NaiveTime, Timelike, Weekday};
use iced::{
    Alignment, Color, Element, Length, Subscription, Task, Theme,
    widget::{button, column, container, row, rule, space, text, toggler},
};
use mtrx_core::{
    PauseEntry, PauseFor, QuietHours, QuietState, QuietStatus, Session, describe_send_time,
    pause_menu,
};

use crate::{
    app::{self, Cx, Popup, Shared},
    fonts,
    hint::{Position, hint},
    icons,
    popover::{Edge, Side, popover},
    send_later::{self, Picker, labelled, scroll_wheels, wheel},
    style, task,
};

/// The popover's width; picking a time, wide enough for the calendar and
/// wheels.
const PANEL_WIDTH: f32 = 300.0;
const PICKING_WIDTH: f32 = 400.0;
const DAYS: [(Weekday, &str); 7] = [
    (Weekday::Mon, "M"),
    (Weekday::Tue, "T"),
    (Weekday::Wed, "W"),
    (Weekday::Thu, "T"),
    (Weekday::Fri, "F"),
    (Weekday::Sat, "S"),
    (Weekday::Sun, "S"),
];

#[derive(Debug, Clone)]
pub enum Message {
    /// Open the popover, or close it.
    Toggle,
    Pause(PauseFor),
    /// "Until…": pick a date and time.
    PickTime,
    /// The calendar's or the wheels' change to the time picked.
    Picker(send_later::Message),
    /// Pause until the time picked.
    PauseUntilPicked,
    Resume,
    /// Pick when quiet hours start, or end, on the wheels; again to stop.
    PickHours(End),
    Hours(QuietHours),
    MentionsThrough(bool),
    /// From the library: now, and after every change.
    Updated(QuietState),
    /// A change was saved, or not.
    Saved(Result<(), String>),
}

/// The start or end of quiet hours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    From,
    To,
}

/// What the popover picks on wheels, in the window's popup.
pub enum Picking {
    /// When a pause ends.
    Until(Picker),
    /// When quiet hours start or end.
    Hours(End),
}

/// Whether notifications are held back, as the library last said.
pub struct Quiet {
    state: QuietState,
    error: Option<String>,
}

/// The state now, then every change.
pub fn subscription(session: &Arc<Session>) -> Subscription<app::Message> {
    Subscription::run_with((Shared(session.clone()), "quiet"), |(s, _)| {
        s.0.quiet_updates().updates()
    })
    .map(|state| app::Message::Quiet(Message::Updated(state)))
}

impl Quiet {
    pub fn new(session: &Session) -> Self {
        Self {
            state: session.quiet(),
            error: None,
        }
    }

    pub fn update(&mut self, message: Message, cx: &mut Cx<'_>) -> Task<app::Message> {
        let session = cx.session.clone();
        let saved = |result| app::Message::Quiet(Message::Saved(result));
        let picking = match cx.popup {
            Some(Popup::Quiet { picking }) => Some(picking),
            _ => None,
        };
        match message {
            Message::Toggle => {
                *cx.popup = match cx.popup {
                    Some(Popup::Quiet { .. }) => None,
                    _ => Some(Popup::Quiet { picking: None }),
                };
                self.error = None;
                Task::none()
            }
            Message::Pause(pause) => {
                if let Some(picking) = picking {
                    *picking = None;
                }
                task::request(
                    async move { session.pause_notifications(pause).await },
                    saved,
                )
            }
            Message::PickTime => {
                let Some(picking) = picking else {
                    return Task::none();
                };
                let picker = Picker::new(SystemTime::now(), None);
                let scroll = picker.scroll();
                *picking = Some(Picking::Until(picker));
                scroll
            }
            Message::Picker(message) => {
                if let Some(Some(Picking::Until(picker))) = picking {
                    picker.update(&message, SystemTime::now());
                }
                Task::none()
            }
            Message::PauseUntilPicked => {
                let at = match picking {
                    Some(Some(Picking::Until(picker))) => picker.time(SystemTime::now()).ok(),
                    _ => None,
                };
                match at {
                    Some(at) => self.update(Message::Pause(PauseFor::At(at)), cx),
                    None => Task::none(),
                }
            }
            Message::Resume => {
                task::request(async move { session.resume_notifications().await }, saved)
            }
            Message::PickHours(end) => {
                let Some(picking) = picking else {
                    return Task::none();
                };
                if matches!(picking, Some(Picking::Hours(open)) if *open == end) {
                    *picking = None;
                    return Task::none();
                }
                *picking = Some(Picking::Hours(end));
                let hours = self.state.quiet.hours;
                let time = match end {
                    End::From => hours.from,
                    End::To => hours.to,
                };
                scroll_wheels(time.hour(), time.minute())
            }
            Message::Hours(hours) => {
                task::request(async move { session.set_quiet_hours(hours).await }, saved)
            }
            Message::MentionsThrough(on) => {
                task::request(async move { session.set_mentions_through(on).await }, saved)
            }
            Message::Updated(state) => {
                self.state = state;
                Task::none()
            }
            Message::Saved(result) => {
                self.error = result.err();
                Task::none()
            }
        }
    }

    /// The bell, with its popover while `popup` is it: an outline while
    /// notifications are on, struck through in the accent colour while
    /// paused or in quiet hours. Its hint says which.
    pub fn view<'a>(&'a self, popup: Option<&'a Popup>) -> Element<'a, app::Message> {
        let picking = match popup {
            Some(Popup::Quiet { picking }) => Some(picking.as_ref()),
            _ => None,
        };
        let open = picking.is_some();
        let icon = if self.state.status == QuietStatus::On {
            icons::bell(20.0, faint)
        } else {
            icons::bell_slash(20.0, accent)
        };
        // While open, a click on it closes the popover as any outside click.
        let bell = button(icon)
            .padding(8)
            .style(style::list_item(open))
            .on_press_maybe((!open).then_some(app::Message::Quiet(Message::Toggle)));
        let bell = hint(bell, self.state.line.as_str(), Position::Right);
        let width = match picking {
            Some(Some(Picking::Until(_))) => PICKING_WIDTH,
            _ => PANEL_WIDTH,
        };
        let panel = picking.map(|picking| self.panel(picking, SystemTime::now()));
        popover(bell, panel, Side::Above, Edge::Left)
            .width(width)
            .on_dismiss(app::Message::ClosePopup)
            .into()
    }

    /// The state, the pause, quiet hours, mentions, and what it can't do.
    fn panel<'a>(
        &'a self,
        picking: Option<&'a Picking>,
        now: SystemTime,
    ) -> Element<'a, app::Message> {
        let state = &self.state;
        let quiet = app::Message::Quiet;
        let mut top = row![text(state.line.as_str()).size(14).font(fonts::SEMIBOLD)]
            .align_y(Alignment::Center);
        if matches!(state.status, QuietStatus::Paused { .. }) {
            top = top.push(space::horizontal()).push(
                button(text("Resume").size(13))
                    .padding([4, 12])
                    .style(style::primary_button)
                    .on_press(quiet(Message::Resume)),
            );
        }
        let mut panel = column![top.width(Length::Fill)].spacing(8);

        panel = panel.push(heading("Pause notifications"));
        let entries = pause_menu(now)
            .into_iter()
            .map(|(entry, title)| -> Element<'a, _> {
                let message = match entry {
                    PauseEntry::Pause(pause) => Message::Pause(pause),
                    PauseEntry::PickTime => Message::PickTime,
                };
                button(text(title).size(13))
                    .width(Length::Fill)
                    .padding([4, 8])
                    .style(style::list_item(false))
                    .on_press(quiet(message))
                    .into()
            });
        panel = panel.push(column(entries).spacing(0));
        if let Some(Picking::Until(picker)) = picking {
            panel = panel.push(until_picker(picker, now));
        }

        panel = panel.push(separator());
        let hours = state.quiet.hours;
        panel = panel.push(
            row![
                heading("Quiet hours"),
                space::horizontal(),
                toggler(hours.on)
                    .on_toggle(move |on| quiet(Message::Hours(QuietHours { on, ..hours }))),
            ]
            .align_y(Alignment::Center),
        );
        let picked = match picking {
            Some(Picking::Hours(end)) => Some(*end),
            _ => None,
        };
        let time_button = |label: &'static str, end: End, time: NaiveTime| {
            row![
                text(label).size(13).style(text::secondary),
                button(text(time.format("%H:%M").to_string()).size(13))
                    .padding([3, 10])
                    .style(style::list_item(picked == Some(end)))
                    .on_press(quiet(Message::PickHours(end))),
            ]
            .spacing(4)
            .align_y(Alignment::Center)
        };
        panel = panel.push(
            row![
                time_button("From", End::From, hours.from),
                time_button("To", End::To, hours.to),
            ]
            .spacing(16),
        );
        if let Some(end) = picked {
            panel = panel.push(hours_wheels(hours, end));
        }
        let days = DAYS.map(|(day, letter)| -> Element<'a, _> {
            let on = hours.days.contains(day);
            let mut days = hours.days;
            if on {
                days.remove(day);
            } else {
                days.insert(day);
            }
            button(text(letter).size(12).center().width(Length::Fill))
                .width(28)
                .padding([4, 0])
                .style(move |theme, status| style::wheel_item(theme, status, on))
                .on_press(quiet(Message::Hours(QuietHours { days, ..hours })))
                .into()
        });
        panel = panel.push(row(days).spacing(4));
        panel = panel.push(
            text(state.hours_line.as_str())
                .size(12)
                .style(text::secondary),
        );

        panel = panel.push(separator());
        panel = panel.push(
            row![
                text("Let mentions and keywords through")
                    .size(13)
                    .width(Length::Fill),
                toggler(state.quiet.mentions_through)
                    .on_toggle(move |on| quiet(Message::MentionsThrough(on))),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
        panel = panel.push(
            text("Mtrx on all your devices. Other Matrix apps aren't paused.")
                .size(12)
                .style(text::secondary),
        );
        if let Some(error) = &self.error {
            panel = panel.push(text(error.as_str()).size(12).style(text::danger));
        }
        container(panel).padding(12).style(style::dialog).into()
    }
}

/// "Until…": the calendar and wheels, the time picked and Pause.
fn until_picker<'a>(picker: &'a Picker, now: SystemTime) -> Element<'a, app::Message> {
    let chooser = picker.chooser(now, |message| app::Message::Quiet(Message::Picker(message)));
    let time = picker.time(now);
    let summary = match time {
        Ok(at) => text(describe_send_time(at, now)).size(13),
        Err(problem) => text(problem).size(13).style(text::danger),
    };
    let pause = button(text("Pause").size(13))
        .padding([5, 12])
        .style(style::primary_button)
        .on_press_maybe(
            time.is_ok()
                .then_some(app::Message::Quiet(Message::PauseUntilPicked)),
        );
    column![
        chooser,
        row![summary, space::horizontal(), pause].align_y(Alignment::Center),
    ]
    .spacing(8)
    .into()
}

/// The hour and minute wheels for when quiet hours start or end.
fn hours_wheels<'a>(hours: QuietHours, end: End) -> Element<'a, app::Message> {
    let time = match end {
        End::From => hours.from,
        End::To => hours.to,
    };
    let set = move |hour: u32, minute: u32| {
        let time = NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or(time);
        let hours = match end {
            End::From => QuietHours {
                from: time,
                ..hours
            },
            End::To => QuietHours { to: time, ..hours },
        };
        app::Message::Quiet(Message::Hours(hours))
    };
    let (hour, minute) = (time.hour(), time.minute());
    row![
        labelled(
            "Hour",
            wheel(true, hour, |_| true, move |hour| set(hour, minute))
        ),
        labelled(
            "Min",
            wheel(false, minute, |_| true, move |minute| set(hour, minute))
        ),
    ]
    .spacing(16)
    .into()
}

fn heading<'a>(label: &'a str) -> Element<'a, app::Message> {
    text(label).size(13).font(fonts::SEMIBOLD).into()
}

fn separator<'a>() -> Element<'a, app::Message> {
    rule::horizontal(1).style(style::separator).into()
}

/// The bell while notifications are on: faint, like the gear.
fn faint(theme: &Theme) -> Color {
    theme
        .extended_palette()
        .background
        .base
        .text
        .scale_alpha(0.7)
}

/// The bell while notifications are held back.
fn accent(theme: &Theme) -> Color {
    theme.extended_palette().primary.base.color
}
