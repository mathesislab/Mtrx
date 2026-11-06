//! A room's topic and its links (see [`Topic`]): the topic panel below the
//! room header,
//! showing them in full, and the editor the panel and the room card share:
//! the text and a list of links, or the whole topic as text, as other apps
//! show it.

use std::time::Duration;

use iced::{
    Alignment, Color, Element, Font, Length, Task, Theme,
    widget::{button, column, container, row, space, text, text_editor, text_input, toggler},
};
use mtrx_core::{CallService, LinkKind, RoomLink, Topic, Url};

use crate::{
    app,
    ellipsis::ellipsis,
    fonts,
    highlight::{self, Marks},
    hint::{Position, hint},
    icons,
    reorder::{self, Reorder},
    selectable::text::selectable_text,
    selection, spelling, style,
};

/// How long Cancel and Save stand out after a click outside the editor.
const NUDGE: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone)]
pub enum Message {
    /// Open or close the panel: the caret below the room's name.
    Toggle,
    /// A click outside the open panel: closes it, unless changes are being
    /// edited, which Cancel and Save then point out.
    Dismiss,
    /// The nudge of this number is over.
    Settle(u32),
    /// Edit the topic in the panel, opening it.
    Edit,
    Editor(Edit),
    Save,
    Cancel,
    Saved(Result<(), String>),
}

/// The topic panel, dropping below the room header: open or not, and the
/// topic's editor. A topic saved shows in the room list at once.
#[derive(Default)]
pub struct Panel {
    pub open: bool,
    pub editor: Option<Editor>,
    pub saving: bool,
    pub error: Option<String>,
    /// Cancel and Save stand out, after the click outside numbered so.
    nudge: (u32, bool),
}

impl Panel {
    /// The topic is being saved.
    pub fn save(&mut self) {
        self.saving = true;
        self.error = None;
    }

    pub fn update(&mut self, message: Message, topic: &str) -> Task<Message> {
        match message {
            Message::Toggle => {
                self.open = !self.open;
                self.editor = None;
                self.error = None;
            }
            Message::Dismiss => {
                if self.editor.as_ref().is_some_and(|e| e.changed(Some(topic))) {
                    let nudge = self.nudge.0 + 1;
                    self.nudge = (nudge, true);
                    let settle = async { tokio::time::sleep(NUDGE).await };
                    return Task::perform(settle, move |()| Message::Settle(nudge));
                }
                self.open = false;
                self.editor = None;
                self.error = None;
            }
            Message::Settle(nudge) => {
                if nudge == self.nudge.0 {
                    self.nudge.1 = false;
                }
            }
            Message::Edit => {
                self.open = true;
                self.editor = Some(Editor::new(topic));
                self.error = None;
            }
            Message::Editor(edit) => {
                if let Some(editor) = &mut self.editor {
                    editor.update(edit);
                }
            }
            Message::Cancel => {
                self.editor = None;
                self.error = None;
            }
            Message::Saved(Ok(())) => {
                self.saving = false;
                self.editor = None;
            }
            Message::Saved(Err(e)) => {
                self.saving = false;
                self.error = Some(e);
            }
            // The app saves.
            Message::Save => {}
        }
        Task::none()
    }

