//! An open room: timeline and composer.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use iced::{
    Rectangle, Task, Vector,
    advanced::widget::{Id, Operation, operation},
    widget::{scrollable, scrollable::Viewport},
};
use mtrx_core::{
    Draft, DraftKeeper, Event, EventContent, EventKey, Member, OwnedEventId, OwnedRoomId,
    OwnedUserId, RoomHandle, RoomTimeline, Row, ScheduledMessage, SendState, TimelineEntry, Upload,
    User, UserId,
};

use crate::{
    actions::{self, Compose, Editing, InlineEdit, Target},
    app::{self, Shared},
    calls::{self, RoomCalls},
    clipboard::Copied,
    emoji,
    find::{self, Find},
    info,
    media::Attached,
    mention::{self, Mentionable},
    message_field::{self, Field, MessageField},
    pins,
    rows::{self, Part},
    selectable::{self},
    selection::{self, Point, Pos, Selection, Shown, Step},
    send_later,
    status::Status,
    style::{GUTTER_SPACING, MESSAGE_PADDING, SENDER_AVATAR},
    task::Owned,
    topic,
};

mod update;
mod view;

pub use update::{copy, keep_now, left};
pub use view::Look;

/// The topic panel's share of the room header's width.
pub const TOPIC_SHARE: f32 = 2.0 / 3.0;

/// Rows rendered when a room opens; laying out every loaded row gets slow
/// in big rooms, so only a window at the bottom is drawn.
const WINDOW: usize = 80;
/// Rows added to the window when scrolling up.
const WINDOW_STEP: usize = 50;
/// Older events are loaded until there are this many rows (or the room has
/// no more): fewer may not fill the view, and then there is nothing to
/// scroll up for more.
const FILL: usize = 40;

/// Uploads quicker than this do not show in the status pill.
const UPLOAD_SHOWN_AFTER: Duration = Duration::from_millis(500);
/// Right padding of the composer's text, clear of its paperclip and smiley.
const COMPOSER_INSET: f32 = emoji::FIELD_INSET + 29.0;
/// The timeline's scrollable.
pub const TIMELINE: &str = "timeline";

/// How long our message is being sent before its clock shows, so a fast
/// send never flashes one.
pub const SENDING_CLOCK: Duration = Duration::from_millis(300);

/// Left edge of message text within the timeline column.
const TEXT_INDENT: f32 = MESSAGE_PADDING + SENDER_AVATAR + GUTTER_SPACING;

/// What concerns one room: built by its views, or by the tasks it started.
/// The app gets it for a room, [`app::Message::Room`], or for the room
/// shown, [`app::Message::Shown`].
#[derive(Debug, Clone)]
pub enum Message {
    Timeline(Arc<Vec<Row>>),
    /// The draft kept for the room just opened.
    DraftLoaded(Option<Draft>),
    /// The message a restored draft answers is in the timeline: the reply
    /// to write, if it can be answered.
    ReplyFound(Option<Target>),
    Scrolled(Viewport),
    Paginated(Result<(), String>),
    /// The composer, or the message edited in place.
    Field(Field, message_field::Message),
    /// Typed with nothing focused: goes on in the composer.
    TypeInComposer(String),
    Send,
    Sent(Result<(), String>),
    /// Our message in a timeline row started sending a while ago.
    SendingSlow(String),
    Later(send_later::Message),
    Act(actions::Message),
    /// Selecting text in the timeline.
    Select(selection::Message),
    /// The find bar (⌘F).
    Find(find::Message),
    /// Scroll the timeline by this much, while dragging a selection.
    ScrollTimeline(f32),
    /// Page Up and Down, ⌥↑ and ⌥↓, ⌘↑ and ⌘↓ outside a field.
    ScrollKey(Scroll),
    /// Names of others typing.
    Typing(Vec<String>),
    MembersLoaded(Result<Vec<Member>, String>),
    Calls(calls::Message),
    /// The topic and links, in the topic panel.
    Topic(topic::Message),
    Pins(pins::Message),
    /// The files of messages being sent, uploading now.
    Uploads(Vec<Upload>),
    /// Close the problem the status pill shows.
    DismissError,
    /// Stop loading older messages to find the one gone to.
    StopSeeking,
    /// The cursor entered (`true`) or left a timeline event.
    Hover(String, bool),
    /// Scroll to a message, e.g. the original of a reply, loading older
    /// events until it is found.
    GoTo(OwnedEventId),
    /// The row of the message gone to, once older events were loaded for
    /// it, or why it was not found.
    Sought(Result<String, String>),
    /// The message gone to is no longer marked.
    Unmark(String),
    /// Ask for files to attach to the message being written.
    Attach,
    /// A file was dropped onto the window: attach it to the message being
    /// written.
    FileDropped(PathBuf),
    /// Files were read to attach to the message.
    Attached(Vec<Result<Attached, String>>),
    /// Remove an attached file, by index.
    Unattach(usize),
    /// The timeline loaded anew (⌘R).
    Reloaded(Result<Shared<RoomTimeline>, String>),
    /// The server is back: history that could not load while offline.
    Online,
    /// The scheduled messages of all rooms changed.
    Scheduled(Vec<ScheduledMessage>),
}

impl Message {
    /// This message for `room`, as the app gets it.
    pub fn to(self, room: &OwnedRoomId) -> app::Message {
        app::Message::Room(room.clone(), self)
    }
}

