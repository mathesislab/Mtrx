//! An open room: timeline and composer.

use std::{
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use iced::{
    Alignment, Element, Length, Padding, Rectangle, Task, Theme, Vector,
    advanced::widget::{Id, Operation, operation},
    keyboard::{Key, key::Named},
    mouse, task,
    widget::{
        button, column, container, image, mouse_area, row, rule, scrollable, scrollable::Viewport,
        space, span, stack, text, text_editor,
    },
};
use mtrx_core::{
    CallService, Draft, DraftKeeper, Event, EventContent, EventKey, Media, Member, MessageKind,
    MessageRow, MxcUri, OwnedEventId, OwnedRoomId, OwnedUserId, RoomKind, RoomSummary,
    RoomTimeline, Row, ScheduledMessage, SendState, TimelineEntry, Topic, Upload, UserId,
};

use crate::{
    actions::{self, Compose, Editing, InlineEdit, PickerAt, Target, act},
    app::{self, Popup, Shared},
    avatar::{self, Avatars, Shape},
    calls::{self, RoomCalls},
    clipboard::{self, Copied, Field},
    context_menu::context_menu,
    drag_scroll::drag_scroll,
    edit_menu,
    ellipsis::ellipsis,
    emoji,
    find::{self, Find},
    fonts,
    highlight::{self, Marks},
    hint::{Position, hint},
    icons, info,
    math::{self, Math, Rendered},
    media::{self, Attached, Previews},
    members,
    mention::{self, Candidate, Completion},
    message,
    message_menu::{self, MenuAt},
    pins,
    popover::{Edge, Side, popover},
    presence::{self, Presences},
    rows::{self, Part, display_name, time},
    selectable::{self, selectable},
    selectable_image::selectable_image,
    selection::{self, Bounds, Point, Pos, Selection, Shown, Step},
    send_later, sidebar,
    status::{self, Status},
    style::{
        self, GUTTER_SPACING, HEADER_AVATAR, MESSAGE_PADDING, SENDER_AVATAR, TEXT_MARGIN,
        TIMELINE_PADDING, TOOLBAR_HEIGHT,
    },
    topic,
};

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
/// The message field, focused again after inserting an emoji.
pub const COMPOSER: &str = "composer";
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
    Composer(text_editor::Action),
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
    /// Completing a mention in the composer.
    Mention(mention::Message),
    /// The members, and whether we may mention everyone (`@room`).
    MembersLoaded(Result<(Vec<Member>, bool), String>),
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
    /// ⌘V (or ⌥⇧⌘V) in a field: read the clipboard.
    Paste(Field, clipboard::Paste),
    /// What the clipboard held, to paste into a field.
    Pasted(Field, Option<clipboard::Pasted>),
    /// The menu of a field one writes in.
    EditMenu(Field, edit_menu::Message),
    /// Remove an attached file, by index.
    Unattach(usize),
    /// The timeline loaded anew (⌘R).
    Reloaded(Result<Shared<RoomTimeline>, String>),
    /// The composer's emoji panel.
    ComposerEmoji(emoji::Message),
    /// Put an emoji into the composer.
    InsertEmoji(String),
}

impl Message {
    /// This message for `room`, as the app gets it.
    pub fn to(self, room: &OwnedRoomId) -> app::Message {
        app::Message::Room(room.clone(), self)
    }
}

