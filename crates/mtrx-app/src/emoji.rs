//! Choosing an emoji: the ones used most, then all of them by group or by
//! name.

use emojis::Group;
use iced::{
    Alignment, Element, Length,
    widget::{Id, button, column, container, row, scrollable, text, text_input},
};

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    app, fonts,
    hint::{Position, hint},
    style,
};

/// Offered before any have been used, and to fill up the most used.
const COMMON: [&str; 8] = ["👍", "❤️", "😂", "🎉", "😮", "😢", "🙏", "👀"];
/// Emoji in the quick row.
const QUICK: usize = 8;
const CELL: f32 = 34.0;
const COLUMNS: usize = 9;
/// Search results shown.
const MAX_FOUND: usize = 9 * 12;
/// Newer emoji are left out: fonts of older systems cannot draw them.
const MAX_VERSION: (u32, u32) = (15, 1);
/// The search field, focused when all emoji open.
pub const SEARCH: &str = "emoji-search";

/// Each group, by the emoji on its tab.
const GROUPS: [(Group, &str, &str); 9] = [
    (Group::SmileysAndEmotion, "😀", "Smileys and emotion"),
    (Group::PeopleAndBody, "👋", "People and body"),
    (Group::AnimalsAndNature, "🐻", "Animals and nature"),
    (Group::FoodAndDrink, "🍔", "Food and drink"),
    (Group::TravelAndPlaces, "✈️", "Travel and places"),
    (Group::Activities, "⚽", "Activities"),
    (Group::Objects, "💡", "Objects"),
    (Group::Symbols, "❤️", "Symbols"),
    (Group::Flags, "🏁", "Flags"),
];

#[derive(Debug, Clone)]
pub enum Message {
    /// Show all emoji, or only the most used.
    /// Show all emoji.
    Open,
    Close,
    Search(String),
    Group(Group),
}

/// All emoji, while shown.
pub struct Panel {
    pub query: String,
    pub group: Group,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            query: String::new(),
            group: Group::SmileysAndEmotion,
        }
    }
}

/// Put `emoji` where the cursor is.
pub fn insert(content: &mut iced::widget::text_editor::Content, emoji: &str) {
    use iced::widget::text_editor::{Action, Edit};
    content.perform(Action::Edit(Edit::Paste(std::sync::Arc::new(
        emoji.to_owned(),
    ))));
}

/// A text field with a smiley at its right edge, showing or hiding its
/// emoji panel: a helper for typing, inside the field. The field needs
/// [`FIELD_INSET`] (or [`FIELD_INSET_COMPACT`]) of right padding so text
/// stays clear of it.
pub fn in_field<'a>(
    field: impl Into<Element<'a, app::Message>>,
    open: bool,
    compact: bool,
    message: app::Message,
) -> Element<'a, app::Message> {
    let edge = if compact { 4.0 } else { 6.0 };
    iced::widget::stack![
        field.into(),
        container(toggle(open, compact, message))
            .align_right(Length::Fill)
            .align_bottom(Length::Fill)
            .padding(iced::Padding::new(edge).top(0)),
    ]
    .into()
}

/// The smiley of [`in_field`], showing or hiding the emoji panel.
pub fn toggle<'a>(open: bool, compact: bool, message: app::Message) -> Element<'a, app::Message> {
    let size = if compact { 15.0 } else { 19.0 };
    hint(
        button(crate::icons::smiley(size, |theme| {
            theme
                .extended_palette()
                .background
                .base
                .text
                .scale_alpha(0.55)
        }))
        .padding(4)
        .style(style::list_item(open))
        // While open, a click on it closes the panel as any outside click.
        .on_press_maybe((!open).then_some(message)),
        "Insert an emoji",
        Position::Top,
    )
}

/// Right padding of a text field with [`in_field`]'s smiley.
pub const FIELD_INSET: f32 = 38.0;
pub const FIELD_INSET_COMPACT: f32 = 30.0;