pub struct Room {
    pub id: OwnedRoomId,
    /// Its members, pins, calls and scheduled messages are changed through it.
    pub handle: RoomHandle,
    pub timeline: Arc<RoomTimeline>,
    /// The timeline's rows, oldest first.
    pub rows: Arc<Vec<Row>>,
    pub composer: MessageField,
    /// Files sent after the composer's text.
    pub attachments: Vec<Attached>,
    pub calls: RoomCalls,
    /// The topic panel, below the room header; closed when the room
    /// opens.
    pub topic: topic::Panel,
    /// Its pinned messages.
    pub pins: pins::Pins,
    pub compose: Compose,
    /// Keeps the draft ([`Room::draft`]) on this device.
    pub drafts: Arc<DraftKeeper>,
    /// The message a restored draft answers, until it is loaded.
    pub pending_reply: Option<OwnedEventId>,
    /// A sent or scheduled message being edited in place.
    pub editing: Option<InlineEdit>,
    /// Our messages being sent.
    sending: Sending,
    /// Text selected in the timeline.
    pub selection: Option<Selection>,
    /// The find bar (⌘F), while open.
    pub find: Option<Find>,
    /// Names of others typing.
    pub typing: Vec<String>,
    /// The members once loaded, or why loading failed: for the member list
    /// and mentions.
    pub member_list: Option<Result<Vec<Member>, String>>,
    /// What `@` may mention, see [`mention::texts`].
    pub mentions: Arc<[String]>,
    /// How many of the newest rows are rendered.
    shown: usize,
    /// Shown in the status pill until dismissed.
    pub error: Option<String>,
    /// Files of messages being sent, uploading, and since when.
    uploads: Vec<Upload>,
    uploading_since: Option<Instant>,
    /// [`Event::id`] of the message under the cursor.
    hovered: Option<String>,
    /// [`Event::id`] of the message just gone to, e.g. a reply's original.
    pub marked: Option<String>,
    /// What it loads, which ends when it closes.
    pub tasks: Owned<Load>,
}

/// What a room loads, one of each kind at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Load {
    Members,
    /// The pinned messages' timeline.
    Pinned,
    /// Its space's call service.
    CallService,
    Draft,
    /// A new timeline (⌘R).
    Reload,
    /// Older events.
    Paginate,
    /// Older events until the message gone to.
    Seek,
    /// The message a restored draft answers.
    ReplyWait,
}

/// What scrolling to the top should do.
pub enum Older {
    None,
    /// Render more of the already loaded rows.
    ShowMore,
    /// Load older events from the server.
    Paginate,
}

impl Room {
    pub fn new(handle: RoomHandle, timeline: Arc<RoomTimeline>, drafts: DraftKeeper) -> Self {
        let id = handle.id().to_owned();
        // Messages being sent already when the room opens show their clock.
        let rows = Arc::new(timeline.current_rows());
        Self {
            calls: RoomCalls::new(id.clone()),
            topic: topic::Panel::default(),
            pins: pins::Pins::default(),
            compose: Compose::New,
            editing: None,
            sending: Sending::already(&rows),
            selection: None,
            find: None,
            typing: Vec::new(),
            member_list: None,
            mentions: Default::default(),
            id,
            handle,
            rows,
            timeline,
            composer: MessageField::new(Field::Composer, ""),
            drafts: Arc::new(drafts),
            pending_reply: None,
            attachments: Vec::new(),
            shown: WINDOW,
            error: None,
            uploads: Vec::new(),
            uploading_since: None,
            hovered: None,
            marked: None,
            tasks: Owned::default(),
        }
    }

    /// The cursor entered or left the message with [`Event::id`] `id`.
    pub fn hover(&mut self, id: String, entered: bool) {
        if entered {
            self.hovered = Some(id);
        } else if self.hovered.as_ref() == Some(&id) {
            // Entering the next message may have been reported first.
            self.hovered = None;
        }
    }

    /// Reaction to a scroll: near the top (the view is anchored to the
    /// bottom, so the top is relative offset 1) we want older rows.
    pub fn older(&self, viewport: &Viewport) -> Older {
        if viewport.relative_offset().y < 0.9 {
            Older::None
        } else if self.shown < self.rows.len() {
            Older::ShowMore
        } else if !self.tasks.running(&Load::Paginate) && !self.at_start() {
            Older::Paginate
        } else {
            Older::None
        }
    }

    /// The scheduled messages changed: one being edited may have been sent
    /// (it became due) or cancelled elsewhere.
    pub fn scheduled_changed(&mut self, scheduled: &[ScheduledMessage]) {
        let gone = |id: &str| !scheduled.iter().any(|m| m.id == id);
        let Some(edit) = &self.editing else {
            return;
        };
        if !matches!(&edit.target, Editing::Scheduled(id) if gone(id)) {
            return;
        }
        // Keep the changes: in the composer if it is free.
        if self.composer.is_empty() {
            self.composer.set_text(&edit.text());
            self.editing = None;
            self.error = Some(
                "The scheduled message was sent before your changes were saved; \
                 they are in the composer."
                    .into(),
            );
        } else {
            self.error =
                Some("The scheduled message was sent before your changes were saved.".into());
        }
    }

