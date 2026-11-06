//! Actions on messages (react, reply, edit, delete, its menu), what the
//! composer is writing (a new message or a reply), and editing a message in
//! place.

use iced::{
    Alignment, Color, Element, Length, Padding, Theme,
    advanced::widget::Id,
    keyboard::{Key, key::Named},
    mouse,
    widget::{
        button, column, container, hover, mouse_area, row, scrollable, space, span, text,
        text::Wrapping, text_editor,
    },
};
use mtrx_core::{
    Draft, Event, EventKey, MessageAction, MessageRow, MxcUri, OwnedEventId, OwnedRoomId, Reaction,
    Reply, SendState, User,
};
use mtrx_format::Document;

use crate::{
    app,
    avatar::{Avatars, Shape},
    clipboard::{self, Copied, Field},
    edit_menu,
    ellipsis::ellipsis,
    emoji, fonts,
    highlight::{self, Marks},
    hint::{Position, hint, hint_with},
    icons, info, message,
    message_menu::MenuAt,
    popover::{Edge, Side, popover},
    room,
    selectable::text::selectable,
    selection::{self, Unit},
    style,
};

/// Read markers shown per message before "+N".
const MAX_READERS: usize = 3;
/// Lines of a reply's quote of its original.
const QUOTE_LINES: usize = 3;
const READER_AVATAR: f32 = 16.0;
/// People named in the hint of read markers or a reaction before "and N
/// others".
const HINT_NAMES: usize = 5;
/// The list of everyone who read a message or reacted: the size of their
/// pictures, its width, and its height before it scrolls.
const LISTED_AVATAR: f32 = 24.0;
const READERS_WIDTH: f32 = 220.0;
const READERS_HEIGHT: f32 = 240.0;
/// Height of the text in a reaction pill: one line of 12pt text.
const PILL_TEXT_HEIGHT: f32 = 12.0 * 1.3;
/// Height of the typing line: one line of 12pt text (iced's line height is
/// 1.3 times the size), kept whether or not anyone types.
const TYPING_HEIGHT: f32 = 12.0 * 1.3;

#[derive(Debug, Clone)]
pub enum Message {
    React(EventKey, String),
    /// Pin the message, or unpin it (`false`).
    Pin(OwnedEventId, bool),
    /// Open or close the reaction window of a timeline row, there.
    TogglePicker(String, PickerAt),
    ClosePicker,
    Reply(Target),
    /// Edit one of our sent messages where it is shown, starting from its text.
    Edit(EventKey, String),
    /// ↑ in an empty composer: edit our newest message.
    EditLast,
    EditAction(text_editor::Action),
    SaveEdit,
    CancelEdit,
    EditSaved(Result<(), String>),
    /// All emoji in the reaction picker.
    Emoji(emoji::Message),
    /// The emoji panel of the message being edited.
    EditEmoji(emoji::Message),
    /// Put an emoji into the message being edited.
    EditInsertEmoji(String),
    /// Ask before deleting a timeline row's message.
    AskDelete(String),
    Delete(EventKey),
    CancelDelete,
    /// Open a timeline row's menu, there.
    OpenMenu(String, MenuAt),
    CloseMenu,
    /// List everyone who read up to a timeline row's message.
    OpenReaders(String),
    CloseReaders,
    /// List everyone who reacted to a timeline row's message with a key.
    OpenReactors(String, String),
    CloseReactors,
    /// Offer to retry a timeline row's message that was not sent, or to
    /// take it back.
    OpenUnsent(String),
    CloseUnsent,
    /// Send again a message that was not sent.
    Retry(EventKey),
    /// Take a message out of the send queue, its text back in the composer.
    Unsend(EventKey),
    /// It was taken out: its text, if not a file's, to put back.
    Unsent(Result<Option<Draft>, String>),
    /// Open a link from its menu.
    OpenLink(String),
    /// Put a message's text on the clipboard, as text and HTML.
    Copy(Copied),
    /// Put a link's address on the clipboard.
    CopyLink(String),
    /// Put the text selected in the timeline on the clipboard.
    CopySelection,
    /// Put a message with math on the clipboard as a picture.
    CopyImage(Document),
    /// Save a message with math as a picture.
    SaveImage(Document),
    /// Show a timeline row's text in place of it, to select from.
    /// Stop replying.
    CancelCompose,
    Done(Result<(), String>),
}