/// The quick row: the most used, filled up with common ones.
pub fn quick(frequent: &[String]) -> Vec<&str> {
    let mut quick: Vec<&str> = frequent.iter().map(String::as_str).take(QUICK).collect();
    for emoji in COMMON {
        if quick.len() == QUICK {
            break;
        }
        if !quick.contains(&emoji) {
            quick.push(emoji);
        }
    }
    quick
}

impl Panel {
    pub fn update(&mut self, message: Message) {
        match message {
            Message::Search(query) => self.query = query,
            Message::Group(group) => {
                self.group = group;
                self.query.clear();
            }
            Message::Open | Message::Close => {}
        }
    }

    /// The emoji shown: matching the search, else the chosen group's.
    fn shown(&self) -> Vec<&'static str> {
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return self
                .group
                .emojis()
                .filter(|e| drawable(e))
                .map(|e| e.as_str())
                .collect();
        }
        // At the start of a word: "ice" finds ice cream, not police.
        let starts_word = |text: &str| {
            text.starts_with(&query)
                || text.contains(&format!(" {query}"))
                || text.contains(&format!("_{query}"))
        };
        emojis::iter()
            .filter(|e| drawable(e))
            .filter(|e| starts_word(e.name()) || e.shortcodes().any(starts_word))
            .take(MAX_FOUND)
            .map(|e| e.as_str())
            .collect()
    }

    /// The most used (if given), search, group tabs and a grid of emoji;
    /// `pick` makes the message for a chosen one.
    pub fn view<'a>(
        &'a self,
        frequent: &[String],
        pick: impl Fn(String) -> app::Message,
        wrap: impl Fn(Message) -> app::Message + Clone + 'a,
    ) -> Element<'a, app::Message> {
        let cell = |emoji: &str| -> Element<'a, app::Message> {
            button(
                text(emoji.to_owned())
                    .size(20)
                    .font(fonts::EMOJI)
                    .center()
                    .width(Length::Fill),
            )
            .width(CELL)
            .height(CELL)
            .padding(0)
            .style(style::list_item(false))
            .on_press(pick(emoji.to_owned()))
            .into()
        };
        let search = text_input("Search emoji", &self.query)
            .id(Id::new(SEARCH))
            .on_input({
                let wrap = wrap.clone();
                move |query| wrap(Message::Search(query))
            })
            .style(style::input)
            .padding([4, 8])
            .size(13);
        let searching = !self.query.trim().is_empty();
        let tabs = row(GROUPS.map(|(group, icon, name)| {
            hint(
                button(
                    text(icon)
                        .size(16)
                        .font(fonts::EMOJI)
                        .center()
                        .width(Length::Fill),
                )
                .width(CELL)
                .padding([2, 0])
                .style(style::list_item(!searching && group == self.group))
                .on_press(wrap(Message::Group(group))),
                name,
                Position::Top,
            )
        }));
        let shown = self.shown();
        let grid: Element<'a, app::Message> = if shown.is_empty() {
            text("No emoji found")
                .size(12)
                .style(text::secondary)
                .into()
        } else {
            column(
                shown
                    .chunks(COLUMNS)
                    .map(|line| row(line.iter().map(|emoji| cell(emoji))).into()),
            )
            .into()
        };
        let width = CELL * COLUMNS as f32 + 12.0;
        let mut panel = column![].spacing(6).align_x(Alignment::Start);
        if !frequent.is_empty() {
            panel = panel.push(row(quick(frequent).into_iter().map(cell)));
        }
        container(
            panel
                .push(search.width(width))
                .push(tabs)
                .push(scrollable(grid).height(CELL * 6.0).width(width)),
        )
        .padding(6)
        .style(style::toolbar)
        .into()
    }
}