    /// The field `field`: the composer, or the message edited in place
    /// while one is.
    pub fn field_mut(&mut self, field: Field) -> Option<&mut MessageField> {
        match field {
            Field::Composer => Some(&mut self.composer),
            Field::Edit => self.editing.as_mut().map(|edit| &mut edit.field),
        }
    }

    /// Who may be mentioned here: `@room` too if `mention_room` (our
    /// right).
    pub fn mentionable<'a>(&'a self, own: &'a UserId, mention_room: bool) -> Mentionable<'a> {
        Mentionable {
            members: self
                .member_list
                .as_ref()
                .and_then(|members| members.as_deref().ok()),
            texts: &self.mentions,
            own,
            room: mention_room,
        }
    }

    /// `message` to the mention completion of `field`.
    pub fn mention<T>(
        &mut self,
        field: Field,
        message: mention::Message,
        own: &UserId,
        mention_room: bool,
    ) -> Task<T> {
        let members = self.member_list.as_ref().and_then(|m| m.as_deref().ok());
        let mentions = Mentionable {
            members,
            texts: &self.mentions,
            own,
            room: mention_room,
        };
        let target = match field {
            Field::Composer => &mut self.composer,
            Field::Edit => match &mut self.editing {
                Some(edit) => &mut edit.field,
                None => return Task::none(),
            },
        };
        let candidates = target.candidates(&mentions);
        target.mention(message, &candidates)
    }

    /// A mention is being completed, with someone to offer.
    pub fn completing(&self, own: &UserId, mention_room: bool) -> bool {
        let mentions = self.mentionable(own, mention_room);
        self.fields()
            .any(|field| !field.candidates(&mentions).is_empty())
    }

    /// Stop completing mentions.
    pub fn close_completion(&mut self) {
        self.composer.close_completion();
        if let Some(edit) = &mut self.editing {
            edit.field.close_completion();
        }
    }

    /// A mention is being written, and the members it may be are not
    /// loaded yet.
    pub fn wants_members(&self) -> bool {
        self.member_list.is_none()
            && !self.tasks.running(&Load::Members)
            && self.fields().any(|field| field.completion().is_some())
    }

    /// The composer, and the message edited in place.
    fn fields(&self) -> impl Iterator<Item = &MessageField> {
        std::iter::once(&self.composer).chain(self.editing.as_ref().map(|edit| &edit.field))
    }

    /// The members were loaded, or not.
    pub fn members_loaded(&mut self, result: Result<Vec<Member>, String>) {
        self.member_list = Some(result.inspect(|members| {
            self.mentions = mention::texts(members);
        }));
    }

    /// The card of `user`, mentioned here: see [`mentioned`].
    pub fn mentioned(&self, user: OwnedUserId) -> info::Target {
        let members = self.member_list.as_ref().and_then(|m| m.as_deref().ok());
        mentioned(user, members, &self.rows)
    }

    /// What is written and not sent: the composer's text and the message it
    /// answers.
    pub fn draft(&self) -> Draft {
        Draft {
            text: self.composer.text().trim_end().to_owned(),
            reply_to: self
                .compose
                .reply_to()
                .or(self.pending_reply.as_ref())
                .cloned(),
        }
    }

    /// Put back the draft kept for the room (if any), unless something was
    /// written meanwhile; the reply once its message is in the timeline.
    pub fn restore_draft(&mut self, draft: Option<Draft>) -> Task<app::Message> {
        match draft.filter(|_| self.draft().is_empty()) {
            Some(draft) => self.put_back(draft),
            None => Task::none(),
        }
    }

    /// Put `draft` (e.g. of a message taken back) in the composer: after
    /// what is written and a blank line, or as is in an empty composer,
    /// answering its message unless another is answered; the reply once its
    /// message is in the timeline.
    pub fn put_back(&mut self, draft: Draft) -> Task<app::Message> {
        let written = self.composer.text().trim_end().to_owned();
        let empty = written.trim().is_empty();
        let text = if empty {
            draft.text
        } else {
            format!("{written}\n\n{}", draft.text)
        };
        self.composer.set_text(&text);
        let answering = self.compose.reply_to().is_some() || self.pending_reply.is_some();
        let Some(event_id) = draft.reply_to.filter(|_| empty && !answering) else {
            return Task::none();
        };
        self.pending_reply = Some(event_id.clone());
        let (id, timeline) = (self.id.clone(), self.timeline.clone());
        let wait = Task::perform(
            async move {
                let item = timeline.item_of(&event_id).await;
                let entries = timeline.current();
                let event = entries.iter().find_map(|entry| match entry {
                    TimelineEntry::Event(event) if event.id == item => Some(event),
                    _ => None,
                })?;
                let EventContent::Message(m) = &event.content else {
                    return None;
                };
                let preview = first_line(&m.document.to_plain_text());
                Target::of(event, event.sender.display_name(), preview)
            },
            move |target| Message::ReplyFound(target).to(&id),
        );
        self.tasks.run(Load::ReplyWait, wait)
    }

    /// Answer `target`, the message a restored draft answers, unless
    /// another reply was begun meanwhile.
    pub fn reply_found(&mut self, target: Option<Target>) {
        let Some(target) = target else {
            return;
        };
        if self.pending_reply.as_ref() == Some(&target.event_id) && self.compose == Compose::New {
            self.compose = Compose::Reply(target);
        }
        self.pending_reply = None;
    }