/// Where a message's reaction window floats from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerAt {
    /// The smiley of the hover toolbar.
    Toolbar,
    /// The smiley after the message's reactions.
    Reactions,
}

/// A message being replied to or edited.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub key: EventKey,
    pub event_id: OwnedEventId,
    pub sender: String,
    pub preview: String,
}

/// What the composer's text becomes when sent.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Compose {
    #[default]
    New,
    Reply(Target),
}

impl Compose {
    pub fn reply_to(&self) -> Option<&OwnedEventId> {
        match self {
            Self::Reply(target) => Some(&target.event_id),
            _ => None,
        }
    }

    /// Esc leaves the mode.
    pub fn cancellable(&self) -> bool {
        *self != Self::New
    }

    /// The bar above the composer saying what is being written.
    pub fn bar<'a>(&self, room: &OwnedRoomId) -> Option<Element<'a, app::Message>> {
        let (label, preview) = match self {
            Self::New => return None,
            Self::Reply(target) => (
                format!("Replying to {}", target.sender),
                target.preview.clone(),
            ),
        };
        Some(
            container(
                row![
                    text(label).size(12).font(fonts::SEMIBOLD),
                    text(preview)
                        .size(12)
                        .style(text::secondary)
                        .wrapping(text::Wrapping::None)
                        .width(Length::Fill),
                    hint(
                        button(text("×").size(14))
                            .padding([0, 6])
                            .style(style::text_button)
                            .on_press(act(room, Message::CancelCompose)),
                        "Stop replying (Esc)",
                        Position::Left,
                    ),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .padding([4, 10])
            .clip(true)
            .width(Length::Fill)
            .style(style::reply_quote)
            .into(),
        )
    }
}

impl Target {
    pub fn of(event: &Event, sender: String, preview: String) -> Option<Self> {
        Some(Self {
            key: event.key.clone(),
            event_id: event.event_id.clone()?,
            sender,
            preview,
        })
    }
}

/// The field editing a message in place.
const EDITOR: &str = "inline-edit";

/// A message being edited where it is shown: a sent one in the timeline, or
/// a scheduled one in its row. The composer keeps its draft meanwhile.
pub struct InlineEdit {
    pub target: Editing,
    content: text_editor::Content,
    /// The text before editing.
    original: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Editing {
    Sent(EventKey),
    /// By [`mtrx_core::ScheduledMessage::id`].
    Scheduled(String),
}

impl InlineEdit {
    pub fn new(target: Editing, text: &str) -> Self {
        let mut content = text_editor::Content::with_text(text);
        content.perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
        Self {
            target,
            content,
            original: text.to_owned(),
        }
    }

    pub fn insert_emoji(&mut self, emoji: &str) {
        emoji::insert(&mut self.content, emoji);
    }

    /// The emoji the edit adds, one for each use.
    pub fn added_emoji(&self) -> Vec<String> {
        emoji::added(&self.original, &self.text())
    }

    /// Put the cursor into the editor.
    pub fn focus<T>() -> iced::Task<T> {
        iced::widget::operation::focus(EDITOR)
    }

    pub fn perform(&mut self, action: text_editor::Action) {
        self.content.perform(action);
    }

    pub fn text(&self) -> String {
        self.content.text().trim_end().to_owned()
    }

    /// Its editor, for its menu's actions.
    pub fn content_mut(&mut self) -> &mut text_editor::Content {
        &mut self.content
    }

    /// The editor, with emoji, Save and Cancel; Enter saves, Esc cancels.
    /// `popup`: the window's, which may be its menu or emoji panel.
    pub fn view<'a>(
        &'a self,
        room: &OwnedRoomId,
        frequent: &[String],
        popup: Option<&'a app::Popup>,
    ) -> Element<'a, app::Message> {
        let (menu, emoji) = room::in_field(popup, Field::Edit);
        let selected = self.content.selection().is_some();
        let (on_action, on_key) = (room.clone(), room.clone());
        let editor = text_editor(&self.content)
            .id(EDITOR)
            .highlight_with::<highlight::Highlighter>(
                Marks::new(Default::default()),
                highlight::format,
            )
            .style(style::editor)
            .on_action(move |action| act(&on_action, Message::EditAction(action)))
            .key_binding(move |press| {
                let custom = |message| Some(text_editor::Binding::Custom(message));
                // Rich text as Markdown, as in the composer.
                if let Some(how) = clipboard::paste(&press) {
                    return custom(room::Message::Paste(Field::Edit, how).to(&on_key));
                }
                match press.key {
                    Key::Named(Named::Enter) if !press.modifiers.shift() => {
                        custom(act(&on_key, Message::SaveEdit))
                    }
                    _ => selection::editor_key(press, selected),
                }
            })
            .padding(iced::Padding::new(6.0).right(emoji::FIELD_INSET_COMPACT))
            .max_height(240);
        let editor = edit_menu::with_menu(room, Field::Edit, editor, menu, selected);
        let editor = emoji::in_field(
            editor,
            emoji.is_some(),
            true,
            act(room, Message::EditEmoji(emoji::Message::Open)),
        );
        let panel = emoji.map(|panel| {
            panel.view(
                frequent,
                |emoji| act(room, Message::EditInsertEmoji(emoji)),
                {
                    let room = room.clone();
                    move |message| act(&room, Message::EditEmoji(message))
                },
            )
        });
        let editor = popover(editor, panel, Side::Below, Edge::Right)
            .on_dismiss(act(room, Message::EditEmoji(emoji::Message::Close)))
            .click_through();
        let empty = self.content.text().trim().is_empty();
        let view = column![
            editor,
            row![
                text("Enter saves · Esc cancels")
                    .size(11)
                    .style(text::secondary),
                space::horizontal(),
                button(text("Cancel").size(12))
                    .padding([2, 8])
                    .style(style::text_button)
                    .on_press(act(room, Message::CancelEdit)),
                button(text("Save").size(12))
                    .padding([2, 10])
                    .style(style::primary_button)
                    .on_press_maybe((!empty).then(|| act(room, Message::SaveEdit))),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        ]
        .spacing(4);
        view.into()
    }
}

fn tool_color(theme: &Theme) -> iced::Color {
    theme
        .extended_palette()
        .background
        .base
        .text
        .scale_alpha(0.7)
}

/// The toolbar's "⋯" for a message with a menu: the menu while open there.
pub struct Menu<'a>(pub Option<Element<'a, app::Message>>);

/// The actions of a message that its toolbar offers.
const TOOLS: [MessageAction; 4] = [
    MessageAction::React,
    MessageAction::Reply,
    MessageAction::Edit,
    MessageAction::Delete,
];

/// Some of `actions` are in a message's toolbar.
pub fn has_tools(actions: &[MessageAction]) -> bool {
    actions.iter().any(|action| TOOLS.contains(action))
}

/// Symbols floating over a hovered message for its actions (see
/// [`TOOLS`]), each named on hover; the reaction window floats from the
/// smiley while `picker` is given.
pub fn toolbar<'a>(
    room: &OwnedRoomId,
    row: &MessageRow,
    target: Option<Target>,
    edit_text: Option<String>,
    picker: Option<Element<'a, app::Message>>,
    menu: Option<Menu<'a>>,
) -> Element<'a, app::Message> {
    let event = &row.event;
    let offers = |action| row.actions.contains(&action);
    let tool = |icon: Element<'a, app::Message>, tip: &'static str, message: Option<Message>| {
        hint(
            button(icon)
                .padding([style::TOOL_PADDING, 6.0])
                .style(style::text_button)
                .on_press_maybe(message.map(|message| act(room, message))),
            tip,
            Position::Top,
        )
    };
    let mut bar = row![].spacing(2);
    if offers(MessageAction::React) {
        // While open, a click on it closes the window as any outside click.
        let react = tool(
            icons::smiley(style::TOOL_ICON, tool_color),
            "React",
            picker
                .is_none()
                .then(|| Message::TogglePicker(event.id.clone(), PickerAt::Toolbar)),
        );
        bar = bar.push(
            popover(react, picker, Side::Below, Edge::Right)
                .on_dismiss(act(room, Message::ClosePicker)),
        );
    }
    if offers(MessageAction::Reply) {
        bar = bar.push(tool(
            icons::reply(style::TOOL_ICON, tool_color),
            "Reply",
            target.map(Message::Reply),
        ));
    }
    if offers(MessageAction::Edit) {
        bar = bar.push(tool(
            icons::pencil(style::TOOL_ICON, tool_color),
            "Edit",
            edit_text.map(|text| Message::Edit(event.key.clone(), text)),
        ));
    }
    if offers(MessageAction::Delete) {
        bar = bar.push(tool(
            icons::trash(style::TOOL_ICON, tool_color),
            "Delete",
            Some(Message::AskDelete(event.id.clone())),
        ));
    }
    if let Some(Menu(open)) = menu {
        // While open, a click on it closes the menu as any outside click.
        let more = tool(
            text("⋯")
                .size(style::TOOL_ICON)
                .line_height(1.0)
                .style(|theme: &Theme| text::Style {
                    color: Some(tool_color(theme)),
                })
                .into(),
            "More: copy, select text",
            open.is_none()
                .then(|| Message::OpenMenu(event.id.clone(), MenuAt::Toolbar)),
        );
        bar = bar.push(
            popover(more, open, Side::Below, Edge::Right).on_dismiss(act(room, Message::CloseMenu)),
        );
    }
    container(bar)
        .padding(style::TOOLBAR_PADDING)
        .style(style::toolbar)
        .into()
}