/// `text` split into runs of emoji and of other text, in order, so emoji
/// can be drawn with the emoji font. Symbols from U+2600 on (❤, ☀) count
/// even without the emoji variation selector; earlier ones (©, ↔) only
/// with it.
pub fn runs(text: &str) -> Vec<(bool, String)> {
    let mut runs: Vec<(bool, String)> = Vec::new();
    for grapheme in text.graphemes(true) {
        let emoji = is_emoji(grapheme);
        match runs.last_mut() {
            Some((kind, run)) if *kind == emoji => run.push_str(grapheme),
            _ => runs.push((emoji, grapheme.to_owned())),
        }
    }
    runs
}

/// The emoji of `text`, in order, one for each use: what sending it uses.
pub fn used(text: &str) -> Vec<String> {
    text.graphemes(true)
        .filter(|g| is_emoji(g))
        .map(str::to_owned)
        .collect()
}

/// The emoji of `new` beyond those `old` has, one for each use: what an
/// edit adds.
pub fn added(old: &str, new: &str) -> Vec<String> {
    let mut old = used(old);
    used(new)
        .into_iter()
        .filter(|emoji| match old.iter().position(|o| o == emoji) {
            Some(at) => {
                old.swap_remove(at);
                false
            }
            None => true,
        })
        .collect()
}

/// See [`runs`].
fn is_emoji(grapheme: &str) -> bool {
    let pictographic =
        grapheme.contains('\u{fe0f}') || grapheme.chars().next().is_some_and(|c| c >= '\u{2600}');
    pictographic
        && (emojis::get(grapheme).is_some()
            || emojis::get(&format!("{grapheme}\u{fe0f}")).is_some())
}

fn drawable(emoji: &emojis::Emoji) -> bool {
    let version = emoji.unicode_version();
    (version.major(), version.minor()) <= MAX_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_emoji_from_text() {
        let runs = |text| {
            runs(text)
                .into_iter()
                .map(|(emoji, run)| format!("{}{run}", if emoji { "E:" } else { "" }))
                .collect::<Vec<_>>()
        };
        assert_eq!(runs("Love it ❤ 👍🏽!"), ["Love it ", "E:❤", " ", "E:👍🏽", "!"]);
        assert_eq!(runs("© 2026 → done ✅"), ["© 2026 → done ", "E:✅"]);
        assert_eq!(runs("👩‍🔬🧊"), ["E:👩‍🔬🧊"]);
        assert!(runs("").is_empty());
    }

    #[test]
    fn emoji_sent_and_added() {
        assert_eq!(used("Done ✅ 🎉🎉 → 👍🏽 ©"), ["✅", "🎉", "🎉", "👍🏽"]);
        assert!(used("No emoji :)").is_empty());
        assert_eq!(added("Nice 🎉", "Nice 🎉🎉 👩‍🔬"), ["🎉", "👩‍🔬"]);
        assert!(added("🎉 ok", "ok").is_empty());
    }

    #[test]
    fn most_used_then_common() {
        assert_eq!(quick(&[]), COMMON);
        let used = ["🦀".to_owned(), "👍".to_owned()];
        assert_eq!(
            quick(&used),
            ["🦀", "👍", "❤️", "😂", "🎉", "😮", "😢", "🙏"]
        );
    }

    #[test]
    fn finds_emoji_by_name_and_shortcode() {
        let mut panel = Panel::default();
        panel.update(Message::Search("crab".into()));
        assert!(panel.shown().contains(&"🦀"));
        panel.update(Message::Search("tada".into()));
        assert!(panel.shown().contains(&"🎉"));
        panel.update(Message::Search("ice".into()));
        let found = panel.shown();
        assert!(found.contains(&"🧊") && found.contains(&"🍦"));
        assert!(!found.contains(&"👮"), "police");
        // Too new for most fonts.
        panel.update(Message::Search("cracking face".into()));
        assert!(panel.shown().is_empty());
        panel.update(Message::Group(Group::Flags));
        assert!(panel.query.is_empty() && panel.shown().len() > 100);
    }
}