    /// Show a freshly loaded timeline, keeping what is being written.
    pub fn reload(&mut self, timeline: Arc<RoomTimeline>) {
        self.rows = Arc::new(timeline.current_rows());
        self.sending = Sending::already(&self.rows);
        self.timeline = timeline;
        self.shown = WINDOW;
        self.error = None;
        // Its rows are new: what loads into the old timeline stops.
        self.tasks.cancel(&Load::Paginate);
        self.tasks.cancel(&Load::Seek);
        self.find_again();
    }

    /// Search the rows again for the find bar's text, as they changed.
    pub fn find_again(&mut self) {
        if let Some(find) = &mut self.find {
            find.search(&self.rows);
        }
    }

    /// Render the message with the current match of the find bar, and
    /// scroll to the match.
    pub fn show_found(&mut self) -> Task<app::Message> {
        let Some(event) = self
            .find
            .as_ref()
            .and_then(Find::current)
            .map(|m| m.event.clone())
        else {
            return Task::none();
        };
        if let Some(index) =
            (self.rows.iter()).position(|row| row.event().is_some_and(|e| e.id == event))
        {
            self.shown = self.shown.max(self.rows.len() - index);
        }
        scroll_to_found(&event)
    }

    /// Follow which of our messages are being sent, once the rows
    /// changed. Returns the rows of those that started: their clock shows
    /// once [`Room::sending_slow`] after [`SENDING_CLOCK`].
    pub fn sending_changed(&mut self) -> Vec<String> {
        self.sending.changed(&self.rows)
    }

    /// Our message in row `id` started sending [`SENDING_CLOCK`] ago: its
    /// clock shows if it still is.
    pub fn sending_slow(&mut self, id: &str) {
        self.sending.slow(id);
    }

    /// Our newest message: sent, it implies the older ones were, and read
    /// markers sit on the last message each reader read, so only it says it
    /// was sent.
    fn newest_own(&self) -> Option<&str> {
        (self.events().rev())
            .find(|event| event.own)
            .map(|event| event.id.as_str())
    }

    /// The events of the message rows, oldest first.
    fn events(&self) -> impl DoubleEndedIterator<Item = &Event> {
        self.rows.iter().filter_map(Row::event)
    }

    /// Back-pagination reached the room's creation.
    pub fn at_start(&self) -> bool {
        matches!(self.rows.first(), Some(Row::Start))
    }

    /// Too few rows are loaded to fill the view.
    pub fn wants_older(&self) -> bool {
        !self.tasks.running(&Load::Paginate) && !self.at_start() && self.rows.len() < FILL
    }

    pub fn show_more(&mut self) {
        self.shown += WINDOW_STEP;
    }

    /// Render every loaded row, to go to the oldest.
    pub fn show_all(&mut self) {
        self.shown = self.shown.max(self.rows.len());
    }

    /// Render the message `event` if it is loaded; its [`Event::id`].
    pub fn reveal(&mut self, event: &OwnedEventId) -> Option<String> {
        let index = (self.rows.iter()).position(|row| {
            row.event()
                .is_some_and(|e| e.event_id.as_ref() == Some(event))
        })?;
        self.shown = self.shown.max(self.rows.len() - index);
        self.rows[index].event().map(|e| e.id.clone())
    }

    /// Render the message with [`Event::id`] `id`, if its row is there.
    pub fn reveal_row(&mut self, id: &str) -> bool {
        let Some(index) =
            (self.rows.iter()).position(|row| row.event().is_some_and(|e| e.id == id))
        else {
            return false;
        };
        self.shown = self.shown.max(self.rows.len() - index);
        true
    }

    /// Our newest message that we may edit, rendered to be edited in place:
    /// its [`Event::id`], key and text.
    pub fn newest_editable(&mut self) -> Option<(String, EventKey, String)> {
        let (event_id, key, text) = self.events().rev().find_map(|event| match &event.content {
            EventContent::Message(m) if event.editable => {
                Some((event.event_id.clone(), event.key.clone(), m.body.clone()))
            }
            _ => None,
        })?;
        // Not while it is being sent.
        let row = self.reveal(&event_id?)?;
        Some((row, key, text))
    }

    fn visible(&self) -> &[Row] {
        &self.rows[self.rows.len().saturating_sub(self.shown)..]
    }

    /// Move the end of the selection by `step`, on into the part of the
    /// timeline's text before or after.
    pub fn extend(&mut self, step: Step) {
        let focus = (self.selection.as_ref()).and_then(|s| self.stepped(&s.focus, step));
        if let Some(selection) = &mut self.selection
            && let Some(focus) = focus
        {
            selection.focus = focus;
        }
    }

    /// `focus` moved by `step`, on into the part before or after; `None`
    /// past the first or last. A reply's preview is stepped through as a
    /// whole, not as cut to fit.
    fn stepped(&self, focus: &Point, step: Step) -> Option<Point> {
        let parts = rows::parts(self.visible());
        let parts = rows::in_order(&parts);
        let units = |part: &Part<'_>| part.units(&Shown::default());
        let at = parts.iter().position(|(_, part)| part.key == focus.event)?;
        if let Some(pos) = selection::step(&units(parts[at].1), focus.pos, step) {
            return Some(Point {
                pos,
                ..focus.clone()
            });
        }
        let next = if step.forward() {
            at + 1
        } else {
            at.checked_sub(1)?
        };
        let (_, next) = parts.get(next)?;
        let pos = if step.forward() {
            Pos::default()
        } else {
            selection::end_of(&units(next))
        };
        Some(Point {
            event: next.key.clone(),
            pos,
        })
    }