    /// The topic in full and its links, or the editor, below the room
    /// header; to edit, if `can_edit`.
    pub fn view<'a>(
        &'a self,
        topic: &Topic,
        services: &'a [CallService],
        can_edit: bool,
        message: impl Fn(Message) -> app::Message + Clone + 'a,
    ) -> Element<'a, app::Message> {
        let mut panel = match &self.editor {
            Some(editor) => {
                let edit = message.clone();
                let nudged = self.nudge.1;
                let actions = row![
                    space::horizontal(),
                    button(text("Cancel").size(13))
                        .padding([4, 10])
                        .style(style::ringed(style::text_button, nudged))
                        .on_press(message(Message::Cancel)),
                    button(text("Save").size(13))
                        .padding([4, 12])
                        .style(style::ringed(style::primary_button, nudged))
                        .on_press_maybe(
                            (editor.valid() && !self.saving).then(|| message(Message::Save))
                        ),
                ]
                .spacing(6)
                .align_y(Alignment::Center);
                column![
                    editor.view(services).map(move |e| edit(Message::Editor(e))),
                    actions
                ]
                .spacing(8)
            }
            None => {
                let details = details(topic, services);
                if can_edit {
                    // In the corner, clear of the text and links.
                    let edit = button(
                        row![icons::pencil(12.0, secondary), text("Edit").size(13)]
                            .spacing(5)
                            .align_y(Alignment::Center),
                    )
                    .padding([4, 10])
                    .style(style::secondary_button)
                    .on_press(message(Message::Edit));
                    column![row![container(details).width(Length::Fill), edit].spacing(12)]
                } else {
                    column![details]
                }
            }
        };
        if let Some(error) = &self.error {
            panel = panel.push(text(error).size(12).style(text::danger));
        }
        panel.spacing(8).into()
    }
}

/// The line the room header previews the topic with, beside the caret:
/// its text's first line, else its links' names, else "Add a topic" if
/// `can_edit`.
pub fn preview(topic: &Topic, services: &[CallService], can_edit: bool) -> Option<String> {
    if let Some(first) = topic.text.lines().next() {
        return Some(first.to_owned());
    }
    if !topic.links.is_empty() {
        let names: Vec<String> = topic.links.iter().map(|l| l.name(services)).collect();
        return Some(names.join(" · "));
    }
    can_edit.then(|| "Add a topic".to_owned())
}

/// The topic's text in full, to select and copy, and a button for each
/// link.
pub fn details<'a>(topic: &Topic, services: &[CallService]) -> Element<'a, app::Message> {
    let mut details = column![].spacing(10);
    if !topic.text.is_empty() {
        details = details.push(selectable_text(topic.text.clone()).size(13));
    }
    let mut links = row![].spacing(6);
    for link in &topic.links {
        let mut label = row![
            kind_icon(link.is_call()),
            text(link.name(services)).size(13)
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        if let Some(passcode) = link.passcode() {
            label = label.push(
                text(format!("passcode {passcode}"))
                    .size(12)
                    .style(text::secondary),
            );
        }
        links = links.push(hint(
            button(label)
                .padding([4, 10])
                .style(style::secondary_button)
                .on_press(app::Message::LinkClicked(link.url.to_string())),
            link.url.to_string(),
            Position::Bottom,
        ));
    }
    if topic.text.is_empty() && topic.links.is_empty() {
        details = details.push(
            text("No topic or links yet.")
                .size(13)
                .style(text::secondary),
        );
    }
    details.push(links.wrap().vertical_spacing(6)).into()
}

/// A camera for a call, else a link.
pub fn kind_icon<'a, M: 'a>(call: bool) -> Element<'a, M> {
    if call {
        icons::camera(14.0, primary_text)
    } else {
        icons::link(14.0, primary_text)
    }
}

/// `topic` with its calls (`calls`) or other links moved: the calls come
/// first.
pub fn moved(mut topic: Topic, calls: bool, moved: (usize, usize)) -> Topic {
    let (mut call_links, mut other): (Vec<_>, Vec<_>) =
        topic.links.into_iter().partition(RoomLink::is_call);
    reorder::apply(if calls { &mut call_links } else { &mut other }, moved);
    topic.links = call_links.into_iter().chain(other).collect();
    topic
}

/// A topic being edited: its text and links, or all of it as text.
pub struct Editor {
    text: text_editor::Content,
    /// The calls, then the other links.
    links: Vec<Draft>,
    /// The link whose fields are open.
    open: Option<usize>,
    /// The whole topic, while edited as text.
    raw: Option<text_editor::Content>,
    calls: Reorder,
    others: Reorder,
}

