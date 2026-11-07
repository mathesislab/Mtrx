//! A field one writes a message in: the composer, or a message edited in
//! place. Markdown highlighted, mentions completed, its own menu and emoji
//! panel, and pictures, files and rich text pasted.

use std::{path::PathBuf, sync::Arc};

use iced::{
    Element, Length, Padding, Task,
    keyboard::{Key, key::Named},
    widget::{container, row, stack, text_editor},
};
use mtrx_core::Attachment;

use crate::{
    app::{self, Popup},
    avatar::Avatars,
    clipboard::{self, Pasted},
    edit_menu, emoji,
    highlight::{self, Marks},
    mention::{self, Candidate, Completion, Mentionable},
    popover::{Edge, Side, popover},
    selection, sidebar, spelling, style,
};

/// The composer's editor, focused e.g. when typing with nothing focused.
pub const COMPOSER: &str = "composer";
/// The editor of the message edited in place.
const EDIT: &str = "inline-edit";

/// Which of a room's message fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// The composer, which also attaches files and pictures.
    Composer,
    /// A message being edited in place: text only, as an edit cannot add
    /// files.
    Edit,
}

impl Field {
    /// Its editor's widget ID.
    fn id(self) -> &'static str {
        match self {
            Field::Composer => COMPOSER,
            Field::Edit => EDIT,
        }
    }
}

/// Its text, and the mention being completed in it.
pub struct MessageField {
    field: Field,
    content: text_editor::Content,
    completion: Option<Completion>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Action(text_editor::Action),
    /// Its menu (right-click).
    Menu(edit_menu::Message),
    /// ⌘V (or ⌥⇧⌘V): read the clipboard.
    Paste(clipboard::Paste),
    /// What the clipboard held.
    Pasted(Option<Pasted>),
    /// Its emoji panel.
    Emoji(emoji::Message),
    /// Put an emoji where the cursor is.
    InsertEmoji(String),
    /// The mention completion: its keys, a click on a candidate, closed.
    Mention(mention::Message),
    /// Enter.
    Submit,
    /// Esc, see [`Look::cancellable`].
    Cancel,
    /// ↑ in the empty field, see [`Look::edit_last`].
    EditLast,
}

/// What the field asks of its owner.
#[derive(Debug)]
pub enum Event {
    /// Its text changed.
    Edited,
    /// Enter: send or save.
    Submit,
    Cancel,
    /// Edit our newest message.
    EditLast,
    /// Complete the mention: the owner knows who may be mentioned, see
    /// [`MessageField::mention`].
    Mention(mention::Message),
    /// Files pasted: attach them, as if dropped.
    Attach(Vec<PathBuf>),
    /// A picture pasted: attach it.
    AttachImage(Attachment),
    /// Its emoji panel opened: the most used may have changed elsewhere.
    EmojiShown,
}

/// How the field looks and which keys it takes, as its owner wants.
pub struct Look<'a> {
    pub placeholder: Option<&'a str>,
    /// Esc sends [`Event::Cancel`] (once no mention is being completed).
    pub cancellable: bool,
    /// ↑ in the empty field sends [`Event::EditLast`].
    pub edit_last: bool,
    /// Where its emoji panel and the completion open.
    pub side: Side,
    /// A smaller smiley, closer to the edge.
    pub compact: bool,
    /// Around the text, clear of the buttons inside the field.
    pub padding: Padding,
    pub max_height: f32,
    /// Buttons inside the field, before its smiley: the composer's
    /// paperclip.
    pub tools: Option<Element<'a, app::Message>>,
}

/// What a field shows of its room and the window.
#[derive(Clone, Copy)]
pub struct Context<'a, 'b> {
    /// Who may be mentioned: highlighted, and offered to complete.
    pub mentions: Mentionable<'b>,
    pub avatars: &'b Avatars,
    /// The most used emoji, first in the panel.
    pub frequent: &'b [String],
    /// The window's, which may be this field's menu or emoji panel.
    pub popup: Option<&'a Popup>,
}