    /// Move the end of the selection to the start or end (`forward`) of its
    /// line, by the layout of the text holding it.
    pub fn extend_to_line_edge(&self, forward: bool) -> Task<app::Message> {
        let Some(focus) = self.selection.as_ref().map(|s| s.focus.clone()) else {
            return Task::none();
        };
        let room = self.id.clone();
        selectable::text::line_edge(focus.pos.offset, forward).and_then(move |offset| {
            Task::done(
                Message::Select(selection::Message::LineMoved(
                    focus.clone(),
                    !forward,
                    Some(offset),
                ))
                .to(&room),
            )
        })
    }

    /// Move the end of the selection to the start of the first part on
    /// screen (`up`), or the end of the last.
    pub fn extend_to_screen_edge(&self, up: bool) -> Task<app::Message> {
        if self.selection.is_none() {
            return Task::none();
        }
        let room = self.id.clone();
        self.on_screen().and_then(move |(first, last)| {
            let part = if up { first } else { last };
            Task::done(Message::Select(selection::Message::ExtendToPart(part, up)).to(&room))
        })
    }

    /// Move the end of the selection to the start (`start`) or end of the
    /// part `key`.
    pub fn extend_to_part(&mut self, key: String, start: bool) {
        let parts = rows::parts(self.visible());
        let Some(part) = parts.iter().flatten().find(|part| part.key == key) else {
            return;
        };
        let pos = if start {
            Pos::default()
        } else {
            selection::end_of(&part.units(&Shown::default()))
        };
        if let Some(selection) = &mut self.selection {
            selection.focus = Point { event: key, pos };
        }
    }

    /// Move the end of the selection a line up or down, by the layout of
    /// the text holding it.
    pub fn extend_line(&self, up: bool) -> Task<app::Message> {
        let Some(focus) = self.selection.as_ref().map(|s| s.focus.clone()) else {
            return Task::none();
        };
        let room = self.id.clone();
        selectable::text::line_step(focus.pos.offset, up).map(move |moved| {
            Message::Select(selection::Message::LineMoved(
                focus.clone(),
                up,
                moved.flatten(),
            ))
            .to(&room)
        })
    }

    /// The end of the selection moved from `from` a line up or down to
    /// `offset` in its unit, or past the unit (`None`): on into the one
    /// before or after.
    pub fn line_moved(&mut self, from: Point, up: bool, offset: Option<usize>) {
        let Some(selection) = self.selection.as_mut().filter(|s| s.focus == from) else {
            return;
        };
        match offset {
            Some(offset) => selection.focus.pos.offset = offset,
            None => {
                // From the unit's edge, a step goes on into the next one.
                selection.focus.pos.offset = if up { 0 } else { usize::MAX };
                self.extend(if up { Step::Back } else { Step::Forward });
            }
        }
    }

    /// The first part of the first row with text on screen (partly, or
    /// wholly) and the last part of the last, for ⌘A.
    pub fn on_screen(&self) -> Task<Option<(String, String)>> {
        // Each row with text: its container, and its first and last part.
        let (ids, ends): (Vec<Id>, Vec<(String, String)>) = (self.visible().iter())
            .filter_map(|row| {
                let parts = rows::row_parts(row);
                let ends = (parts.first()?.key.clone(), parts.last()?.key.clone());
                Some((row_container(row, &parts)?, ends))
            })
            .unzip();
        iced::advanced::widget::operate(FindRows {
            rows: ids,
            view: None,
            found: Vec::new(),
        })
        .map(move |range| range.map(|(first, last)| (ends[first].0.clone(), ends[last].1.clone())))
    }

    /// The selected text, as shown and as HTML (`shown` says what reply
    /// previews show, see [`Room::read_shown`]); with several messages,
    /// each sender's name and time before their messages unless it is
    /// selected.
    pub fn selected(&self, shown: &Shown) -> Option<Copied> {
        let selection = self.selection.as_ref()?;
        let parts = rows::parts(self.visible());
        let bounds = selection.ordered(&rows::order(&parts))?;
        rows::copied(self.visible(), &parts, bounds, shown)
    }

    /// What the reply previews on screen show, cut to fit, read from the
    /// view for copying them as shown.
    pub fn read_shown(&self) -> Task<Shown> {
        let ids = (self.visible().iter())
            .flat_map(rows::row_parts)
            .filter(|part| matches!(part.text, rows::Text::Reply(_)))
            .map(|part| (rows::shown_id(&part.key), part.key))
            .collect();
        iced::advanced::widget::operate(ReadShown {
            ids,
            shown: Shown::new(),
        })
    }

    /// The files being uploaded now.
    pub fn set_uploads(&mut self, uploads: Vec<Upload>) {
        self.uploading_since = match (uploads.is_empty(), self.uploading_since) {
            (true, _) => None,
            (false, None) => Some(Instant::now()),
            (false, since) => since,
        };
        self.uploads = uploads;
    }