pub struct Room {
    pub id: OwnedRoomId,
    pub timeline: Arc<RoomTimeline>,
    /// The timeline's rows, oldest first.
    pub rows: Arc<Vec<Row>>,
    pub composer: text_editor::Content,
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
    /// Waits for the message a restored draft answers.
    _reply_wait: Option<task::Handle>,
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
    /// The members are being loaded.
    pub loading_members: bool,
    /// What `@` may mention, see [`mention::texts`].
    pub mentions: Arc<[String]>,
    /// We may mention everyone with `@room`.
    pub can_mention_room: bool,
    /// Completing a mention in the composer.
    pub completion: Option<Completion>,
    /// How many of the newest rows are rendered.
    shown: usize,
    pub paginating: bool,
    /// A new timeline is being loaded (⌘R).
    pub reloading: bool,
    /// Shown in the status pill until dismissed.
    pub error: Option<String>,
    /// Files of messages being sent, uploading, and since when.
    uploads: Vec<Upload>,
    uploading_since: Option<Instant>,
    /// [`Event::id`] of the message under the cursor.
    hovered: Option<String>,
    /// [`Event::id`] of the message just gone to, e.g. a reply's original.
    pub marked: Option<String>,
    /// Loads older events to find the message gone to; dropping it stops.
    pub seek: Option<task::Handle>,
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
    pub fn new(id: OwnedRoomId, timeline: Arc<RoomTimeline>, drafts: DraftKeeper) -> Self {
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
            loading_members: false,
            mentions: Default::default(),
            can_mention_room: false,
            completion: None,
            id,
            rows,
            timeline,
            composer: text_editor::Content::new(),
            drafts: Arc::new(drafts),
            pending_reply: None,
            _reply_wait: None,
            attachments: Vec::new(),
            shown: WINDOW,
            paginating: false,
            reloading: false,
            error: None,
            uploads: Vec::new(),
            uploading_since: None,
            hovered: None,
            marked: None,
            seek: None,
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
        } else if !self.paginating && !self.at_start() {
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
        if self.composer.text().trim().is_empty() {
            self.set_composer(text_editor::Content::with_text(&edit.text()));
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

    /// Replace what the composer holds.
    pub fn set_composer(&mut self, content: text_editor::Content) {
        self.composer = content;
        self.update_completion();
    }

    /// The composer's text or cursor changed: complete the mention there.
    pub fn update_completion(&mut self) {
        self.completion = Completion::of(&self.composer, self.completion.as_ref());
    }

    /// Who the mention being completed may be.
    pub fn candidates(&self, own: &UserId) -> Vec<Candidate<'_>> {
        match (&self.completion, &self.member_list) {
            (Some(completion), Some(Ok(members))) => mention::candidates(
                &completion.query.text,
                members,
                &self.mentions,
                own,
                self.can_mention_room,
            ),
            _ => Vec::new(),
        }
    }

    /// The members were loaded (with whether we may mention the room), or
    /// not.
    pub fn members_loaded(&mut self, result: Result<(Vec<Member>, bool), String>) {
        self.loading_members = false;
        self.member_list = Some(result.map(|(members, can_mention_room)| {
            self.mentions = mention::texts(&members);
            self.can_mention_room = can_mention_room;
            members
        }));
    }

    /// The card of `user`, mentioned here: see [`mentioned`].
    pub fn mentioned(&self, user: OwnedUserId) -> info::Target {
        let members = self.member_list.as_ref().and_then(|m| m.as_deref().ok());
        mentioned(user, members, &self.rows)
    }

    /// Complete the mention being written with `mention`.
    pub fn complete(&mut self, mention: &str) {
        if let Some(completion) = self.completion.take() {
            mention::insert(&mut self.composer, &completion.query, mention);
        }
        self.update_completion();
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
        let Some(draft) = draft.filter(|_| self.draft().is_empty()) else {
            return Task::none();
        };
        let mut composer = text_editor::Content::with_text(&draft.text);
        composer.perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
        self.set_composer(composer);
        let Some(event_id) = draft.reply_to else {
            return Task::none();
        };
        self.pending_reply = Some(event_id.clone());
        let (id, timeline) = (self.id.clone(), self.timeline.clone());
        let (wait, handle) = Task::perform(
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
                Target::of(event, display_name(event), preview)
            },
            move |target| Message::ReplyFound(target).to(&id),
        )
        .abortable();
        self._reply_wait = Some(handle.abort_on_drop());
        wait
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
        self.paginating = false;
        self.reloading = false;
        self.error = None;
        // Its rows are new.
        self.seek = None;
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
        !self.paginating && !self.at_start() && self.rows.len() < FILL
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

    /// The events of the rendered message rows.
    fn visible_events(&self) -> impl Iterator<Item = &Event> {
        self.visible().iter().filter_map(Row::event)
    }

    /// Rendered messages with math, for typesetting.
    pub fn math_documents(&self) -> impl Iterator<Item = &mtrx_format::Document> {
        self.visible_events().filter_map(|e| match &e.content {
            EventContent::Message(m) if m.document.has_math() => Some(&m.document),
            _ => None,
        })
    }

    /// Pictures of the header and the rendered senders, with their sizes.
    /// `members_shown`: with the member list's; `popup`: with those it
    /// lists.
    pub fn avatars<'a>(
        &'a self,
        summary: Option<&'a RoomSummary>,
        own: &UserId,
        members_shown: bool,
        popup: Option<&'a Popup>,
    ) -> impl Iterator<Item = (&'a MxcUri, f32)> {
        let room = summary.and_then(|s| Some((s.avatar.as_deref()?, HEADER_AVATAR)));
        let senders = self.visible_events().flat_map(move |event| {
            let sender = event.sender_avatar.as_deref().map(|a| (a, SENDER_AVATAR));
            let listed = matches!(popup, Some(Popup::Readers(row)) if *row == event.id);
            let reactors = match popup {
                Some(Popup::Reactors { row, key }) if *row == event.id => Some(key.as_str()),
                _ => None,
            };
            sender
                .into_iter()
                .chain(actions::reader_avatars(event, listed))
                .chain(actions::reactor_avatars(event, reactors))
        });
        let listed = match &self.member_list {
            Some(Ok(list)) if members_shown => list.as_slice(),
            _ => &[],
        };
        let members = members::avatars(listed);
        let candidates = self
            .candidates(own)
            .into_iter()
            .filter_map(|candidate| match candidate {
                Candidate::Member { member, .. } => Some(member),
                Candidate::Room => None,
            });
        let completed = members::avatars_of(candidates);
        room.into_iter()
            .chain(senders)
            .chain(members)
            .chain(completed)
    }