/// The reaction window: the most used emoji, and below them all emoji
/// once "+" is clicked.
pub fn picker<'a>(
    room: &OwnedRoomId,
    event: &Event,
    reactions: &[Reaction],
    frequent: &[String],
    all: Option<&'a emoji::Panel>,
) -> Element<'a, app::Message> {
    let key = event.key.clone();
    let quick: Vec<Element<'a, app::Message>> = emoji::quick(frequent)
        .into_iter()
        .map(|emoji| {
            let own = reactions.iter().any(|r| r.key == emoji && r.own);
            button(text(emoji.to_owned()).size(18).font(fonts::EMOJI))
                .padding([2, 6])
                .style(style::list_item(own))
                .on_press(act(room, Message::React(key.clone(), emoji.to_owned())))
                .into()
        })
        .collect();
    let more = hint(
        button(text(if all.is_some() { "−" } else { "+" }).size(18))
            .padding([2, 8])
            .style(style::list_item(all.is_some()))
            .on_press(act(
                room,
                Message::Emoji(if all.is_some() {
                    emoji::Message::Close
                } else {
                    emoji::Message::Open
                }),
            )),
        if all.is_some() {
            "Fewer emoji"
        } else {
            "All emoji"
        },
        Position::Top,
    );
    let mut window =
        column![row(quick).push(more).spacing(2).align_y(Alignment::Center)].spacing(4);
    if let Some(panel) = all {
        window = window.push(panel.view(
            &[],
            move |emoji| act(room, Message::React(key.clone(), emoji)),
            {
                let room = room.clone();
                move |message| act(&room, Message::Emoji(message))
            },
        ));
    }
    container(window).padding(4).style(style::toolbar).into()
}

