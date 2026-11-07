//! The window's menus: entries built by plain functions, shown in one
//! look. Where a menu floats stays with its caller: at the cursor
//! ([`crate::context_menu::context_menu`]), below a button
//! ([`crate::popover::popover`]), or in the room list's flow.

use iced::{
    Alignment, Element, Length,
    widget::{button, column, container, row, rule, space, text},
};

use crate::{
    ellipsis::ellipsis,
    hint::{Position, hint},
    icons, style,
};

/// The usual width of a menu.
const MENU_WIDTH: f32 = 170.0;
/// The size of a tick, and the room kept for it in unticked items.
const TICK: f32 = 12.0;

/// One line of a menu.
pub enum Entry<'a, M> {
    Item(Item<M>),
    Heading(String),
    Separator,
    /// Rows of its own: the calls menu's reorderable lists.
    Custom(Element<'a, M>),
}

/// Something to choose.
pub struct Item<M> {
    pub label: String,
    /// `None`: shown, but disabled.
    pub on_choose: Option<M>,
    /// A tick, or room for one: the role menu's.
    pub checked: Option<bool>,
    /// At the right, quieter: the role menu's level.
    pub detail: Option<String>,
    /// Shown on hover: the calls menu's services.
    pub hint: Option<String>,
}

impl<M> Item<M> {
    pub fn new(label: impl Into<String>, on_choose: M) -> Self {
        Self {
            label: label.into(),
            on_choose: Some(on_choose),
            checked: None,
            detail: None,
            hint: None,
        }
    }

    pub fn disabled(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            on_choose: None,
            checked: None,
            detail: None,
            hint: None,
        }
    }

    pub fn checked(self, on: bool) -> Self {
        Self {
            checked: Some(on),
            ..self
        }
    }

    pub fn detail(self, text: impl Into<String>) -> Self {
        Self {
            detail: Some(text.into()),
            ..self
        }
    }

    pub fn hint(self, text: impl Into<String>) -> Self {
        Self {
            hint: Some(text.into()),
            ..self
        }
    }

    /// The same item, choosing `f` of its message.
    pub fn map<N>(self, f: impl FnOnce(M) -> N) -> Item<N> {
        Item {
            label: self.label,
            on_choose: self.on_choose.map(f),
            checked: self.checked,
            detail: self.detail,
            hint: self.hint,
        }
    }
}

impl<'a, M> From<Item<M>> for Entry<'a, M> {
    fn from(item: Item<M>) -> Self {
        Entry::Item(item)
    }
}

impl<'a, M: 'a> Entry<'a, M> {
    /// The same entry, sending `f` of its messages.
    pub fn map<N: 'a>(self, f: impl Fn(M) -> N + Clone + 'a) -> Entry<'a, N> {
        match self {
            Entry::Item(item) => Entry::Item(item.map(f)),
            Entry::Heading(heading) => Entry::Heading(heading),
            Entry::Separator => Entry::Separator,
            Entry::Custom(element) => Entry::Custom(element.map(f)),
        }
    }

    /// Its label, if it is an item: what tests compare.
    #[cfg(test)]
    pub fn label(&self) -> Option<&str> {
        match self {
            Entry::Item(item) => Some(&item.label),
            _ => None,
        }
    }

    /// What choosing it sends, if it is an item that can be chosen.
    #[cfg(test)]
    pub fn on_choose(&self) -> Option<&M> {
        match self {
            Entry::Item(item) => item.on_choose.as_ref(),
            _ => None,
        }
    }
}

/// How wide a menu is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Width {
    /// The usual width.
    Menu,
    At(f32),
    /// As wide as where it is: the room list's.
    Fill,
}

/// `entries` in the menus' one look.
pub fn view<'a, M: Clone + 'a>(entries: Vec<Entry<'a, M>>, width: Width) -> Element<'a, M> {
    let entries = entries.into_iter().map(|entry| match entry {
        Entry::Item(item) => item_view(item),
        Entry::Heading(heading) => container(text(heading).size(12).style(text::secondary))
            .padding([4, 10])
            .into(),
        Entry::Separator => container(rule::horizontal(1).style(style::separator))
            .padding([4, 0])
            .into(),
        Entry::Custom(element) => element,
    });
    container(column(entries).spacing(1))
        .width(match width {
            Width::Menu => Length::Fixed(MENU_WIDTH),
            Width::At(width) => Length::Fixed(width),
            Width::Fill => Length::Fill,
        })
        .padding(4)
        .style(style::toolbar)
        .into()
}

fn item_view<'a, M: Clone + 'a>(item: Item<M>) -> Element<'a, M> {
    let mut label = row![].spacing(8).align_y(Alignment::Center);
    if let Some(checked) = item.checked {
        label = label.push(if checked {
            icons::check(TICK, |theme| theme.extended_palette().background.base.text)
        } else {
            space().width(TICK).into()
        });
    }
    label = label.push(ellipsis(item.label).size(13).width(Length::Fill));
    if let Some(detail) = item.detail {
        label = label.push(text(detail).size(12).style(text::secondary));
    }
    let button = button(label)
        .width(Length::Fill)
        .padding([4, 10])
        .style(style::list_item(false))
        .on_press_maybe(item.on_choose);
    match item.hint {
        Some(tip) => hint(button, tip, Position::Left),
        None => button.into(),
    }
}