    /// Images of the rendered messages, for loading their previews.
    /// Files with previews: images and PDFs.
    pub fn previewed(&self) -> impl Iterator<Item = &Media> {
        self.visible_events().filter_map(|e| match &e.content {
            EventContent::Message(m) => m.media.as_ref().filter(|media| media::previewed(media)),
            _ => None,
        })
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
        selectable::line_edge(focus.pos.offset, forward).and_then(move |offset| {
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
        selectable::line_step(focus.pos.offset, up).map(move |moved| {
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
        if self.seek.is_some() {
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

    pub fn view<'a>(
        &'a self,
        summary: Option<&'a RoomSummary>,
        look: &Look<'a>,
        scheduled: &send_later::Pending<'a>,
    ) -> Element<'a, app::Message> {
        let header = self.header(summary, look);
        let rows = self.visible();
        let parts = rows::parts(rows);
        let order = self.selection.as_ref().map(|_| rows::order(&parts));
        let selected = self
            .selection
            .as_ref()
            .zip(order.as_ref())
            .and_then(|(selection, order)| Some((selection.ordered(order)?, order)));
        let id = &self.id;
        let popup = look.popup;
        let context = Context {
            room: id,
            theme: &look.theme,
            own_user: look.own_user,
            math: look.math,
            params: &look.params,
            avatars: look.avatars,
            previews: look.previews,
            colored_names: look.colored_names,
            call_services: look.call_services,
            hovered: self.hovered.as_deref(),
            marked: self.marked.as_deref(),
            picker: match popup {
                Some(Popup::Reactions { row, at, .. }) => Some((row.as_str(), *at)),
                _ => None,
            },
            menu: match popup {
                Some(Popup::MessageMenu { row, at }) => Some((row.as_str(), at)),
                _ => None,
            },
            readers: match popup {
                Some(Popup::Readers(row)) => Some(row.as_str()),
                _ => None,
            },
            reactors: match popup {
                Some(Popup::Reactors { row, key }) => Some((row.as_str(), key.as_str())),
                _ => None,
            },
            unsent: match popup {
                Some(Popup::Unsent(row)) => Some(row.as_str()),
                _ => None,
            },
            newest_own: self.newest_own(),
            sending: &self.sending,
            selected,
            found: self.find.as_ref(),
            focus: self.selection.as_ref().map(|s| &s.focus),
            dragging: self.selection.as_ref().is_some_and(|s| s.dragging),
            emoji: match popup {
                Some(Popup::Reactions { all, .. }) => all.as_ref(),
                _ => None,
            },
            frequent_emoji: look.frequent_emoji,
            confirm_delete: match popup {
                Some(Popup::ConfirmDelete(row)) => Some(row.as_str()),
                _ => None,
            },
            editing: self.editing.as_ref(),
            popup,
        };

        let mut timeline = column![].padding([8.0, TIMELINE_PADDING]);
        let all_shown = self.shown >= self.rows.len();
        if self.reloading {
            timeline = timeline.push(notice("Reloading…"));
        } else if !(self.at_start() && all_shown) && (self.paginating || !all_shown) {
            timeline = timeline.push(notice("Loading…"));
        }
        for (row, parts) in rows.iter().zip(&parts) {
            let view = match row {
                Row::Message(message) => event_view(message, parts, &context),
                Row::Day(_) => day_divider(parts, &context),
                Row::ReadMarker => read_marker(),
                Row::Summary { .. } | Row::Start => line(parts, &context),
            };
            timeline = timeline.push(view);
        }
        // Pegged to the bottom also when shorter than the view: new messages
        // push older ones up, and loaded history appears above.
        let timeline = scrollable(container(timeline).align_bottom(Length::Shrink))
            .id(TIMELINE)
            .anchor_bottom()
            .on_scroll(|viewport| Message::Scrolled(viewport).to(id))
            .height(Length::Fill)
            .width(Length::Fill);
        // A click beside the text clears the selection; dragging one past
        // the top or bottom scrolls.
        let dragging = self.selection.as_ref().is_some_and(|s| s.dragging);
        let timeline = drag_scroll(
            mouse_area(timeline).on_press(Message::Select(selection::Message::Clear).to(id)),
            dragging,
            |dy| Message::ScrollTimeline(dy).to(id),
        );

        let cancellable = self.compose.cancellable();
        let candidates = self.candidates(look.own_user);
        let completing = !candidates.is_empty();
        let selected = self.composer.selection().is_some();
        let empty = self.composer.text().is_empty();
        let composer = text_editor(&self.composer)
            .id(COMPOSER)
            .highlight_with::<highlight::Highlighter>(
                Marks::new(self.mentions.clone()),
                highlight::format,
            )
            .style(style::editor)
            .placeholder("Message…  (Markdown, $…$ for math; Shift+Enter for a new line)")
            .on_action(|action| Message::Composer(action).to(id))
            .key_binding(move |press| {
                let custom = |message: Message| Some(text_editor::Binding::Custom(message.to(id)));
                // The window's: going to another room.
                if sidebar::Go::from_key(&press.key, press.physical_key, press.modifiers).is_some()
                {
                    return None;
                }
                // Pictures, files and rich text too, which iced's clipboard
                // lacks.
                if let Some(how) = clipboard::paste(&press) {
                    return custom(Message::Paste(Field::Composer, how));
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
                    Key::Named(Named::Enter) if !press.modifiers.shift() => custom(Message::Send),
                    Key::Named(Named::Escape) if cancellable => {
                        custom(Message::Act(actions::Message::CancelCompose))
                    }
                    // Bindings are asked also while not focused.
                    Key::Named(Named::ArrowUp)
                        if empty
                            && press.modifiers.is_empty()
                            && matches!(press.status, text_editor::Status::Focused { .. }) =>
                    {
                        custom(Message::Act(actions::Message::EditLast))
                    }
                    _ => selection::editor_key(press, selected),
                }
            })
            .padding(
                Padding::new(10.0)
                    .right(COMPOSER_INSET)
                    .top(if self.attachments.is_empty() {
                        10.0
                    } else {
                        media::CHIP_HEIGHT + 18.0
                    }),
            )
            .max_height(200);
        let (field_menu, field_emoji) = in_field(popup, Field::Composer);
        let composer = edit_menu::with_menu(id, Field::Composer, composer, field_menu, selected);
        let attach = hint(
            button(icons::paperclip(19.0, |theme| {
                theme
                    .extended_palette()
                    .background
                    .base
                    .text
                    .scale_alpha(0.55)
            }))
            .padding(4)
            .style(style::list_item(false))
            .on_press(Message::Attach.to(id)),
            "Attach files (or drop them on the window)",
            Position::Top,
        );
        let toggle = emoji::toggle(
            field_emoji.is_some(),
            false,
            Message::ComposerEmoji(emoji::Message::Open).to(id),
        );
        // The attached files sit inside the field, above the text.
        let mut composer = stack![
            composer,
            container(row![attach, toggle].spacing(2))
                .align_right(Length::Fill)
                .align_bottom(Length::Fill)
                .padding(Padding::new(6.0).top(0)),
        ];
        if !self.attachments.is_empty() {
            composer = composer.push(
                container(media::attached_view(&self.attachments, |index| {
                    Message::Unattach(index).to(id)
                }))
                .padding(Padding::new(8.0).right(COMPOSER_INSET)),
            );
        }
        let panel = field_emoji.map(|panel| {
            panel.view(
                look.frequent_emoji,
                |emoji| Message::InsertEmoji(emoji).to(id),
                |message| Message::ComposerEmoji(message).to(id),
            )
        });
        let composer = popover(composer, panel, Side::Above, Edge::Right)
            .on_dismiss(Message::ComposerEmoji(emoji::Message::Close).to(id))
            .click_through();
        let selected = self.completion.as_ref().map_or(0, |c| c.selected);
        let completion = completing.then(|| mention::view(id, &candidates, selected, look.avatars));
        let composer = popover(composer, completion, Side::Above, Edge::Left)
            .on_dismiss(Message::Mention(mention::Message::Close).to(id))
            .click_through();
        let composer_empty = self.composer.text().trim().is_empty() && self.attachments.is_empty();
        let send = hint(
            button(icons::send(18.0, |theme| {
                theme.extended_palette().primary.base.text
            }))
            .padding([9, 12])
            .style(style::primary_button)
            .on_press_maybe((!composer_empty).then(|| Message::Send.to(id))),
            "Send (Enter)",
            Position::Top,
        );
        // The send time picked for the composer's text, or a scheduled
        // message's.
        let (scheduling, rescheduling) = match popup {
            Some(Popup::SendLater(later)) => match &later.rescheduling {
                None => (Some(later), None),
                Some(_) => (None, Some(later)),
            },
            _ => (None, None),
        };
        // While open, a click on it closes the picker as any outside click.
        let picking = scheduling.is_some();
        let later = hint(
            button(icons::clock(18.0, |theme| {
                theme
                    .extended_palette()
                    .background
                    .base
                    .text
                    .scale_alpha(0.75)
            }))
            .padding([9, 9])
            .style(style::secondary_button)
            .on_press_maybe((!picking).then(|| Message::Later(send_later::Message::Open).to(id))),
            "Send later: pick a time",
            Position::Top,
        );
        let picker =
            scheduling.map(|later| send_later::picker(id, later, !composer_empty, scheduled.now));
        let send = popover(
            row![send, later].spacing(4),
            picker,
            Side::Above,
            Edge::Right,
        )
        .on_dismiss(Message::Later(send_later::Message::Close).to(id));

        // Bars above and below the composer come and go inside columns that
        // are always there: widgets keep their state by position, so the
        // composer must stay where it is to keep its focus.
        let mut above = column![].spacing(6);
        if !scheduled.messages.is_empty() {
            above = above.push(send_later::queue(
                id,
                scheduled,
                rescheduling,
                self.editing.as_ref(),
                look.frequent_emoji,
                popup,
            ));
        }
        above = above.push(actions::typing(&self.typing));
        if let Some(bar) = self.compose.bar(id) {
            above = above.push(bar);
        }
        let bottom = column![
            above,
            row![composer, send].spacing(8).align_y(Alignment::End),
        ]
        .spacing(6)
        .padding([8.0, TIMELINE_PADDING + MESSAGE_PADDING]);

        // Always there, so the timeline and composer keep their state.
        let mut find_bar = column![];
        if let Some(find) = &self.find {
            find_bar = find_bar.push(find.view(|m| m).map(|m| Message::Find(m).to(id)));
            find_bar = find_bar.push(rule::horizontal(1));
        }
        // What the room waits on, or what failed, over the timeline's
        // bottom left.
        let timeline: Element<'a, app::Message> = match self.status() {
            Some(status) => stack![
                timeline,
                container(status::view(id, &status))
                    .align_left(Length::Fill)
                    .align_bottom(Length::Fill)
                    .padding(Padding::new(8.0).left(TIMELINE_PADDING + MESSAGE_PADDING)),
            ]
            .into(),
            None => timeline.into(),
        };
        let main = column![find_bar, timeline, bottom].width(Length::Fill);
        // The member list comes after, so it does not move the composer.
        let mut body = row![main];
        if look.show_members {
            let name = summary.map_or("", |s| s.name.as_str());
            body = body.push(members::view(
                self.member_list.as_ref(),
                name,
                members::Shown {
                    room: &self.id,
                    avatars: look.avatars,
                    presences: look.presences,
                    menu: app::Popup::member_menu(popup),
                },
            ));
        }
        column![header, body].into()
    }

    /// Picture, name and topic, the call controls, and their menu when
    /// open; below, the topic in full and its links while the caret beside
    /// the name opens them.
    fn header<'a>(
        &'a self,
        summary: Option<&'a RoomSummary>,
        look: &Look<'a>,
    ) -> Element<'a, app::Message> {
        let avatars = look.avatars;
        let name = summary.map_or("", |s| s.name.as_str());
        let topic = Topic::parse(self.topic.shown(summary.and_then(|s| s.topic.as_deref())));
        let can_edit = self.topic.can_edit == Some(true);
        // A direct chat's person, as long as their presence is known.
        let presence = summary
            .filter(|s| s.kind == RoomKind::Direct && !s.is_notes(look.own_user))
            .and_then(|s| s.direct_user.as_ref())
            .and_then(|user| look.presences.get(user));
        // The name (and the person's presence) and picture open the room's
        // info card.
        let info = summary.map_or(info::Target::Room(self.id.clone()), info::target_of);
        let about_tip = match summary.map(|s| s.kind) {
            Some(RoomKind::Direct) => "Who this is",
            _ => "Room settings and address",
        };
        let opens_info = |content: Element<'a, app::Message>| {
            hint(
                mouse_area(content)
                    .interaction(mouse::Interaction::Pointer)
                    .on_press(app::Message::ShowInfo(info.clone())),
                about_tip,
                Position::Bottom,
            )
        };
        let mut about = column![ellipsis(name).size(16).font(fonts::SEMIBOLD)];
        if let Some(line) = presence.and_then(|p| p.line(look.now)) {
            about = about.push(ellipsis(line).size(12).style(text::secondary));
        }
        // A room just created or joined is listed once sync delivers it.
        if summary.is_none() {
            about = about.push(
                text("Setting up the room… it appears in the list in a moment.")
                    .size(12)
                    .style(text::secondary),
            );
        }
        let mut title = column![opens_info(about.into())];
        // The caret, then a preview of the topic and how many messages are
        // pinned while closed: open or close the topic panel. While it is
        // open, a click here is outside it and closes it.
        let preview = topic::preview(&topic, look.call_services, can_edit);
        let pinned = summary.map_or(&[][..], |s| &s.pinned);
        if summary.is_some() && (preview.is_some() || !pinned.is_empty()) {
            let open = self.topic.open;
            // As the section headers' caret, between the middle of the
            // capitals and that of the lowercase letters.
            let caret = container(icons::chevron(12.0, open, |theme| {
                theme
                    .extended_palette()
                    .background
                    .base
                    .text
                    .scale_alpha(0.7)
            }))
            .padding(Padding::ZERO.top(1));
            let mut line = row![caret].spacing(5).align_y(Alignment::Center);
            if !open && !pinned.is_empty() {
                line = line.push(
                    row![
                        icons::pin(11.0, |theme| {
                            theme
                                .extended_palette()
                                .background
                                .base
                                .text
                                .scale_alpha(0.55)
                        }),
                        text(pinned.len().to_string())
                            .size(12)
                            .style(text::secondary),
                    ]
                    .spacing(2)
                    .align_y(Alignment::Center),
                );
            }
            if !open && let Some(preview) = preview {
                line = line.push(ellipsis(preview).size(12).style(text::secondary));
            }
            title = title.push(
                mouse_area(line)
                    .interaction(mouse::Interaction::Pointer)
                    .on_press(Message::Topic(topic::Message::Toggle).to(&self.id)),
            );
        }
        let avatar = avatars.view(
            self.id.as_str(),
            name,
            summary.and_then(|s| s.avatar.as_deref()),
            HEADER_AVATAR,
            Shape::Circle,
        );
        let avatar = presence::with_dot(avatar, presence, HEADER_AVATAR, |theme| {
            theme.extended_palette().background.base.color
        });
        let about = row![opens_info(avatar), title.width(Length::Fill)]
            .spacing(10)
            .align_y(Alignment::Center);
        let members_tip = if look.show_members {
            "Hide the member list"
        } else {
            "Show who is in the room"
        };
        let header = column![
            row![
                about,
                self.calls.header_controls(
                    &topic,
                    look.call_services,
                    can_edit,
                    match look.popup {
                        Some(Popup::Calls(menu)) => Some(menu),
                        _ => None,
                    },
                ),
                hint(
                    button(text("Members").size(13))
                        .padding([5, 10])
                        .style(style::list_item(look.show_members))
                        .on_press(app::Message::ToggleMembers),
                    members_tip,
                    Position::Bottom,
                ),
                hint(
                    button(icons::gear(16.0, |theme| {
                        theme
                            .extended_palette()
                            .background
                            .base
                            .text
                            .scale_alpha(0.7)
                    }))
                    .padding([5, 8])
                    .style(style::list_item(false))
                    .on_press(app::Message::ShowInfo(info::Target::Room(self.id.clone()))),
                    "Room settings: name, notifications, favourite",
                    Position::Bottom,
                ),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .padding([8.0, TIMELINE_PADDING + MESSAGE_PADDING]),
        ];
        let header = header.push(rule::horizontal(1).style(style::separator));
        // The topic in full and its links, or their editor, dropping over
        // the timeline below the room's picture.
        let panel = self.topic.open.then(|| {
            let topic = self.topic.view(&topic, look.call_services, move |message| {
                Message::Topic(message).to(&self.id)
            });
            let mut content = column![topic].spacing(14);
            // Not while the topic is edited.
            if self.topic.editor.is_none() {
                content = content.push(self.pins.view(&self.id, pinned));
            }
            container(scrollable(container(content).padding(14)).height(Length::Shrink))
                .width(Length::Fill)
                .style(style::toolbar)
                .into()
        });
        popover(header, panel, Side::Below, Edge::Left)
            .share_of_anchor(TOPIC_SHARE, TIMELINE_PADDING + MESSAGE_PADDING)
            .on_dismiss(Message::Topic(topic::Message::Dismiss).to(&self.id))
            .into()
    }
}

/// What rendering a room needs from the rest of the app.
pub struct Look<'a> {
    pub theme: Theme,
    pub math: &'a Math,
    pub params: math::Params,
    pub avatars: &'a Avatars,
    /// Senders' names in their colour instead of the text colour.
    pub colored_names: bool,
    /// Our call services, to recognise and start calls.
    pub call_services: &'a [CallService],
    pub own_user: &'a OwnedUserId,
    pub previews: &'a Previews,
    /// The user's most used emoji, for reactions.
    pub frequent_emoji: &'a [String],
    /// The member list is shown beside the room.
    pub show_members: bool,
    /// The menu or picker open in the window.
    pub popup: Option<&'a Popup>,
    /// People's presence, in the member list and a direct chat's header.
    pub presences: &'a Presences,
    /// For how long ago people were last active.
    pub now: SystemTime,
}