/// A message's reactions, each a pill: a click on its emoji adds or takes
/// back ours, on its count lists who reacted (`listed`, the key whose list
/// is open). A smiley after them opens the reaction window there
/// (`picker`, while given).
pub fn reactions<'a>(
    room: &OwnedRoomId,
    event: &Event,
    reactions: &[Reaction],
    listed: Option<&str>,
    avatars: &Avatars,
    picker: Option<Element<'a, app::Message>>,
) -> Element<'a, app::Message> {
    let pills = reactions.iter().map(|reaction| {
        pill(
            room,
            event,
            reaction,
            listed == Some(reaction.key.as_str()),
            avatars,
        )
    });
    // While open, a click on it closes the window as any outside click.
    let open = picker.is_none();
    let react = hint(
        button(icons::smiley(14.0, tool_color))
            .padding([2, 6])
            .style(style::reaction(false))
            .on_press_maybe(open.then(|| {
                act(
                    room,
                    Message::TogglePicker(event.id.clone(), PickerAt::Reactions),
                )
            })),
        "React",
        Position::Top,
    );
    let react =
        popover(react, picker, Side::Below, Edge::Left).on_dismiss(act(room, Message::ClosePicker));
    row(pills)
        .push(react)
        .spacing(4)
        .align_y(Alignment::Center)
        .wrap()
        .into()
}