    /// What the status pill says, if anything: a problem first, then
    /// looking for a message, then uploads taking longer than a moment.
    fn status(&self) -> Option<Status> {
        if let Some(error) = &self.error {
            return Some(Status::Problem(error.clone()));
        }
        if self.tasks.running(&Load::Seek) {
            let back_to = self.events().next().map(|event| event.timestamp);
            return Some(Status::Seeking { back_to });
        }
        let since = self.uploading_since?;
        if since.elapsed() < UPLOAD_SHOWN_AFTER {
            return None;
        }
        let first = self.uploads.first()?;
        let name = self
            .events()
            .find_map(|event| match &event.content {
                EventContent::Message(m) if event.key == first.message => {
                    m.media.as_ref().map(|f| f.filename.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| "a file".to_owned());
        let (sent, total) =
            (self.uploads.iter()).fold((0, 0), |(sent, total), u| (sent + u.sent, total + u.total));
        Some(Status::Uploading {
            name,
            files: self.uploads.len(),
            done: if total == 0 {
                0.0
            } else {
                sent as f32 / total as f32
            },
        })
    }
}

/// Our messages being sent, by row, and whether they show their clock yet:
/// only once sending took [`SENDING_CLOCK`], so a fast send shows none.
#[derive(Debug, Default)]
struct Sending(HashMap<String, bool>);

impl Sending {
    /// Those being sent in `rows` when a room opens: they show their
    /// clock at once.
    fn already(rows: &[Row]) -> Self {
        Self(Self::rows(rows).map(|id| (id.to_owned(), true)).collect())
    }

    /// Follow the `rows`. Returns the rows that started sending.
    fn changed(&mut self, rows: &[Row]) -> Vec<String> {
        let now: Vec<&str> = Self::rows(rows).collect();
        self.0.retain(|id, _| now.contains(&id.as_str()));
        let started: Vec<String> = now
            .into_iter()
            .filter(|id| !self.0.contains_key(*id))
            .map(ToOwned::to_owned)
            .collect();
        for id in &started {
            self.0.insert(id.clone(), false);
        }
        started
    }

    /// Row `id` started sending [`SENDING_CLOCK`] ago.
    fn slow(&mut self, id: &str) {
        if let Some(shown) = self.0.get_mut(id) {
            *shown = true;
        }
    }

    fn shows_clock(&self, id: &str) -> bool {
        self.0.get(id) == Some(&true)
    }

    fn rows(rows: &[Row]) -> impl Iterator<Item = &str> {
        (rows.iter().filter_map(Row::event))
            .filter(|event| event.own && event.send_state == SendState::Sending)
            .map(|event| event.id.as_str())
    }
}

/// The container around a message's row.
fn row_id(event: &str) -> Id {
    Id::from(format!("row:{event}"))
}

/// Scroll the timeline to show the row of message `id` in the middle.
pub fn scroll_to_row(id: &str) -> Task<app::Message> {
    iced::advanced::widget::operate(FindRow {
        row: row_id(id),
        view: None,
        found: None,
    })
    .then(|offset| match offset {
        Some(y) => iced::widget::operation::scroll_to(
            TIMELINE,
            scrollable::AbsoluteOffset {
                x: None,
                y: Some(y),
            },
        ),
        None => Task::none(),
    })
}

/// Where the keys scroll the timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroll {
    /// Page Up, ⌥↑.
    PageUp,
    /// Page Down, ⌥↓.
    PageDown,
    /// ⌘↑: the top, once all loaded entries are shown ([`Room::show_all`]).
    Oldest,
    /// ⌘↓: the bottom, following new messages from there.
    Newest,
}

/// The share of the view a page scrolls, so a little of it stays in view.
const PAGE: f32 = 0.9;

/// Scroll the timeline.
pub fn scroll(scroll: Scroll) -> Task<app::Message> {
    iced::advanced::widget::operate(ScrollTimeline(scroll)).discard()
}

struct ScrollTimeline(Scroll);

impl Operation for ScrollTimeline {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        _translation: Vector,
        state: &mut dyn operation::Scrollable,
    ) {
        if id != Some(&Id::new(TIMELINE)) {
            return;
        }
        // Anchored at the bottom: offsets count upwards from there.
        let page = bounds.height * PAGE;
        match self.0 {
            Scroll::PageUp | Scroll::PageDown => {
                let y = if self.0 == Scroll::PageUp {
                    page
                } else {
                    -page
                };
                state.scroll_by(
                    scrollable::AbsoluteOffset { x: 0.0, y },
                    bounds,
                    content_bounds,
                );
            }
            // Where the top is now: as older entries load above, the view
            // stays rather than going on to the top.
            Scroll::Oldest => state.scroll_to(scrollable::AbsoluteOffset {
                x: None,
                y: Some((content_bounds.height - bounds.height).max(0.0)),
            }),
            Scroll::Newest => state.snap_to(scrollable::RelativeOffset {
                x: None,
                y: Some(0.0),
            }),
        }
    }
}

/// The offset of the timeline (from the bottom, where it is anchored)
/// with a row in the middle.
struct FindRow {
    row: Id,
    /// The timeline's bounds and content height.
    view: Option<(Rectangle, f32)>,
    found: Option<Rectangle>,
}

impl Operation<Option<f32>> for FindRow {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<f32>>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        _translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        if id == Some(&Id::new(TIMELINE)) {
            self.view = Some((bounds, content_bounds.height));
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&self.row) {
            self.found = Some(bounds);
        }
    }

    fn finish(&self) -> operation::Outcome<Option<f32>> {
        operation::Outcome::Some(
            (self.view.zip(self.found)).map(|((view, content), row)| centred(view, content, row)),
        )
    }
}

