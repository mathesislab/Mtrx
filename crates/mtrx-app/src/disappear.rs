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
use mtrx_core::{
    AFTER_READ_CHOICES, Disappearing, Event, OwnedRoomId, disappear_words, disappearing_mode_words,
    duration_words,
};

use crate::{
    app,
    hint::{Position, hint},
    icons,
    menu::{self, Entry, Item, Width},
    popover::{Edge, Side, popover},
    room, style,
};

/// A room's mode in its settings' menu: off, or the time after read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode(pub Option<Duration>);

impl Mode {
    pub fn all() -> Vec<Mode> {
        std::iter::once(Mode(None))
            .chain(AFTER_READ_CHOICES.map(|after| Mode(Some(after))))
            .collect()
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&disappearing_mode_words(self.0))
    }
}

/// The limit in Settings' menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit(pub Duration);

impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&duration_words(self.0))
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
    let label = disappear_words(after_read);
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
    for after in AFTER_READ_CHOICES {
        entries.push(
            Item::new(duration_words(after), choose(Some(after)))
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
        (Some(entry), _) => entry.words(now),
        (None, Some(disappear)) => disappear.words(event.own),
        (None, None) => return None,
    };
    let icon = container(icons::hourglass(12.0, style::muted)).padding([2, 0]);
    Some(hint(icon, label, Position::Left))
}

#[cfg(test)]
mod tests {
    use super::*;

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