impl MessageField {
    /// `field` holding `text`, the cursor at its end.
    pub fn new(field: Field, text: &str) -> Self {
        let mut this = Self {
            field,
            content: text_editor::Content::new(),
            completion: None,
        };
        this.set_text(text);
        this
    }

    pub fn text(&self) -> String {
        self.content.text()
    }

    /// Replace its text, the cursor at its end.
    pub fn set_text(&mut self, text: &str) {
        self.content = text_editor::Content::with_text(text);
        self.perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
    }

    /// Nothing but white space.
    pub fn is_empty(&self) -> bool {
        self.content.text().trim().is_empty()
    }

    /// Put the cursor into it.
    pub fn focus<T>(&self) -> Task<T> {
        iced::widget::operation::focus(self.field.id())
    }

    /// The mention being completed, if any.
    pub fn completion(&self) -> Option<&Completion> {
        self.completion.as_ref()
    }

    /// Who the mention being completed may be, among `mentions`.
    pub fn candidates<'m>(&self, mentions: &Mentionable<'m>) -> Vec<Candidate<'m>> {
        self.completion
            .as_ref()
            .map_or_else(Vec::new, |c| mentions.candidates(&c.query.text))
    }

    /// Stop completing the mention.
    pub fn close_completion(&mut self) {
        self.completion = None;
    }

    /// Complete the mention being written with `mention`.
    pub fn complete(&mut self, mention: &str) {
        if let Some(completion) = self.completion.take() {
            mention::insert(&mut self.content, &completion.query, mention);
        }
        self.update_completion();
    }

    /// `message` to the completion, offering `candidates`.
    pub fn mention<T>(&mut self, message: mention::Message, candidates: &[Candidate]) -> Task<T> {
        let mention = match message {
            mention::Message::Close => {
                self.close_completion();
                return Task::none();
            }
            mention::Message::Up | mention::Message::Down => {
                let step = if matches!(message, mention::Message::Up) {
                    -1
                } else {
                    1
                };
                if let Some(completion) = &mut self.completion {
                    completion.step(step, candidates.len());
                }
                return Task::none();
            }
            mention::Message::Choose => {
                let selected = self.completion.as_ref().map_or(0, |c| c.selected);
                match candidates.get(selected) {
                    Some(candidate) => candidate.mention(),
                    None => return Task::none(),
                }
            }
            mention::Message::Pick(mention) => mention,
        };
        self.complete(&mention);
        self.focus()
    }

    fn perform(&mut self, action: text_editor::Action) {
        self.content.perform(action);
        self.update_completion();
    }

    /// Its text or cursor changed: complete the mention there.
    fn update_completion(&mut self) {
        self.completion = Completion::of(&self.content, self.completion.as_ref());
    }

    /// Typed over the selection, or where the cursor is.
    fn paste(&mut self, text: String) {
        use text_editor::{Action, Edit};
        self.perform(Action::Edit(Edit::Paste(Arc::new(text))));
    }

    /// Read the clipboard to paste `how`: files and pictures only into the
    /// composer.
    fn read(&self, how: clipboard::Paste) -> Task<Message> {
        Task::perform(
            clipboard::read(how, self.field == Field::Composer),
            Message::Pasted,
        )
    }

    /// `popup` if it is this field's menu, and if it is its emoji panel.
    fn popups<'p>(
        &self,
        popup: Option<&'p Popup>,
    ) -> (Option<&'p edit_menu::Menu>, Option<&'p emoji::Panel>) {
        match popup {
            Some(Popup::FieldMenu(f, menu)) if *f == self.field => (Some(menu), None),
            Some(Popup::FieldEmoji(f, panel)) if *f == self.field => (None, Some(panel)),
            _ => (None, None),
        }
    }

    /// Its menu and emoji panel open in `popup`, the window's.
    pub fn update(
        &mut self,
        message: Message,
        popup: &mut Option<Popup>,
    ) -> (Task<Message>, Option<Event>) {
        match message {
            Message::Action(action) => {
                let edited = action.is_edit();
                self.perform(action);
                (Task::none(), edited.then_some(Event::Edited))
            }
            Message::Menu(message) => self.menu(message, popup),
            Message::Paste(how) => (self.read(how), None),
            Message::Pasted(pasted) => match pasted {
                Some(Pasted::Text(text)) => {
                    self.paste(text);
                    (Task::none(), Some(Event::Edited))
                }
                Some(Pasted::Files(paths)) => (Task::none(), Some(Event::Attach(paths))),
                Some(Pasted::Image(file)) => (Task::none(), Some(Event::AttachImage(file))),
                None => (Task::none(), None),
            },
            Message::Emoji(message) => self.emoji(message, popup),
            Message::InsertEmoji(emoji) => {
                emoji::insert(&mut self.content, &emoji);
                self.update_completion();
                // Counted as used once sent or saved.
                (self.focus(), Some(Event::Edited))
            }
            Message::Mention(message) => (Task::none(), Some(Event::Mention(message))),
            Message::Submit => (Task::none(), Some(Event::Submit)),
            Message::Cancel => (Task::none(), Some(Event::Cancel)),
            Message::EditLast => (Task::none(), Some(Event::EditLast)),
        }
    }

    fn emoji(
        &mut self,
        message: emoji::Message,
        popup: &mut Option<Popup>,
    ) -> (Task<Message>, Option<Event>) {
        let field = self.field;
        match message {
            emoji::Message::Open => {
                *popup = Some(Popup::FieldEmoji(field, emoji::Panel::default()));
                (
                    iced::widget::operation::focus(emoji::SEARCH),
                    Some(Event::EmojiShown),
                )
            }
            message => {
                if let Some(Popup::FieldEmoji(f, panel)) = popup
                    && *f == field
                {
                    panel.update(message);
                }
                (Task::none(), None)
            }
        }
    }

    fn menu(
        &mut self,
        message: edit_menu::Message,
        popup: &mut Option<Popup>,
    ) -> (Task<Message>, Option<Event>) {
        use edit_menu::Message as Menu;
        use text_editor::{Action, Edit};

        let field = self.field;
        // Any choice closes the menu.
        let misspelled = match popup.take_if(|p| matches!(p, Popup::FieldMenu(f, _) if *f == field))
        {
            Some(Popup::FieldMenu(_, menu)) => menu.misspelled,
            _ => None,
        };
        let selection = self.content.selection();
        // The field keeps the focus, as a text field does.
        let (done, edited) = match message {
            Menu::Open => {
                *popup = Some(Popup::FieldMenu(field, edit_menu::Menu::at(&self.content)));
                return (Task::none(), None);
            }
            // The spelling replaces the word as typed over it.
            Menu::Correct(i) => match misspelled.and_then(|m| {
                m.select(&mut self.content);
                m.suggestions.get(i).cloned()
            }) {
                Some(spelling) => {
                    self.paste(spelling);
                    (Task::none(), true)
                }
                None => (Task::none(), false),
            },
            Menu::LearnSpelling => {
                if let Some(misspelled) = misspelled {
                    spelling::learn(&misspelled.word);
                }
                (Task::none(), false)
            }
            Menu::IgnoreSpelling => {
                if let Some(misspelled) = misspelled {
                    spelling::ignore(&misspelled.word);
                }
                (Task::none(), false)
            }
            Menu::Cut => match selection {
                Some(selection) => {
                    self.perform(Action::Edit(Edit::Delete));
                    (iced::clipboard::write(selection), true)
                }
                None => (Task::none(), false),
            },
            Menu::Copy => (
                selection.map_or_else(Task::none, iced::clipboard::write),
                false,
            ),
            Menu::Paste => (self.read(clipboard::Paste::Rich), false),
            Menu::SelectAll => {
                self.perform(Action::SelectAll);
                (Task::none(), false)
            }
            #[cfg(target_os = "macos")]
            Menu::EmojiAndSymbols => {
                edit_menu::emoji_and_symbols();
                (Task::none(), false)
            }
        };
        (
            Task::batch([self.focus(), done]),
            edited.then_some(Event::Edited),
        )
    }

    /// The editor, with its menu, smiley and emoji panel, and the mention
    /// completion; `to` makes the app's message of one of its own. Enter
    /// submits; ⌘↑, ⌘↓ and the other keys going to another room are left
    /// to the window.
    pub fn view<'a>(
        &'a self,
        look: Look<'a>,
        cx: Context<'a, '_>,
        to: impl Fn(Message) -> app::Message + Clone + 'a,
    ) -> Element<'a, app::Message> {
        let candidates = self.candidates(&cx.mentions);
        let completing = !candidates.is_empty();
        let (menu, emoji) = self.popups(cx.popup);
        let selected = self.content.selection().is_some();
        let empty = self.content.text().is_empty();
        let (cancellable, edit_last) = (look.cancellable, look.edit_last);
        let mut editor = text_editor(&self.content)
            .id(self.field.id())
            .highlight_with::<highlight::Highlighter>(
                Marks::new(cx.mentions.texts.clone()),
                highlight::format,
            )
            .style(style::editor)
            .on_action({
                let to = to.clone();
                move |action| to(Message::Action(action))
            })
            .key_binding({
                let to = to.clone();
                move |press| {
                    let custom = |message| Some(text_editor::Binding::Custom(to(message)));
                    // The window's: going to another room.
                    if sidebar::Go::from_key(&press.key, press.physical_key, press.modifiers)
                        .is_some()
                    {
                        return None;
                    }
                    // Pictures, files and rich text too, which iced's
                    // clipboard lacks.
                    if let Some(how) = clipboard::paste(&press) {
                        return custom(Message::Paste(how));
                    }
                    match press.key {
                        Key::Named(Named::ArrowUp) if completing => {
                            custom(Message::Mention(mention::Message::Up))
                        }
                        Key::Named(Named::ArrowDown) if completing => {
                            custom(Message::Mention(mention::Message::Down))
                        }
                        Key::Named(Named::Enter | Named::Tab) if completing => {
                            custom(Message::Mention(mention::Message::Choose))
                        }
                        // The window's Esc closes the completion.
                        Key::Named(Named::Escape) if completing => None,
                        Key::Named(Named::Enter) if !press.modifiers.shift() => {
                            custom(Message::Submit)
                        }
                        Key::Named(Named::Escape) if cancellable => custom(Message::Cancel),
                        // Bindings are asked also while not focused.
                        Key::Named(Named::ArrowUp)
                            if edit_last
                                && empty
                                && press.modifiers.is_empty()
                                && matches!(press.status, text_editor::Status::Focused { .. }) =>
                        {
                            custom(Message::EditLast)
                        }
                        _ => selection::editor_key(press, selected),
                    }
                }
            })
            .padding(look.padding)
            .max_height(look.max_height);
        if let Some(placeholder) = look.placeholder {
            editor = editor.placeholder(placeholder);
        }
        let editor = edit_menu::with_menu(
            editor,
            menu,
            selected,
            {
                let to = to.clone();
                move |message| to(Message::Menu(message))
            },
            app::Message::ClosePopup,
        );
        let toggle = emoji::toggle(
            emoji.is_some(),
            look.compact,
            to(Message::Emoji(emoji::Message::Open)),
        );
        let edge = if look.compact { 4.0 } else { 6.0 };
        let field = stack![
            editor,
            container(row![look.tools, toggle].spacing(2))
                .align_right(Length::Fill)
                .align_bottom(Length::Fill)
                .padding(Padding::new(edge).top(0)),
        ];
        let panel = emoji.map(|panel| {
            panel.view(
                cx.frequent,
                {
                    let to = to.clone();
                    move |emoji| to(Message::InsertEmoji(emoji))
                },
                {
                    let to = to.clone();
                    move |message| to(Message::Emoji(message))
                },
            )
        });
        let field = popover(field, panel, look.side, Edge::Right)
            .on_dismiss(app::Message::ClosePopup)
            .click_through();
        let highlighted = self.completion.as_ref().map_or(0, |c| c.selected);
        let completion = completing.then(|| {
            mention::view(&candidates, highlighted, cx.avatars, {
                let to = to.clone();
                move |mention| to(Message::Mention(mention::Message::Pick(mention)))
            })
        });
        popover(field, completion, look.side, Edge::Left)
            .on_dismiss(to(Message::Mention(mention::Message::Close)))
            .click_through()
            .into()
    }
}

