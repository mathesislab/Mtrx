//! Disappearing messages in the app (see `docs/disappearing.md`): the
//! composer's choice beside Send, the hourglass on such messages, and the
//! choices for a room's mode and the limit.

use std::{
    fmt,
    time::{Duration, SystemTime},
};

use iced::{
    Element,
    widget::{button, container},
};
use mtrx_core::{Disappearing, Event, OwnedRoomId};

use crate::{
    app,
    hint::{Position, hint},
    icons,
    menu::{self, Entry, Item, Width},
    popover::{Edge, Side, popover},
    room, style,
};

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

/// How long after everyone read it a message may disappear.
pub const AFTER_READ: [Duration; 4] = [
    Duration::from_secs(MINUTE),
    Duration::from_secs(5 * MINUTE),
    Duration::from_secs(HOUR),
    Duration::from_secs(DAY),
];

/// How long after sending one goes at the latest, as Settings offers it.
pub const AT_MOST: [Duration; 4] = [
    Duration::from_secs(HOUR),
    Duration::from_secs(6 * HOUR),
    Duration::from_secs(DAY),
    Duration::from_secs(7 * DAY),
];

/// `duration` in words, in its largest whole unit: "5 minutes", "1 day".
pub fn words(duration: Duration) -> String {
    let secs = duration.as_secs();
    let (count, unit) = [(DAY, "day"), (HOUR, "hour"), (MINUTE, "minute")]
        .into_iter()
        .find(|(unit, _)| secs >= *unit && secs.is_multiple_of(*unit))
        .map_or((secs, "second"), |(size, unit)| (secs / size, unit));
    plural(count, unit)
}

/// The time left until `due`, roughly: "3 minutes", "less than a minute".
fn left(due: SystemTime, now: SystemTime) -> String {
    let secs = due.duration_since(now).unwrap_or_default().as_secs();
    if secs < MINUTE {
        "less than a minute".to_owned()
    } else if secs < HOUR {
        plural(secs.div_ceil(MINUTE), "minute")
    } else if secs < DAY {
        plural(secs.div_ceil(HOUR), "hour")
    } else {
        plural(secs.div_ceil(DAY), "day")
    }
}

fn plural(count: u64, unit: &str) -> String {
    match count {
        1 => format!("1 {unit}"),
        n => format!("{n} {unit}s"),
    }
}

/// A room's mode in its settings' menu: off, or the time after read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode(pub Option<Duration>);

impl Mode {
    pub fn all() -> Vec<Mode> {
        std::iter::once(Mode(None))
            .chain(AFTER_READ.map(|after| Mode(Some(after))))
            .collect()
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            None => f.write_str("Off"),
            Some(after) => write!(f, "{} after read", words(after)),
        }
    }
}

/// The limit in Settings' menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit(pub Duration);

impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&words(self.0))
    }
}

/// The hourglass beside Send: lit while what is sent disappears
/// `after_read`; a click opens the menu to choose, `open` while it is.
pub fn composer_button<'a>(
    room: &OwnedRoomId,
    after_read: Option<Duration>,
    open: bool,
) -> Element<'a, app::Message> {
    let icon = match after_read {
        Some(_) => icons::hourglass(18.0, |theme| theme.extended_palette().primary.base.color),
        None => icons::hourglass(18.0, |theme| {
            theme
                .extended_palette()
                .background
                .base
                .text
                .scale_alpha(0.75)
        }),
    };
    let label = match after_read {
        Some(after) => format!("Disappears {} after everyone has read it", words(after)),
        None => "Disappearing messages: off".to_owned(),
    };
    // While open, a click on it closes the menu as any outside click.
    let button = hint(
        button(icon)
            .padding([9, 9])
            .style(style::secondary_button)
            .on_press_maybe((!open).then(|| room::Message::DisappearMenu.to(room))),
        label,
        Position::Top,
    );
    let menu = open.then(|| menu::view(entries(room, after_read), Width::Menu));
    popover(button, menu, Side::Above, Edge::Right)
        .on_dismiss(app::Message::ClosePopup)
        .into()
}

/// Off, or each time after read; the current one ticked.
fn entries(room: &OwnedRoomId, after_read: Option<Duration>) -> Vec<Entry<'static, app::Message>> {
    let choose = |choice| room::Message::Disappear(choice).to(room);
    let mut entries = vec![Entry::Heading(
        "Disappear after everyone read it".to_owned(),
    )];
    entries.push(
        Item::new("Off", choose(None))
            .checked(after_read.is_none())
            .into(),
    );
    for after in AFTER_READ {
        entries.push(
            Item::new(words(after), choose(Some(after)))
                .checked(after_read == Some(after))
                .into(),
        );
    }
    entries
}

/// The hourglass in the margin of a message that disappears, saying when:
/// ours by `listed`, the list of those waiting, others' by their marker.
pub fn mark<'a>(
    event: &Event,
    listed: &[Disappearing],
    now: SystemTime,
) -> Option<Element<'a, app::Message>> {
    let entry = (event.event_id.as_deref())
        .and_then(|id| listed.iter().find(|entry| entry.event_id() == Some(id)));
    let label = match (entry, event.disappear) {
        (Some(entry), _) => match (entry.read_at, entry.due()) {
            (Some(_), Some(due)) => {
                format!("Everyone has read it: disappears in {}", left(due, now))
            }
            _ => format!(
                "Disappears {} after everyone has read it, at the latest {} after sending",
                words(entry.disappear.after_read),
                words(entry.disappear.at_most),
            ),
        },
        (None, Some(disappear)) if event.own => format!(
            "Disappears {} after everyone has read it",
            words(disappear.after_read)
        ),
        (None, Some(_)) => "Disappears after it was read".to_owned(),
        (None, None) => return None,
    };
    let icon = container(icons::hourglass(12.0, style::muted)).padding([2, 0]);
    Some(hint(icon, label, Position::Left))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_in_words() {
        let secs = Duration::from_secs;
        assert_eq!(words(secs(60)), "1 minute");
        assert_eq!(words(secs(300)), "5 minutes");
        assert_eq!(words(secs(3600)), "1 hour");
        assert_eq!(words(secs(6 * 3600)), "6 hours");
        assert_eq!(words(secs(86400)), "1 day");
        assert_eq!(words(secs(7 * 86400)), "7 days");
        assert_eq!(words(secs(90)), "90 seconds");
    }

    #[test]
    fn time_left_rounds_up() {
        let now = SystemTime::UNIX_EPOCH;
        let at = |s| now + Duration::from_secs(s);
        assert_eq!(left(at(30), now), "less than a minute");
        assert_eq!(left(at(61), now), "2 minutes");
        assert_eq!(left(at(3600), now), "1 hour");
        assert_eq!(left(at(3601), now), "2 hours");
        assert_eq!(left(now, at(5)), "less than a minute", "overdue");
    }

    #[test]
    fn modes_read_as_menu_items() {
        let modes: Vec<String> = Mode::all().iter().map(ToString::to_string).collect();
        assert_eq!(
            modes,
            [
                "Off",
                "1 minute after read",
                "5 minutes after read",
                "1 hour after read",
                "1 day after read"
            ]
        );
    }
}