/// One reaction: its emoji toggles ours, its count lists who reacted
/// (`open`, while listed). The halves together are as tall as a pill of
/// one button: the padding moved to inside the border.
fn pill<'a>(
    room: &OwnedRoomId,
    event: &Event,
    reaction: &Reaction,
    open: bool,
    avatars: &Avatars,
) -> Element<'a, app::Message> {
    let toggle = button(text(reaction.key.clone()).size(12).font(fonts::EMOJI))
        .padding(Padding::ZERO.left(6).right(5))
        .style(style::reaction_half(true))
        .on_press(act(
            room,
            Message::React(event.key.clone(), reaction.key.clone()),
        ));
    // As tall as the text: a filling height would make the message row
    // fill the timeline.
    let divider = container(space())
        .width(1)
        .height(PILL_TEXT_HEIGHT)
        .style(style::reaction_divider(reaction.own));
    // While open, a click on it closes the list as any outside click.
    let count = button(text(reaction.count.to_string()).size(12))
        .padding(Padding::ZERO.left(5).right(6))
        .style(style::reaction_half(false))
        .on_press_maybe((!open).then(|| {
            act(
                room,
                Message::OpenReactors(event.id.clone(), reaction.key.clone()),
            )
        }));
    let pill = container(row![toggle, divider, count])
        .padding(1)
        .style(style::reaction_pill(reaction.own));
    if open {
        return popover(
            pill,
            Some(people(&reaction.by, avatars)),
            Side::Below,
            Edge::Left,
        )
        .on_dismiss(act(room, Message::CloseReactors))
        .into();
    }
    let names: Vec<_> = reaction.by.iter().map(User::display_name).collect();
    let names: Vec<_> = names.iter().map(String::as_str).collect();
    let label = row![
        text(reaction.key.clone()).size(12).font(fonts::EMOJI),
        text(named(&names)).size(12),
    ]
    .spacing(4);
    hint_with(pill, label, Position::Top)
}

/// The message a reply answers, in a few lines: its sender's name and the
/// start of it (`units`, see [`crate::rows::Text::Reply`]), selected as
/// `select` says. The start is cut to fit, and reports what it shows under
/// `shown`. A click goes to the message; a drag selects.
pub fn quote<'a>(
    room: &OwnedRoomId,
    reply: &'a Reply,
    units: &[Unit],
    select: message::Select<'a, app::Message>,
    shown: Id,
) -> Element<'a, app::Message> {
    let go = (!reply.failed).then(|| room::Message::GoTo(reply.event_id.clone()).to(room));
    let unit = |index: usize, spans| {
        let unit = selectable(spans, index).size(12);
        let unit = select.apply(unit, index);
        match &go {
            Some(go) => unit.on_click(go.clone()),
            None => unit,
        }
    };
    let sender = unit(0, vec![span(units[0].text.clone()).font(fonts::SEMIBOLD)]);
    let preview = unit(1, vec![span(units[1].text.clone())])
        .style(text::secondary)
        .lines(QUOTE_LINES)
        .wrapping(Wrapping::WordOrGlyph)
        .id(shown);
    let quote = row![
        container(space())
            .width(3)
            .height(Length::Fill)
            .style(style::quote_accent),
        row![sender, preview].spacing(6),
    ]
    .spacing(8)
    .height(Length::Shrink);
    let Some(go) = go else {
        return quote.into();
    };
    // Also beside the text.
    hint(
        mouse_area(quote)
            .interaction(mouse::Interaction::Pointer)
            .on_press(go),
        "Go to the original message",
        Position::Top,
    )
}