/// What rendering an event needs besides the event.
struct Context<'a, 'b> {
    /// The room the event is in.
    room: &'b OwnedRoomId,
    theme: &'b Theme,
    /// Us, whose mentions stand out.
    own_user: &'b UserId,
    math: &'a Math,
    params: &'b math::Params,
    avatars: &'b Avatars,
    previews: &'b Previews,
    colored_names: bool,
    call_services: &'b [CallService],
    hovered: Option<&'b str>,
    marked: Option<&'b str>,
    picker: Option<(&'b str, PickerAt)>,
    menu: Option<(&'b str, &'b MenuAt)>,
    /// The row whose readers are listed.
    readers: Option<&'b str>,
    /// The row and reaction key whose people are listed.
    reactors: Option<(&'b str, &'b str)>,
    /// The row whose unsent message is offered to retry or delete.
    unsent: Option<&'b str>,
    /// Our newest message, the only one to say it was sent.
    newest_own: Option<&'b str>,
    /// Our messages being sent.
    sending: &'b Sending,
    /// The selection, with each message's index in the timeline.
    selected: Option<(Bounds, &'b HashMap<&'b str, usize>)>,
    /// The find bar, to highlight its matches.
    found: Option<&'b Find>,
    /// Where the selection ends.
    focus: Option<&'b Point>,
    dragging: bool,
    emoji: Option<&'a emoji::Panel>,
    /// The user's most used emoji.
    frequent_emoji: &'b [String],
    confirm_delete: Option<&'b str>,
    /// The message being edited in place, if any.
    editing: Option<&'a InlineEdit>,
    /// The menu or picker open in the window, for the message edited.
    popup: Option<&'a Popup>,
}

impl<'a> Context<'a, '_> {
    /// How the text of part `key` ([`rows::Part::key`]) is selected.
    fn select(&self, key: &str) -> message::Select<'a, app::Message> {
        let (id, room) = (key.to_owned(), self.room.clone());
        message::Select {
            range: (self.selected).and_then(|(bounds, order)| bounds.in_message(*order.get(key)?)),
            focus: (self.focus)
                .filter(|focus| focus.event == key)
                .map(|focus| focus.pos),
            dragging: self.dragging,
            on_gesture: Rc::new(move |gesture| {
                Message::Select(selection::Message::Gesture(id.clone(), gesture)).to(&room)
            }),
            found: (self.found).map_or_else(Vec::new, |find| find.in_message(key)),
        }
    }
}