/// A link as typed.
#[derive(Debug, Clone, Default, PartialEq)]
struct Draft {
    call: bool,
    title: String,
    passcode: String,
    url: String,
}

#[derive(Debug, Clone)]
pub enum Edit {
    Text(text_editor::Action),
    Raw(text_editor::Action),
    AsText(bool),
    /// Add a call (`true`) or another link, with its fields open.
    Add(bool),
    /// Open the fields of the link at this place, or close them.
    Open(Option<usize>),
    Remove(usize),
    Title(usize, String),
    Passcode(usize, String),
    Url(usize, String),
    Calls(reorder::Message),
    Links(reorder::Message),
}

impl Editor {
    pub fn new(topic: &str) -> Self {
        let topic = Topic::parse(topic);
        let mut links: Vec<Draft> = topic.links.iter().map(Draft::from).collect();
        links.sort_by_key(|d| !d.call);
        Self {
            text: text_editor::Content::with_text(&topic.text),
            links,
            open: None,
            raw: None,
            calls: Reorder::default(),
            others: Reorder::default(),
        }
    }

    /// The topic as edited, tidied: its text, then the links.
    pub fn topic(&self) -> String {
        Topic::parse(&self.written()).to_topic()
    }

    /// The topic differs from `topic`.
    pub fn changed(&self, topic: Option<&str>) -> bool {
        self.topic() != Topic::parse(topic.unwrap_or_default()).to_topic()
    }

    /// Every link has a web address.
    pub fn valid(&self) -> bool {
        self.raw.is_some() || self.links.iter().all(|d| d.link().is_some())
    }