/// Asks before deleting.
pub fn confirm_delete<'a>(room: &OwnedRoomId, event: &Event) -> Element<'a, app::Message> {
    row![
        text("Delete this message for everyone?").size(12),
        button(text("Delete").size(12))
            .padding([2, 8])
            .style(style::danger_button)
            .on_press(act(room, Message::Delete(event.key.clone()))),
        button(text("Cancel").size(12))
            .padding([2, 8])
            .style(style::text_button)
            .on_press(act(room, Message::CancelDelete)),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .into()
}

/// Small pictures of who read up to this message; a click lists them all
/// (`open`, while listed).
pub fn read_markers<'a>(
    room: &OwnedRoomId,
    event: &Event,
    avatars: &Avatars,
    open: bool,
) -> Option<Element<'a, app::Message>> {
    if event.read_by.is_empty() {
        return None;
    }
    let mut markers = row![].spacing(2).align_y(Alignment::Center);
    for reader in event.read_by.iter().take(MAX_READERS) {
        markers = markers.push(avatars.view(
            reader.id.as_str(),
            &reader.display_name(),
            reader.avatar.as_deref(),
            READER_AVATAR,
            Shape::Circle,
        ));
    }
    if event.read_by.len() > MAX_READERS {
        markers = markers.push(
            text(format!("+{}", event.read_by.len() - MAX_READERS))
                .size(10)
                .style(text::secondary),
        );
    }
    // While open, a click on them closes the list as any outside click.
    let markers = button(markers)
        .padding(0)
        .style(style::text_button)
        .on_press_maybe((!open).then(|| act(room, Message::OpenReaders(event.id.clone()))));
    if open {
        let list = people(&event.read_by, avatars);
        return Some(
            popover(markers, Some(list), Side::Above, Edge::Right)
                .on_dismiss(act(room, Message::CloseReaders))
                .into(),
        );
    }
    let names: Vec<_> = event.read_by.iter().map(User::display_name).collect();
    let names: Vec<_> = names.iter().map(String::as_str).collect();
    // The names show over the pictures.
    Some(hint(markers, read_by(&names), Position::Top))
}

/// How a message's send status shows, besides its state.
#[derive(Clone, Copy, Debug, Default)]
pub struct Status {
    /// Our newest message: the only one with the tick.
    pub newest: bool,
    /// Being sent long enough to show the clock.
    pub slow: bool,
    /// Its menu to retry or take it back is open.
    pub open: bool,
}

/// Whether our message is sent, while nobody read it (once someone did, it
/// was): a clock while it is being sent (only once that takes a while) or
/// waits for the connection, a tick once it is sent (only on our newest
/// message: the older ones were sent too), a red cross if it could not be.
/// The same size as a read marker. Until sent, a click on the cross (over
/// the clock while hovered) offers to take it back, and to retry it if it
/// could not be sent.
pub fn send_status<'a>(
    room: &OwnedRoomId,
    event: &Event,
    status: Status,
) -> Option<Element<'a, app::Message>> {
    if !event.own || !event.read_by.is_empty() {
        return None;
    }
    let grey: fn(&Theme) -> Color = |theme| theme.extended_palette().secondary.base.color;
    let red: fn(&Theme) -> Color = |theme| theme.palette().danger;
    let (icon, label) = match &event.send_state {
        SendState::Sending if status.slow => {
            (icons::clock(READER_AVATAR, grey), "Sending…".to_owned())
        }
        SendState::Sending => return None,
        SendState::Waiting => (
            icons::clock(READER_AVATAR, grey),
            "Waiting for connection…".to_owned(),
        ),
        SendState::Sent if status.newest => {
            (icons::check_circle(READER_AVATAR, grey), "Sent".to_owned())
        }
        SendState::Sent => return None,
        SendState::Failed(e) => (
            icons::x_circle(READER_AVATAR, red),
            format!("Not sent: {e}"),
        ),
    };
    if event.send_state == SendState::Sent {
        return Some(hint(icon, label, Position::Top));
    }
    let failed = matches!(event.send_state, SendState::Failed(_));
    // While open, a click on it closes the menu as any outside click.
    let cross = button(icons::x_circle(
        READER_AVATAR,
        if failed { red } else { grey },
    ))
    .padding(0)
    .style(style::text_button)
    .on_press_maybe((!status.open).then(|| act(room, Message::OpenUnsent(event.id.clone()))));
    if status.open {
        return Some(
            popover(
                cross,
                Some(unsent_menu(room, event, failed)),
                Side::Above,
                Edge::Right,
            )
            .on_dismiss(act(room, Message::CloseUnsent))
            .into(),
        );
    }
    // The clock, and the cross over it while hovered.
    let icon = if failed {
        cross.into()
    } else {
        hover(icon, cross)
    };
    Some(hint(icon, label, Position::Top))
}