/// The offset of a timeline (`view`, with `content` high) from the bottom,
/// where it is anchored, with `target` in the middle.
fn centred(view: Rectangle, content: f32, target: Rectangle) -> f32 {
    // From the top, then from the bottom.
    let top = target.y - view.y - (view.height - target.height).max(0.0) / 2.0;
    ((content - view.height).max(0.0) - top.max(0.0)).max(0.0)
}

/// Scroll the timeline to the current match of the find bar, in message
/// `event`, unless it is in view: to the middle, as browsers do.
fn scroll_to_found(event: &str) -> Task<app::Message> {
    iced::advanced::widget::operate(FindFound {
        row: row_id(event),
        view: None,
        found: None,
        row_found: None,
    })
    .then(|offset| match offset {
        Some(y) => iced::widget::operation::scroll_to(
            TIMELINE,
            scrollable::AbsoluteOffset {
                x: None,
                y: Some(y),
            },
        ),
        None => Task::none(),
    })
}

/// Where the timeline should be scrolled to show the current match, if it
/// is not in view.
struct FindFound {
    /// The row of its message, for when its text does not report it (e.g.
    /// while being edited).
    row: Id,
    /// The timeline's bounds, content height and how far it is scrolled.
    view: Option<(Rectangle, f32, Vector)>,
    found: Option<Rectangle>,
    row_found: Option<Rectangle>,
}

impl Operation<Option<f32>> for FindFound {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<f32>>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        if id == Some(&Id::new(TIMELINE)) {
            self.view = Some((bounds, content_bounds.height, translation));
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&self.row) {
            self.row_found = Some(bounds);
        }
    }

    fn custom(&mut self, id: Option<&Id>, bounds: Rectangle, _state: &mut dyn std::any::Any) {
        if id == Some(&Id::new(find::CURRENT)) {
            self.found = Some(bounds);
        }
    }

    fn finish(&self) -> operation::Outcome<Option<f32>> {
        let target = self.found.or(self.row_found);
        operation::Outcome::Some(self.view.zip(target).and_then(
            |((view, content, scrolled), target)| {
                let top = view.y + scrolled.y;
                let shown = target.y >= top && target.y + target.height <= top + view.height;
                (!shown).then(|| centred(view, content, target))
            },
        ))
    }
}

/// What reply previews show, cut to fit: each one's id
/// ([`rows::shown_id`]) and part, and the text found.
struct ReadShown {
    ids: Vec<(Id, String)>,
    shown: Shown,
}

impl Operation<Shown> for ReadShown {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Shown>)) {
        operate(self);
    }

    fn text(&mut self, id: Option<&Id>, _bounds: Rectangle, text: &str) {
        if let Some((_, key)) = id.and_then(|id| self.ids.iter().find(|(shown, _)| shown == id)) {
            self.shown.insert(key.clone(), text.to_owned());
        }
    }

    fn finish(&self) -> operation::Outcome<Shown> {
        operation::Outcome::Some(self.shown.clone())
    }
}

/// Where the timeline is and which messages' rows are in its view.
struct FindRows {
    rows: Vec<Id>,
    /// The timeline's bounds and how far it is scrolled.
    view: Option<(Rectangle, Vector)>,
    /// Each found row: its index in `rows` and bounds.
    found: Vec<(usize, Rectangle)>,
}

impl Operation<Option<(usize, usize)>> for FindRows {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<(usize, usize)>>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        if id == Some(&Id::new(TIMELINE)) {
            self.view = Some((bounds, translation));
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if let Some(index) = id.and_then(|id| self.rows.iter().position(|row| row == id)) {
            self.found.push((index, bounds));
        }
    }

    fn finish(&self) -> operation::Outcome<Option<(usize, usize)>> {
        operation::Outcome::Some(
            self.view
                .and_then(|(view, scrolled)| on_screen(view, scrolled.y, &self.found)),
        )
    }
}

/// The first and last of `rows` (indices and unscrolled bounds) in the
/// view of a timeline scrolled by `scrolled`.
fn on_screen(
    view: Rectangle,
    scrolled: f32,
    rows: &[(usize, Rectangle)],
) -> Option<(usize, usize)> {
    let (top, bottom) = (view.y + scrolled, view.y + scrolled + view.height);
    let shown = rows
        .iter()
        .filter(|(_, row)| row.y < bottom && row.y + row.height > top)
        .map(|(index, _)| *index);
    Some((shown.clone().min()?, shown.max()?))
}

/// The card of `user`, mentioned in `rows`: their name and picture
/// among `members` if loaded, else the name the newest mention shows.
fn mentioned(user: OwnedUserId, members: Option<&[Member]>, rows: &[Row]) -> info::Target {
    let member = members.and_then(|members| members.iter().find(|m| m.user.id == user));
    let name = member.and_then(|m| m.user.name.clone()).or_else(|| {
        (rows.iter().rev().filter_map(Row::event)).find_map(|event| match &event.content {
            EventContent::Message(m) => m.document.mention_name(user.as_str()).map(str::to_owned),
            _ => None,
        })
    });
    let avatar = member.and_then(|m| m.user.avatar.clone());
    info::Target::User(User {
        id: user,
        name,
        avatar,
    })
}