    /// The text and the links' lines, as edited; a link without a valid
    /// address as typed.
    fn written(&self) -> String {
        if let Some(raw) = &self.raw {
            return raw.text();
        }
        let links = (self.links.iter())
            .filter(|d| !d.is_blank())
            .map(Draft::line);
        std::iter::once(self.text.text())
            .chain(links)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn call_count(&self) -> usize {
        self.links.iter().take_while(|d| d.call).count()
    }

    pub fn update(&mut self, edit: Edit) {
        let calls = self.call_count();
        match edit {
            Edit::Text(action) => self.text.perform(action),
            Edit::Raw(action) => {
                if let Some(raw) = &mut self.raw {
                    raw.perform(action);
                }
            }
            Edit::AsText(true) => {
                self.raw = Some(text_editor::Content::with_text(&self.written()));
                self.open = None;
            }
            Edit::AsText(false) => {
                if let Some(raw) = self.raw.take() {
                    *self = Self::new(&raw.text());
                }
            }
            Edit::Add(call) => {
                let at = if call { calls } else { self.links.len() };
                self.links.insert(
                    at,
                    Draft {
                        call,
                        ..Draft::default()
                    },
                );
                self.open = Some(at);
            }
            Edit::Open(at) => self.open = at,
            Edit::Remove(at) => {
                if at < self.links.len() {
                    self.links.remove(at);
                }
                self.open = None;
            }
            Edit::Title(at, title) => {
                if let Some(draft) = self.links.get_mut(at) {
                    draft.title = title;
                }
            }
            Edit::Passcode(at, passcode) => {
                if let Some(draft) = self.links.get_mut(at) {
                    draft.passcode = passcode;
                }
            }
            Edit::Url(at, url) => {
                if let Some(draft) = self.links.get_mut(at) {
                    draft.url = url;
                }
            }
            Edit::Calls(message) => {
                if let Some(moved) = self.calls.update(message) {
                    reorder::apply(&mut self.links[..calls], moved);
                    self.open = None;
                }
            }
            Edit::Links(message) => {
                if let Some(moved) = self.others.update(message) {
                    reorder::apply(&mut self.links[calls..], moved);
                    self.open = None;
                }
            }
        }
    }

    /// The switch to the topic as text, and the text and the links, or the
    /// topic as text.
    pub fn view<'a>(&'a self, services: &'a [CallService]) -> Element<'a, Edit> {
        let switch = row![
            space::horizontal(),
            text("As text").size(12).style(text::secondary),
            toggler(self.raw.is_some()).size(16).on_toggle(Edit::AsText),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let body: Element<'a, Edit> = match &self.raw {
            Some(raw) => column![
                field(raw, Edit::Raw)
                    .font(Font::MONOSPACE)
                    .size(12)
                    .min_height(96),
                text("This is the topic other apps show.")
                    .size(11)
                    .style(text::secondary),
            ]
            .spacing(4)
            .into(),
            None => column![
                field(&self.text, Edit::Text)
                    .placeholder("What the room is about")
                    .size(13)
                    .min_height(56),
                self.links_view(services),
                row![add_button(true, "Add call"), add_button(false, "Add link"),].spacing(6),
            ]
            .spacing(8)
            .into(),
        };
        column![switch, body].spacing(6).into()
    }

    /// The calls, then the other links, each group dragged by its grips.
    fn links_view<'a>(&'a self, services: &'a [CallService]) -> Element<'a, Edit> {
        let calls = self.call_count();
        let mut groups = column![].spacing(6);
        for (call, range, order, message) in [
            (true, 0..calls, &self.calls, Edit::Calls as fn(_) -> _),
            (false, calls..self.links.len(), &self.others, Edit::Links),
        ] {
            if range.is_empty() {
                continue;
            }
            let rows = range.map(|at| self.link_view(at, services)).collect();
            groups = groups.push(column![
                text(if call { "Calls" } else { "Links" })
                    .size(11)
                    .font(fonts::SEMIBOLD)
                    .style(text::secondary),
                reorder::list(order, rows, self.open.is_none(), message),
            ]);
        }
        groups.into()
    }

    /// The link at `at`: its name and address, or its fields while open.
    fn link_view<'a>(&'a self, at: usize, services: &'a [CallService]) -> Element<'a, Edit> {
        let draft = &self.links[at];
        let link = draft.link();
        if self.open != Some(at) {
            let name = link
                .as_ref()
                .map_or_else(|| draft.url.clone(), |l| l.name(services));
            let mut detail = Vec::new();
            if draft.call && !draft.passcode.trim().is_empty() {
                detail.push(format!("passcode {}", draft.passcode.trim()));
            }
            detail.push(draft.url.trim().to_owned());
            return row![
                kind_icon(draft.call),
                ellipsis(name).size(13),
                ellipsis(detail.join(" · "))
                    .size(12)
                    .style(text::secondary)
                    .width(Length::Fill),
                icon_button(icons::pencil(12.0, secondary), Edit::Open(Some(at))),
                icon_button(icons::trash(12.0, secondary), Edit::Remove(at)),
            ]
            .spacing(8)
            .padding([3, 4])
            .align_y(Alignment::Center)
            .into();
        }
        let input = |placeholder: &str, value: &str| {
            text_input(placeholder, value)
                .style(style::input)
                .padding([4, 8])
                .size(13)
                .on_submit(Edit::Open(None))
        };
        let title = link
            .as_ref()
            .map_or_else(|| "Title".to_owned(), |l| l.name(services));
        let mut first = row![
            kind_icon(draft.call),
            spelling::checked(input(&title, &draft.title), &draft.title)
                .on_input(move |v| Edit::Title(at, v)),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        if draft.call {
            first = first.push(
                input("Passcode", &draft.passcode)
                    .on_input(move |v| Edit::Passcode(at, v))
                    .width(120),
            );
        }
        let invalid = link.is_none() && !draft.url.trim().is_empty();
        let address = input(
            if draft.call {
                "https://zoom.us/j/123456789"
            } else {
                "https://…"
            },
            &draft.url,
        )
        .on_input(move |v| Edit::Url(at, v))
        .style(move |theme: &Theme, status| {
            let style = style::input(theme, status);
            if invalid {
                text_input::Style {
                    border: style
                        .border
                        .color(theme.extended_palette().danger.base.color),
                    ..style
                }
            } else {
                style
            }
        });
        column![
            first,
            row![
                address,
                button(text("Done").size(12))
                    .padding([4, 10])
                    .style(style::secondary_button)
                    .on_press(Edit::Open(None)),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        ]
        .spacing(4)
        .padding([4, 4])
        .into()
    }
}

impl From<&RoomLink> for Draft {
    fn from(link: &RoomLink) -> Self {
        Self {
            call: link.is_call(),
            title: link.title.clone().unwrap_or_default(),
            passcode: link.passcode().unwrap_or_default().to_owned(),
            url: link.url.to_string(),
        }
    }
}

impl Draft {
    /// The link, if its address is a web address ("https://" may be left
    /// out).
    fn link(&self) -> Option<RoomLink> {
        let url = self.url.trim();
        let url = match Url::parse(url) {
            Ok(url) => url,
            Err(_) if !url.contains("://") => Url::parse(&format!("https://{url}")).ok()?,
            Err(_) => return None,
        };
        if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
            return None;
        }
        let some = |s: &str| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
        Some(RoomLink {
            kind: if self.call {
                LinkKind::Call {
                    passcode: some(&self.passcode),
                }
            } else {
                LinkKind::Link
            },
            title: some(&self.title),
            url,
        })
    }

    /// Nothing typed in.
    fn is_blank(&self) -> bool {
        [&self.title, &self.passcode, &self.url]
            .iter()
            .all(|s| s.trim().is_empty())
    }

    /// Its line in the topic, as typed if not a link yet.
    fn line(&self) -> String {
        match self.link() {
            Some(link) => link.line(),
            None => format!(
                "[{}] {}",
                if self.call { "call" } else { "link" },
                self.url.trim()
            ),
        }
    }
}

/// A text editor with the app's look and keys.
fn field<'a>(
    content: &'a text_editor::Content,
    on_action: fn(text_editor::Action) -> Edit,
) -> iced::widget::TextEditor<'a, highlight::Highlighter, Edit> {
    let selected = content.selection().is_some();
    text_editor(content)
        .highlight_with::<highlight::Highlighter>(Marks::new(Default::default()), highlight::format)
        .on_action(on_action)
        .style(style::editor)
        .padding(8)
        .max_height(240)
        .key_binding(move |press| selection::editor_key(press, selected))
}