/// Retry a message that could not be sent (`failed`), or take back one not
/// sent: its text goes back to the composer.
fn unsent_menu<'a>(room: &OwnedRoomId, event: &Event, failed: bool) -> Element<'a, app::Message> {
    let retry = button(text("Retry").size(12))
        .padding([2, 8])
        .style(style::text_button)
        .on_press(act(room, Message::Retry(event.key.clone())));
    let unsend = button(text("Cancel and put back").size(12))
        .padding([2, 8])
        .style(style::text_button)
        .on_press(act(room, Message::Unsend(event.key.clone())));
    let mut choices = row![].spacing(4).align_y(Alignment::Center);
    if failed {
        choices = choices.push(retry);
    }
    container(choices.push(unsend))
        .padding(4)
        .style(style::toolbar)
        .into()
}

/// People who read up to a message or reacted to it; a click on one opens
/// their card.
fn people<'a>(people: &[User], avatars: &Avatars) -> Element<'a, app::Message> {
    let rows = people.iter().map(|reader| {
        let name = reader.display_name();
        let mut label = column![ellipsis(name.clone()).size(13)];
        if reader.name.is_some() {
            label = label.push(
                ellipsis(reader.id.to_string())
                    .size(11)
                    .style(text::secondary),
            );
        }
        let picture = avatars.view(
            reader.id.as_str(),
            &name,
            reader.avatar.as_deref(),
            LISTED_AVATAR,
            Shape::Circle,
        );
        button(row![picture, label].spacing(8).align_y(Alignment::Center))
            .width(Length::Fill)
            .padding([4, 6])
            .style(style::list_item(false))
            .on_press(app::Message::ShowInfo(info::Target::User(reader.clone())))
            .into()
    });
    container(scrollable(column(rows).spacing(2)))
        .width(READERS_WIDTH)
        .max_height(READERS_HEIGHT)
        .padding(4)
        .style(style::toolbar)
        .into()
}

/// "Read by Alice, Bob and Carol", naming at most `HINT_NAMES` readers:
/// "Read by Alice, …, Eve and 7 others".
fn read_by(names: &[&str]) -> String {
    format!("Read by {}", named(names))
}

/// "Alice, Bob and Carol", naming at most `HINT_NAMES`: "Alice, …, Eve
/// and 7 others".
fn named(names: &[&str]) -> String {
    let shown = names.len().min(HINT_NAMES);
    let mut parts: Vec<String> = names[..shown].iter().map(|name| name.to_string()).collect();
    match names.len() - shown {
        0 => {}
        1 => parts.push("1 other".to_owned()),
        others => parts.push(format!("{others} others")),
    }
    match parts.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, front)) => format!("{} and {last}", front.join(", ")),
        None => String::new(),
    }
}

/// Pictures of readers shown, with their sizes: in the markers, and of
/// everyone while `listed`.
pub fn reader_avatars(event: &Event, listed: bool) -> impl Iterator<Item = (&MxcUri, f32)> {
    let markers = event
        .read_by
        .iter()
        .take(MAX_READERS)
        .filter_map(|r| Some((r.avatar.as_deref()?, READER_AVATAR)));
    let list = listed_avatars(&event.read_by).filter(move |_| listed);
    markers.chain(list)
}

/// Pictures of everyone who reacted with `key` to the message, while
/// listed.
pub fn reactor_avatars<'a>(
    event: &'a Event,
    key: Option<&'a str>,
) -> impl Iterator<Item = (&'a MxcUri, f32)> {
    let by = match &event.content {
        mtrx_core::EventContent::Message(m) => key
            .and_then(|key| m.reactions.iter().find(|r| r.key == key))
            .map_or(&[][..], |r| r.by.as_slice()),
        _ => &[],
    };
    listed_avatars(by)
}

fn listed_avatars(people: &[User]) -> impl Iterator<Item = (&MxcUri, f32)> {
    people
        .iter()
        .filter_map(|r| Some((r.avatar.as_deref()?, LISTED_AVATAR)))
}

/// Others typing, above the composer: a line always there, empty when
/// nobody types, so the composer and the timeline above stay put.
pub fn typing<'a>(names: &[String]) -> Element<'a, app::Message> {
    let line = match names {
        [] => None,
        [one] => Some(format!("{one} is typing…")),
        [one, two] => Some(format!("{one} and {two} are typing…")),
        [one, rest @ ..] => Some(format!("{one} and {} others are typing…", rest.len())),
    };
    let line: Element<'a, app::Message> = match line {
        Some(line) => text(line).size(12).style(text::secondary).into(),
        None => space().into(),
    };
    container(line).height(TYPING_HEIGHT).into()
}