/// The container of `row`, with `parts`, found by [`Room::on_screen`]: a
/// message's by its event, other rows' by their part.
fn row_container(row: &Row, parts: &[Part<'_>]) -> Option<Id> {
    match row {
        Row::Message(message) => Some(row_id(&message.event.id)),
        _ => parts.first().map(|part| row_id(&part.key)),
    }
}

/// The first non-empty line of a message, for previews.
fn first_line(body: &str) -> String {
    body.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use iced::{Point, Size};
    use mtrx_core::TimelineEntry;

    use super::*;

    #[test]
    fn finds_the_messages_on_screen() {
        // A timeline at y 100, 300 high, scrolled 500 down; rows 100 high.
        let view = Rectangle::new(Point::new(0.0, 100.0), Size::new(800.0, 300.0));
        let rows: Vec<_> = (0..12)
            .map(|i| {
                let row = Rectangle::new(
                    Point::new(0.0, 100.0 + 100.0 * i as f32),
                    Size::new(800.0, 100.0),
                );
                (i, row)
            })
            .collect();
        // Content 500..800 shows rows 5, 6 and 7.
        assert_eq!(on_screen(view, 500.0, &rows), Some((5, 7)));
        // Partly shown rows count.
        assert_eq!(on_screen(view, 450.0, &rows), Some((4, 7)));
        assert_eq!(on_screen(view, 5000.0, &rows), None);
    }

    #[test]
    fn scrolls_to_a_match_out_of_view() {
        // A timeline at y 100, 300 high, scrolled 500 down, 2000 of content.
        let view = Rectangle::new(Point::new(0.0, 100.0), Size::new(800.0, 300.0));
        let offset = |y| {
            let find = FindFound {
                row: row_id("$a"),
                view: Some((view, 2000.0, Vector::new(0.0, 500.0))),
                found: Some(Rectangle::new(Point::new(40.0, y), Size::new(30.0, 20.0))),
                row_found: None,
            };
            match find.finish() {
                operation::Outcome::Some(offset) => offset,
                _ => unreachable!(),
            }
        };
        // In view: stays.
        assert_eq!(offset(700.0), None);
        // Above: in the middle, counted from the bottom.
        assert_eq!(offset(300.0), Some(2000.0 - 300.0 - (200.0 - 140.0)));
        // Partly below.
        assert!(offset(890.0).is_some());
    }

    /// A mentioned person's card has their name and picture as a member,
    /// else the name the newest mention of them shows.
    #[test]
    fn a_mentioned_persons_card_has_what_the_room_knows() {
        let nils: OwnedUserId = "@nils:x".try_into().unwrap();
        let mention = |id: &str, name: &str| {
            let mut event = Event::test_message(id, "@ana:x".try_into().unwrap(), name);
            if let EventContent::Message(m) = &mut event.content {
                m.document = mtrx_format::Document::from_html(&format!(
                    r#"Ask <a href="https://matrix.to/#/@nils:x">{name}</a>"#
                ));
            }
            event
        };
        let entries = rows([mention("$a", "Nils"), mention("$b", "Nils W")]);
        let member = Member {
            user: User {
                id: nils.clone(),
                name: Some("Nils Wolf".into()),
                avatar: Some("mxc://x/nils".into()),
            },
            role: mtrx_core::Role::Member,
            level: Some(0),
            membership: mtrx_core::Membership::Joined,
            own: false,
            manage: Default::default(),
        };
        let card = |members: Option<&[Member]>, entries: &[Row]| match mentioned(
            nils.clone(),
            members,
            entries,
        ) {
            info::Target::User(user) => (user.name, user.avatar.map(|a| a.to_string())),
            info::Target::Room(_) => panic!("a room's card"),
        };
        assert_eq!(
            card(Some(&[member]), &entries),
            (Some("Nils Wolf".into()), Some("mxc://x/nils".into()))
        );
        assert_eq!(card(None, &entries), (Some("Nils W".into()), None));
        assert_eq!(card(Some(&[]), &[]), (None, None));
    }

    /// The rows of `events`.
    fn rows(events: impl IntoIterator<Item = Event>) -> Vec<Row> {
        let entries: Vec<_> = events.into_iter().map(TimelineEntry::Event).collect();
        mtrx_core::rows(&entries, &Default::default())
    }

    fn own_message(id: &str, send_state: SendState) -> Vec<Row> {
        let sender = "@alice:example.org".try_into().unwrap();
        rows([Event {
            own: true,
            send_state,
            ..Event::test_message(id, sender, "hi")
        }])
    }

    #[test]
    fn the_sending_clock_shows_after_a_while() {
        let mut sending = Sending::default();
        // A send starts: no clock until it took a while.
        let entries = own_message("a", SendState::Sending);
        assert_eq!(sending.changed(&entries), ["a"]);
        assert!(!sending.shows_clock("a"));
        assert!(sending.changed(&entries).is_empty(), "started once");
        sending.slow("a");
        assert!(sending.shows_clock("a"));
        // Sent: gone; a late timer changes nothing.
        assert!(
            sending
                .changed(&own_message("a", SendState::Sent))
                .is_empty()
        );
        sending.slow("a");
        assert!(!sending.shows_clock("a"));
        // Retried after failing: waits again.
        assert_eq!(sending.changed(&entries), ["a"]);
        assert!(!sending.shows_clock("a"));
        // Being sent when the room opens: at once.
        assert!(Sending::already(&entries).shows_clock("a"));
    }
}