fn event_view<'a>(
    shown: &'a MessageRow,
    texts: &[Part<'a>],
    context: &Context<'a, '_>,
) -> Element<'a, app::Message> {
    let event = &shown.event;
    let Context {
        theme,
        math,
        params,
        ..
    } = *context;
    let header = texts
        .iter()
        .find(|part| matches!(part.text, rows::Text::Sender(_)));
    let with_header = header.is_some();
    // Where this message's reaction window is open, if it is.
    let picking = context
        .picker
        .filter(|(id, _)| *id == event.id)
        .map(|(_, at)| at);
    // Where this message's menu is open, if it is.
    let menu_at = context
        .menu
        .filter(|(id, _)| *id == event.id)
        .map(|(_, at)| at);
    // Its math is shown typeset, so it can be saved as a picture.
    let mut math_shown = false;
    // Shown as text, which carries the mark of an edit.
    let mut as_text = false;
    let select = || context.select(&event.id);
    let links = || {
        let (id, room) = (event.id.clone(), context.room.clone());
        message::Links {
            open: app::Message::LinkClicked,
            menu: Rc::new(move |url| {
                act(
                    &room,
                    actions::Message::OpenMenu(id.clone(), MenuAt::Link(url)),
                )
            }),
        }
    };
    let body: Element<'a, app::Message> = match &event.content {
        EventContent::Message(m) => {
            let content: Element<'a, app::Message> = match (m.kind, m.document.has_math()) {
                (MessageKind::Text | MessageKind::Notice | MessageKind::Emote, true) => {
                    match math.get(&Math::key(&m.document, params)) {
                        Some(Rendered::Image {
                            handle,
                            width,
                            height,
                            text,
                        }) => {
                            math_shown = true;
                            let select = select();
                            let links = links();
                            selectable_image(
                                image(handle.clone()).width(*width).height(*height),
                                text.clone(),
                            )
                            .selected(select.range)
                            .found(select.found)
                            .dragging(select.dragging)
                            .on_link(links.open)
                            .on_link_menu(links.menu)
                            .on_gesture(select.on_gesture)
                            .into()
                        }
                        _ => {
                            as_text = true;
                            message::view(
                                &m.document,
                                theme,
                                context.own_user,
                                links(),
                                select(),
                                m.edited,
                            )
                        }
                    }
                }
                (MessageKind::Text | MessageKind::Notice | MessageKind::Emote, false) => {
                    as_text = true;
                    message::view(
                        &m.document,
                        theme,
                        context.own_user,
                        links(),
                        select(),
                        m.edited,
                    )
                }
                (kind, _) => match &m.media {
                    Some(file) => media::view(
                        context.room,
                        file,
                        &m.body,
                        context.previews,
                        context.hovered == Some(event.id.as_str()),
                    ),
                    None => text(format!("[{kind:?}] {}", m.body))
                        .style(text::secondary)
                        .into(),
                },
            };
            let mut parts = column![].spacing(4);
            let reply = texts.iter().find_map(|part| match part.text {
                rows::Text::Reply(reply) => Some((part, reply)),
                _ => None,
            });
            if let Some((part, reply)) = reply {
                parts = parts.push(actions::quote(
                    context.room,
                    reply,
                    &part.units(&Shown::default()),
                    context.select(&part.key),
                    rows::shown_id(&part.key),
                ));
            }
            parts = parts.push(content);
            if let Some(cards) = calls::cards(&m.document.links(), context.call_services) {
                parts = parts.push(cards);
            }
            if m.edited && !as_text {
                parts = parts.push(message::edited_line());
            }
            if !m.reactions.is_empty() {
                let picker = (picking == Some(PickerAt::Reactions)).then(|| {
                    actions::picker(
                        context.room,
                        event,
                        &m.reactions,
                        context.frequent_emoji,
                        context.emoji,
                    )
                });
                let listed = (context.reactors)
                    .filter(|(id, _)| *id == event.id)
                    .map(|(_, key)| key);
                parts = parts.push(actions::reactions(
                    context.room,
                    event,
                    &m.reactions,
                    listed,
                    context.avatars,
                    picker,
                ));
            }
            parts.into()
        }
        // A line in its place, its only part besides the header.
        EventContent::UnableToDecrypt | EventContent::Redacted | EventContent::Other { .. } => {
            match texts.iter().find(|part| part.key == event.id) {
                Some(part) => muted(part, 13.0, context).into(),
                None => space().into(),
            }
        }
        // Rendered as summaries in a row of their own (`Row::Summary`).
        EventContent::Membership { .. }
        | EventContent::DisplayNameChange { .. }
        | EventContent::CallLink { .. }
        | EventContent::Topic { .. }
        | EventContent::Pins(_)
        | EventContent::OtherState { .. } => return space().into(),
    };

    let mut item = column![]
        .spacing(2)
        .width(Length::Fill)
        .padding(Padding::ZERO.right(TEXT_MARGIN));
    if let Some(header) = header {
        item = item.push(sender_line(event, header, context));
    }
    let edit = context
        .editing
        .filter(|e| e.target == Editing::Sent(event.key.clone()));
    item = item.push(match edit {
        Some(edit) => edit.view(context.room, context.frequent_emoji, context.popup),
        None => body,
    });
    let message = match &event.content {
        EventContent::Message(m) => Some(m),
        _ => None,
    };
    let in_selection = context.selected.is_some_and(|(bounds, order)| {
        order
            .get(event.id.as_str())
            .is_some_and(|&i| bounds.in_message(i).is_some())
    });
    let menu_items = Some(message_menu::items(
        context.room,
        shown,
        math_shown,
        in_selection,
    ))
    .filter(|items| !items.is_empty());
    let menu_view = |at| match menu_at {
        Some(MenuAt::Link(url)) if at == MenuAt::Cursor => Some(message_menu::view(
            message_menu::link_items(context.room, url),
        )),
        Some(open) if *open == at => menu_items.clone().map(message_menu::view),
        _ => None,
    };
    if context.confirm_delete == Some(event.id.as_str()) {
        item = item.push(actions::confirm_delete(context.room, event));
    }

    // The avatar starts a group; follow-ups show their time there on hover.
    let hovered = context.hovered == Some(event.id.as_str());
    let gutter: Element<'a, app::Message> = if with_header {
        mouse_area(context.avatars.view(
            event.sender.as_str(),
            &display_name(event),
            event.sender_avatar.as_deref(),
            SENDER_AVATAR,
            Shape::Circle,
        ))
        .interaction(mouse::Interaction::Pointer)
        .on_press(sender_info(event))
        .into()
    } else if hovered {
        container(text(time(event)).size(10).style(text::secondary))
            .center_x(SENDER_AVATAR)
            .height(Length::Fill)
            .padding(Padding::ZERO.top(5))
            .into()
    } else {
        space().width(SENDER_AVATAR).into()
    };
    let line = if with_header {
        row![gutter, item].spacing(GUTTER_SPACING)
    } else {
        // At least as tall as the toolbar, with a short line (e.g. of math)
        // centred; the time stays at the top of longer messages.
        let strut = space().width(GUTTER_SPACING).height(TOOLBAR_HEIGHT);
        row![gutter, strut, item].align_y(Alignment::Center)
    };
    let top = if with_header { 4.0 } else { 1.0 };
    let row = container(line)
        .padding(Padding::new(MESSAGE_PADDING).top(top).bottom(1))
        .width(Length::Fill)
        .style(style::message_row(
            hovered,
            context.marked == Some(event.id.as_str()),
        ));
    // Read markers, or while nobody read our message whether it is sent,
    // sit in the free margin at the bottom right, so neither ever changes
    // the row's height.
    let listed = context.readers == Some(event.id.as_str());
    let markers =
        actions::read_markers(context.room, event, context.avatars, listed).or_else(|| {
            actions::send_status(
                context.room,
                event,
                actions::Status {
                    newest: context.newest_own == Some(event.id.as_str()),
                    slow: context.sending.shows_clock(&event.id),
                    open: context.unsent == Some(event.id.as_str()),
                },
            )
        });
    let markers = markers.map(|markers| {
        container(markers)
            .align_right(Length::Fill)
            .align_bottom(Length::Fill)
            .padding(Padding::ZERO.right(MESSAGE_PADDING).bottom(1.0))
            .into()
    });
    // Actions float over the top right of a hovered message, above the
    // markers of a short one.
    // Also while its reaction window or menu is open.
    let toolbar_shown =
        (hovered || picking == Some(PickerAt::Toolbar) || menu_at == Some(&MenuAt::Toolbar))
            && (actions::has_tools(&shown.actions) || menu_items.is_some())
            && edit.is_none();
    let toolbar = toolbar_shown.then(|| {
        let target = message.and_then(|m| {
            let preview = first_line(&m.document.to_plain_text());
            Target::of(event, display_name(event), preview)
        });
        let edit_text = message.map(|m| m.body.clone());
        let picker = message
            .filter(|_| picking == Some(PickerAt::Toolbar))
            .map(|m| {
                actions::picker(
                    context.room,
                    event,
                    &m.reactions,
                    context.frequent_emoji,
                    context.emoji,
                )
            });
        let menu = menu_items
            .is_some()
            .then(|| actions::Menu(menu_view(MenuAt::Toolbar)));
        container(actions::toolbar(
            context.room,
            shown,
            target,
            edit_text,
            picker,
            menu,
        ))
        .align_right(Length::Fill)
        .padding(Padding::ZERO.top(1.0).right(8.0))
        .into()
    });
    let row: Element<'a, app::Message> = if markers.is_none() && toolbar.is_none() {
        row.into()
    } else {
        stack![row]
            .extend(markers.into_iter().chain(toolbar))
            .into()
    };
    // Right-click opens the menu where the cursor is.
    let row: Element<'a, app::Message> = if menu_items.is_some() {
        context_menu(
            row,
            menu_view(MenuAt::Cursor),
            act(
                context.room,
                actions::Message::OpenMenu(event.id.clone(), MenuAt::Cursor),
            ),
        )
        .on_dismiss(act(context.room, actions::Message::CloseMenu))
        .into()
    } else {
        row
    };
    let row = mouse_area(row)
        .on_enter(Message::Hover(event.id.clone(), true).to(context.room))
        .on_exit(Message::Hover(event.id.clone(), false).to(context.room));
    // Found by `on_screen`.
    let row = container(row).id(row_id(&event.id));
    if with_header {
        // Space between groups, outside the hover background.
        column![space().height(6), row].into()
    } else {
        row.into()
    }
}