/// `message` for `room`, as the app gets it.
pub fn act(room: &OwnedRoomId, message: Message) -> app::Message {
    room::Message::Act(message).to(room)
}

#[cfg(test)]
mod tests {
    use super::{named, read_by};

    const NAMES: [&str; 10] = [
        "Alice", "Bob", "Carol", "Dan", "Eve", "Fay", "Gus", "Hal", "Ivy", "Jo",
    ];

    #[test]
    fn a_quote_goes_to_its_message_on_a_click_and_selects_on_a_drag() {
        use std::rc::Rc;

        use iced::{Event, Point, Size, mouse};
        use iced_test::simulator::Simulator;
        use mtrx_core::Reply;

        use crate::{
            app::Message,
            fonts, message, room,
            selection::{self, Gesture, Unit},
        };

        let reply = Reply {
            event_id: "$original:x".try_into().unwrap(),
            sender: Some(mtrx_core::User::from(
                mtrx_core::OwnedUserId::try_from("@carol:x").unwrap(),
            )),
            preview: Some("Where is the data from?".into()),
            kind: None,
            failed: false,
        };
        let unit = |text: &str| Unit {
            text: text.into(),
            prefix: String::new(),
            gap: "",
        };
        let units = [unit("carol"), unit("Where is the data from?")];
        let room_id: mtrx_core::OwnedRoomId = "!room:x".try_into().unwrap();
        // Pressed on the start of the quote, released at `to`.
        let messages = |to: f32| -> Vec<Message> {
            let select = message::Select {
                range: None,
                focus: None,
                dragging: true,
                on_gesture: Rc::new(|gesture| {
                    room::Message::Select(selection::Message::Gesture("$r/reply".into(), gesture))
                        .to(&"!room:x".try_into().unwrap())
                }),
                found: Vec::new(),
            };
            let quote = super::quote(
                &room_id,
                &reply,
                &units,
                select,
                crate::rows::shown_id("$r/reply"),
            );
            let mut ui = Simulator::with_size(fonts::settings(), Size::new(400.0, 100.0), quote);
            ui.point_at((60.0, 8.0));
            let _ = ui.simulate([Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            ))]);
            ui.point_at((to, 8.0));
            let _ = ui.simulate([
                Event::Mouse(mouse::Event::CursorMoved {
                    position: Point::new(to, 8.0),
                }),
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ]);
            ui.into_messages().collect()
        };
        let went = |messages: &[Message]| {
            (messages.iter()).any(
                |m| matches!(m, Message::Room(_, room::Message::GoTo(id)) if id == "$original:x"),
            )
        };
        let dragged = |messages: &[Message]| {
            (messages.iter()).any(|m| {
                matches!(
                    m,
                    Message::Room(
                        _,
                        room::Message::Select(selection::Message::Gesture(
                            _,
                            Gesture::Drag { unit: 1, .. }
                        ))
                    )
                )
            })
        };
        let clicked = messages(61.0);
        assert!(went(&clicked), "{clicked:?}");
        let selecting = messages(150.0);
        assert!(!went(&selecting) && dragged(&selecting), "{selecting:?}");
    }

    #[test]
    fn read_by_names_up_to_five() {
        assert_eq!(read_by(&NAMES[..1]), "Read by Alice");
        assert_eq!(read_by(&NAMES[..2]), "Read by Alice and Bob");
        assert_eq!(
            read_by(&NAMES[..5]),
            "Read by Alice, Bob, Carol, Dan and Eve"
        );
    }

    #[test]
    fn read_by_counts_the_rest() {
        assert_eq!(
            read_by(&NAMES[..6]),
            "Read by Alice, Bob, Carol, Dan, Eve and 1 other"
        );
        assert_eq!(
            read_by(&NAMES),
            "Read by Alice, Bob, Carol, Dan, Eve and 5 others"
        );
    }

    #[test]
    fn named_without_prefix_for_reactions() {
        assert_eq!(named(&NAMES[..3]), "Alice, Bob and Carol");
        assert_eq!(named(&NAMES), "Alice, Bob, Carol, Dan, Eve and 5 others");
    }
}