#[cfg(test)]
mod tests {
    use mtrx_core::{Member, OwnedUserId, Role};

    use super::*;

    fn member(id: &str, name: &str) -> Member {
        Member {
            user: mtrx_core::User {
                id: OwnedUserId::try_from(id).unwrap(),
                name: Some(name.into()),
                avatar: None,
            },
            role: Role::Member,
            level: Some(0),
            membership: mtrx_core::Membership::Joined,
            own: false,
            manage: Default::default(),
        }
    }

    /// Both fields complete a mention: Enter takes the one offered.
    #[test]
    fn both_fields_complete_mentions() {
        let members = [member("@me:x.org", "Me"), member("@ana:x.org", "Ana")];
        let texts = mention::texts(&members);
        let mentions = Mentionable {
            members: Some(&members),
            texts: &texts,
            own: &members[0].user.id,
            room: false,
        };
        for field in [Field::Composer, Field::Edit] {
            let mut written = MessageField::new(field, "Thanks @a");
            let candidates = written.candidates(&mentions);
            let offered: Vec<_> = candidates.iter().map(Candidate::mention).collect();
            assert_eq!(offered, ["@Ana"], "{field:?}");
            let _: Task<()> = written.mention(mention::Message::Choose, &candidates);
            assert_eq!(written.text(), "Thanks @Ana ", "{field:?}");
            assert!(written.completion().is_none(), "{field:?}");
        }
    }