/// The menu and emoji panel open in `field`, if either is.
pub fn in_field(
    popup: Option<&Popup>,
    field: Field,
) -> (Option<&edit_menu::Menu>, Option<&emoji::Panel>) {
    match popup {
        Some(Popup::FieldMenu(f, menu)) if *f == field => (Some(menu), None),
        Some(Popup::FieldEmoji(f, panel)) if *f == field => (None, Some(panel)),
        _ => (None, None),
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

/// The sender's name, in their colour if names are coloured, and the time
/// above the first message of a group (`part`); a click on either opens
/// the sender's card.
fn sender_line<'a>(
    event: &Event,
    part: &Part<'_>,
    context: &Context<'a, '_>,
) -> Element<'a, app::Message> {
    let units = part.units(&Shown::default());
    let select = context.select(&part.key);
    let open = sender_info(event);
    let mut name = span(units[0].text.clone()).font(fonts::SEMIBOLD);
    if context.colored_names {
        name = name.color(avatar::name_color(event.sender.as_str(), context.theme));
    }
    let time = span(units[1].text.clone());
    row![
        select
            .apply(selectable(vec![name], 0), 0, units[0].text.len())
            .on_click(open.clone()),
        select
            .apply(
                selectable(vec![time], 1).size(11).style(text::secondary),
                1,
                units[1].text.len(),
            )
            .on_click(open),
    ]
    .spacing(8)
    .align_y(Alignment::End)
    .into()
}