fn add_button<'a>(call: bool, label: &'a str) -> Element<'a, Edit> {
    button(
        row![kind_icon(call), text(label).size(12)]
            .spacing(6)
            .align_y(Alignment::Center),
    )
    .padding([3, 10])
    .style(style::secondary_button)
    .on_press(Edit::Add(call))
    .into()
}

fn icon_button<'a>(icon: Element<'a, Edit>, message: Edit) -> Element<'a, Edit> {
    button(icon)
        .padding(4)
        .style(style::text_button)
        .on_press(message)
        .into()
}

fn primary_text(theme: &Theme) -> Color {
    theme.extended_palette().background.base.text
}

fn secondary(theme: &Theme) -> Color {
    theme
        .extended_palette()
        .background
        .base
        .text
        .scale_alpha(0.6)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOPIC: &str = "About.\n\
        [link Draft] https://overleaf.com/project/a\n\
        [call] https://meet.google.com/abc-defg-hij\n\
        [call Weekly passcode 123] https://zoom.us/j/1";

    #[test]
    fn edits_links_and_the_text() {
        let mut editor = Editor::new(TOPIC);
        assert!(!editor.changed(Some(TOPIC)), "only reordered: calls first");
        assert_eq!(
            editor.topic(),
            "About.\n\
             [call] https://meet.google.com/abc-defg-hij\n\
             [call Weekly passcode 123] https://zoom.us/j/1\n\
             [link Draft] https://overleaf.com/project/a"
        );

        // The second call becomes the room's.
        editor.update(Edit::Calls(reorder::Message::Grab(1)));
        editor.update(Edit::Calls(reorder::Message::Over(0)));
        editor.update(Edit::Calls(reorder::Message::Drop));
        editor.update(Edit::Add(false));
        assert!(!editor.valid(), "a link without an address");
        editor.update(Edit::Url(3, "drive.google.com/x".into()));
        editor.update(Edit::Remove(2));
        assert!(editor.valid());
        assert!(editor.changed(Some(TOPIC)));
        assert_eq!(
            editor.topic(),
            "About.\n\
             [call Weekly passcode 123] https://zoom.us/j/1\n\
             [call] https://meet.google.com/abc-defg-hij\n\
             [link] https://drive.google.com/x"
        );
    }

    #[test]
    fn switches_to_text_and_back() {
        let mut editor = Editor::new(TOPIC);
        editor.update(Edit::Add(true));
        editor.update(Edit::Url(2, "not a link".into()));
        editor.update(Edit::AsText(true));
        let Some(raw) = &editor.raw else {
            panic!("not as text")
        };
        assert!(
            (raw.text().trim_end())
                .ends_with("[call] not a link\n[link Draft] https://overleaf.com/project/a"),
            "{}",
            raw.text()
        );
        editor.update(Edit::AsText(false));
        assert_eq!(editor.links.len(), 3, "the unfinished call is text now");
        assert_eq!(editor.text.text().trim(), "About.\n[call] not a link");
    }

    #[test]
    fn moves_calls_or_links() {
        let topic = moved(Topic::parse(TOPIC), true, (1, 0));
        assert_eq!(
            topic.to_topic(),
            "About.\n\
             [call Weekly passcode 123] https://zoom.us/j/1\n\
             [call] https://meet.google.com/abc-defg-hij\n\
             [link Draft] https://overleaf.com/project/a"
        );
    }

    #[test]
    fn previews_the_text_else_the_links() {
        let preview = |topic: &str, can_edit| preview(&Topic::parse(topic), &[], can_edit);
        assert_eq!(preview(TOPIC, false).as_deref(), Some("About."));
        assert_eq!(
            preview(
                "[call Weekly] https://zoom.us/j/1\n[link] https://overleaf.com/a",
                false
            )
            .as_deref(),
            Some("Weekly · overleaf.com")
        );
        assert_eq!(preview("", true).as_deref(), Some("Add a topic"));
        assert_eq!(preview("", false), None);
    }

    #[test]
    fn a_click_outside_closes_unless_changes_are_edited() {
        let mut panel = Panel::default();
        let _ = panel.update(Message::Toggle, "About.");
        let _ = panel.update(Message::Dismiss, "About.");
        assert!(!panel.open);
        // Editing, unchanged.
        let _ = panel.update(Message::Edit, "About.");
        let _ = panel.update(Message::Dismiss, "About.");
        assert!(!panel.open && panel.editor.is_none());
        // Changed: Cancel and Save stand out until the last nudge settles.
        let _ = panel.update(Message::Edit, "About.");
        let _ = panel.update(Message::Editor(Edit::Add(false)), "About.");
        let _ = panel.update(
            Message::Editor(Edit::Url(0, "overleaf.com".into())),
            "About.",
        );
        let _ = panel.update(Message::Dismiss, "About.");
        let _ = panel.update(Message::Dismiss, "About.");
        assert!(panel.open && panel.editor.is_some() && panel.nudge == (2, true));
        let _ = panel.update(Message::Settle(1), "About.");
        assert!(panel.nudge.1);
        let _ = panel.update(Message::Settle(2), "About.");
        assert!(!panel.nudge.1);
    }
}