    /// Nothing is offered before the members are loaded.
    #[test]
    fn no_completion_without_members() {
        let texts: Arc<[String]> = Default::default();
        let own = OwnedUserId::try_from("@me:x.org").unwrap();
        let mentions = Mentionable {
            members: None,
            texts: &texts,
            own: &own,
            room: true,
        };
        let written = MessageField::new(Field::Edit, "Hello @ro");
        assert!(written.completion().is_some());
        assert!(written.candidates(&mentions).is_empty());
    }

    #[test]
    fn pasted_text_is_typed_and_files_are_attached() {
        let mut popup = None;
        let mut field = MessageField::new(Field::Composer, "See ");
        let pasted = Message::Pasted(Some(Pasted::Text("the map".into())));
        let (_, event) = field.update(pasted, &mut popup);
        assert!(matches!(event, Some(Event::Edited)), "{event:?}");
        assert_eq!(field.text(), "See the map");
        let pasted = Message::Pasted(Some(Pasted::Files(vec!["map.png".into()])));
        let (_, event) = field.update(pasted, &mut popup);
        assert!(
            matches!(&event, Some(Event::Attach(paths)) if *paths == [PathBuf::from("map.png")]),
            "{event:?}"
        );
    }

    /// A field's emoji panel is the window's popup: another field's
    /// searches leave it alone, and its panel replaces it.
    #[test]
    fn each_field_has_its_own_panel() {
        let mut popup = None;
        let mut composer = MessageField::new(Field::Composer, "");
        let mut edit = MessageField::new(Field::Edit, "Melt rates");
        let (_, event) = edit.update(Message::Emoji(emoji::Message::Open), &mut popup);
        assert!(matches!(event, Some(Event::EmojiShown)), "{event:?}");
        let search = Message::Emoji(emoji::Message::Search("ice".into()));
        let _ = composer.update(search, &mut popup);
        assert!(matches!(
            &popup,
            Some(Popup::FieldEmoji(Field::Edit, panel)) if panel.query.is_empty()
        ));
        let _ = composer.update(Message::Emoji(emoji::Message::Open), &mut popup);
        assert!(matches!(popup, Some(Popup::FieldEmoji(Field::Composer, _))));
    }

    #[test]
    fn an_emoji_goes_where_the_cursor_is() {
        let mut popup = None;
        let mut field = MessageField::new(Field::Edit, "Done ");
        let (_, event) = field.update(Message::InsertEmoji("🎉".into()), &mut popup);
        assert!(matches!(event, Some(Event::Edited)), "{event:?}");
        assert_eq!(field.text(), "Done 🎉");
    }
}