/// The card of `user`, mentioned in `rows`: their name and picture
/// among `members` if loaded, else the name the newest mention shows.
fn mentioned(user: OwnedUserId, members: Option<&[Member]>, rows: &[Row]) -> info::Target {
    let member = members.and_then(|members| members.iter().find(|m| m.id == user));
    let name = member.and_then(|m| m.name.clone()).or_else(|| {
        (rows.iter().rev().filter_map(Row::event)).find_map(|event| match &event.content {
            EventContent::Message(m) => m.document.mention_name(user.as_str()).map(str::to_owned),
            _ => None,
        })
    });
    let avatar = member.and_then(|m| m.avatar.clone());
    info::sender(user, name, avatar)
}

/// Opens the sender's info card.
fn sender_info(event: &Event) -> app::Message {
    app::Message::ShowInfo(info::sender(
        event.sender.clone(),
        event.sender_name.clone(),
        event.sender_avatar.clone(),
    ))
}

/// A muted line in the text column, e.g. "Loading…".
fn notice<'a>(content: impl text::IntoFragment<'a>) -> Element<'a, app::Message> {
    container(text(content).size(12).style(text::secondary))
        .padding(Padding::new(4.0).left(TEXT_INDENT))
        .into()
}

/// The text of a part of one line (see [`rows::Text::Line`]), muted.
fn muted<'a>(
    part: &Part<'_>,
    size: f32,
    context: &Context<'a, '_>,
) -> selectable::Selectable<'a, app::Message> {
    let line = part.units(&Shown::default()).remove(0).text;
    let len = line.len();
    let unit = selectable(vec![span(line)], 0)
        .size(size)
        .style(text::secondary);
    context.select(&part.key).apply(unit, 0, len)
}

/// A row of one muted line in the text column, e.g. for state events.
fn line<'a>(parts: &[Part<'_>], context: &Context<'a, '_>) -> Element<'a, app::Message> {
    let Some(part) = parts.first() else {
        return space().into();
    };
    container(muted(part, 12.0, context))
        .padding(Padding::new(4.0).left(TEXT_INDENT))
        .id(row_id(&part.key))
        .into()
}

/// The date between two days' messages, centred on a line.
fn day_divider<'a>(parts: &[Part<'_>], context: &Context<'a, '_>) -> Element<'a, app::Message> {
    let Some(part) = parts.first() else {
        return space().into();
    };
    let divider = row![
        rule::horizontal(1).style(style::separator),
        muted(part, 12.0, context),
        rule::horizontal(1).style(style::separator),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .padding([10.0, MESSAGE_PADDING]);
    container(divider).id(row_id(&part.key)).into()
}

/// The unread messages below: "New" on a line, as a day between days.
fn read_marker<'a>() -> Element<'a, app::Message> {
    row![
        rule::horizontal(1).style(style::separator),
        text("New").size(12).style(text::secondary),
        rule::horizontal(1).style(style::separator),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .padding([10.0, MESSAGE_PADDING])
    .into()
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
            id: nils.clone(),
            name: Some("Nils Wolf".into()),
            avatar: Some("mxc://x/nils".into()),
            role: mtrx_core::Role::Member,
            level: Some(0),
            invited: false,
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
