//! What an open room does with its messages.

use std::{path::PathBuf, sync::Arc, time::Duration};

use iced::{Task, widget::text_editor};
use mtrx_core::{
    Connection, Draft, DraftKeeper, OwnedEventId, OwnedRoomId, RoomLists, Session, UserId,
};

use super::{Load, Message, Older, Room, SENDING_CLOCK, Scroll, TIMELINE, scroll, scroll_to_row};
use crate::{
    actions::{self, Editing},
    app::{self, Cx, Shared, people},
    calls,
    clipboard::{self, Copied},
    emoji, find, media,
    message_field::{self, Field},
    pins,
    selection::{self, Selection},
    send_later, task, topic,
};

impl Room {
    /// `message` for this room, and what it then asks of the app. The edit
    /// field's popups close with the edit, and the draft's keeper hears of
    /// each change.
    pub fn update(&mut self, message: Message, cx: &mut Cx<'_>) -> Task<app::Message> {
        let (editing, draft) = (self.editing_target(), self.draft());
        let task = self.handle(message, cx);
        if self.editing_target() != editing {
            cx.popup.take_if(|popup| popup.in_field(Field::Edit));
        }
        let now = self.draft();
        if now != draft {
            self.drafts.changed(now);
        }
        task
    }

    /// What is edited in place.
    fn editing_target(&self) -> Option<Editing> {
        Some(self.editing.as_ref()?.target.clone())
    }

    fn handle(&mut self, message: Message, cx: &mut Cx<'_>) -> Task<app::Message> {
        let id = self.id.clone();
        match message {
            Message::Timeline(rows) => {
                let grew = rows.len() != self.rows.len();
                self.rows = rows;
                self.find_again();
                let read = if grew { self.mark_read() } else { Task::none() };
                let clocks = self.sending_changed().into_iter().map(|row| {
                    let id = id.clone();
                    Task::perform(tokio::time::sleep(SENDING_CLOCK), move |()| {
                        Message::SendingSlow(row.clone()).to(&id)
                    })
                });
                Task::batch([Task::batch(clocks), read, self.fill()])
            }
            Message::Scrolled(viewport) => match self.older(&viewport) {
                Older::None => Task::none(),
                Older::ShowMore => {
                    self.show_more();
                    Task::none()
                }
                Older::Paginate => self.paginate(),
            },
            Message::Paginated(result) => match result {
                Ok(()) => {
                    self.show_more();
                    self.fill()
                }
                // Loaded once the server is back.
                Err(_) if cx.connection != Connection::Online => Task::none(),
                Err(e) => {
                    self.error = Some(e);
                    Task::none()
                }
            },
            Message::Field(field, message) => self.field(field, message, cx),
            Message::TypeInComposer(text) => {
                // Typed, so that it undoes with what is typed next.
                let typed: Vec<_> = text
                    .chars()
                    .map(|c| {
                        let edit = text_editor::Action::Edit(text_editor::Edit::Insert(c));
                        let message = message_field::Message::Action(edit);
                        self.handle(Message::Field(Field::Composer, message), cx)
                    })
                    .collect();
                Task::batch(
                    typed
                        .into_iter()
                        .chain([iced::widget::operation::focus(message_field::COMPOSER)]),
                )
            }
            Message::MembersLoaded(result) => {
                self.members_loaded(result);
                Task::none()
            }
            Message::Send => {
                let text = self.composer.text();
                if text.trim().is_empty() && self.attachments.is_empty() {
                    return Task::none();
                }
                let compose = std::mem::take(&mut self.compose);
                self.composer.set_text("");
                let files = std::mem::take(&mut self.attachments)
                    .into_iter()
                    .map(|attached| attached.file)
                    .collect();
                self.error = None;
                let timeline = self.timeline.clone();
                let used = emoji::used(&text);
                // Queue only: the local echo shows the send state. Queued
                // first, so it shows before the server hears of anything.
                let send = task::request(
                    async move {
                        let sent = timeline
                            .send(text.trim_end(), files, compose.reply_to().map(|id| &**id))
                            .await
                            .map(|_| ());
                        let _ = timeline.typing(false).await;
                        sent
                    },
                    move |result| Message::Sent(result).to(&id),
                );
                Task::batch([send, people::used(used)])
            }
            Message::SendingSlow(row) => {
                self.sending_slow(&row);
                Task::none()
            }
            Message::Sent(result) => {
                if let Err(e) = result {
                    self.error = Some(e);
                }
                Task::none()
            }
            Message::Later(message) => send_later::update(self, message, cx),
            Message::Act(message) => actions::update(self, message, cx),
            // The timeline is anchored at the bottom, so its offset counts
            // upwards.
            Message::ScrollTimeline(dy) => iced::widget::operation::scroll_by(
                TIMELINE,
                iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: -dy },
            ),
            Message::ScrollKey(key) => {
                if key == Scroll::Oldest {
                    self.show_all();
                }
                scroll(key)
            }
            Message::Select(message) => {
                match message {
                    // What reply previews show first, as they are cut to fit.
                    selection::Message::Copy => {
                        return self.read_shown().map(move |shown| {
                            Message::Select(selection::Message::CopyAsShown(shown)).to(&id)
                        });
                    }
                    selection::Message::CopyAsShown(shown) => {
                        return (self.selected(&shown))
                            .map_or_else(Task::none, |copied| copy(&id, copied));
                    }
                    selection::Message::SelectAll => {
                        return self.on_screen().and_then(move |(first, last)| {
                            Task::done(
                                Message::Select(selection::Message::All(first, last)).to(&id),
                            )
                        });
                    }
                    selection::Message::Extend(step) => {
                        self.extend(step);
                        return Task::none();
                    }
                    selection::Message::ExtendLine(up) => return self.extend_line(up),
                    selection::Message::ExtendToLineEdge(forward) => {
                        return self.extend_to_line_edge(forward);
                    }
                    selection::Message::ExtendToScreenEdge(up) => {
                        return self.extend_to_screen_edge(up);
                    }
                    selection::Message::ExtendToPart(event, start) => {
                        self.extend_to_part(event, start);
                        return Task::none();
                    }
                    selection::Message::LineMoved(from, up, offset) => {
                        self.line_moved(from, up, offset);
                        return Task::none();
                    }
                    _ => {}
                }
                Selection::update(&mut self.selection, message);
                Task::none()
            }
            Message::Find(message) => find::update(self, message),
            Message::Typing(names) => {
                self.typing = names;
                Task::none()
            }
            Message::Calls(message) => calls::update(self, message, cx),
            Message::Topic(message) => {
                // From the call menu too.
                if let topic::Message::Edit = message {
                    cx.popup
                        .take_if(|popup| matches!(popup, app::Popup::Calls(_)));
                }
                let listed = (cx.lists.room(&id)).and_then(|r| r.topic.as_deref());
                topic::update(
                    cx.session,
                    &mut self.topic,
                    id.clone(),
                    listed,
                    message,
                    move |message| Message::Topic(message).to(&id),
                )
            }
            Message::Uploads(uploads) => {
                self.set_uploads(uploads);
                Task::none()
            }
            Message::DismissError => {
                self.error = None;
                Task::none()
            }
            Message::StopSeeking => {
                self.tasks.cancel(&Load::Seek);
                Task::none()
            }
            Message::Pins(message) => {
                if let pins::Message::GoTo(event) = message {
                    self.topic.open = false;
                    return Task::done(Message::GoTo(event).to(&id));
                }
                self.pins.update(message);
                Task::none()
            }
            Message::Reloaded(result) => match result {
                Ok(Shared(timeline)) => {
                    self.reload(timeline);
                    // Its rows are new.
                    cx.popup.take_if(|popup| popup.of_room(&id));
                    let call_service = self.load_call_service(cx.session);
                    Task::batch([self.fill(), self.mark_read(), call_service])
                }
                Err(e) => {
                    self.error = Some(e);
                    Task::none()
                }
            },
            Message::DraftLoaded(draft) => self.restore_draft(draft),
            Message::Online => self.fill(),
            Message::Scheduled(scheduled) => {
                self.scheduled_changed(&scheduled);
                Task::none()
            }
            Message::ReplyFound(target) => {
                self.reply_found(target);
                Task::none()
            }
            Message::Hover(row, entered) => {
                self.hover(row, entered);
                Task::none()
            }
            Message::GoTo(event) => {
                self.error = None;
                match self.reveal(&event) {
                    Some(row) => self.go_to(row),
                    // A new one replaces the one under way.
                    None => self.seek(event),
                }
            }
            Message::Sought(result) => {
                let row = match result {
                    Ok(row) => row,
                    Err(e) => {
                        self.error = Some(e);
                        return Task::none();
                    }
                };
                // The core's rows have it, the room's may not yet.
                let caught_up = if self.reveal_row(&row) {
                    Task::none()
                } else {
                    let rows = Arc::new(self.timeline.current_rows());
                    self.handle(Message::Timeline(rows), cx)
                };
                let go = if self.reveal_row(&row) {
                    self.go_to(row)
                } else {
                    Task::none()
                };
                Task::batch([caught_up, go])
            }
            Message::Unmark(row) => {
                if self.marked.as_ref() == Some(&row) {
                    self.marked = None;
                }
                Task::none()
            }
            Message::Attach => {
                Task::future(media::pick_files()).then(move |paths| attach(id.clone(), paths))
            }
            Message::FileDropped(path) => attach(id, vec![path]),
            Message::Attached(attached) => {
                for attached in attached {
                    match attached {
                        Ok(attached) => self.attachments.push(attached),
                        Err(e) => self.error = Some(e),
                    }
                }
                Task::none()
            }
            Message::Unattach(index) => {
                if index < self.attachments.len() {
                    self.attachments.remove(index);
                }
                Task::none()
            }
        }
    }

    /// `message` to `field`, and what it then asks of the room.
    fn field(
        &mut self,
        field: Field,
        message: message_field::Message,
        cx: &mut Cx<'_>,
    ) -> Task<app::Message> {
        use message_field::Event;
        let id = self.id.clone();
        let Some(target) = self.field_mut(field) else {
            return Task::none();
        };
        let (task, event) = target.update(message, cx.popup);
        let task = task.map({
            let id = id.clone();
            move |message| Message::Field(field, message).to(&id)
        });
        // Mentions need the members: loaded on the first `@`.
        let members = if self.wants_members() {
            self.load_members()
        } else {
            Task::none()
        };
        let asked = match (field, event) {
            (_, None) => Task::none(),
            (Field::Composer, Some(Event::Edited)) => {
                let timeline = self.timeline.clone();
                let typing = !self.composer.is_empty();
                task::background(
                    "say we are typing",
                    async move { timeline.typing(typing).await },
                )
            }
            (Field::Edit, Some(Event::Edited)) => Task::none(),
            (Field::Composer, Some(Event::Submit)) => self.handle(Message::Send, cx),
            (Field::Edit, Some(Event::Submit)) => {
                actions::update(self, actions::Message::SaveEdit, cx)
            }
            (Field::Composer, Some(Event::Cancel)) => {
                actions::update(self, actions::Message::CancelCompose, cx)
            }
            (Field::Edit, Some(Event::Cancel)) => {
                actions::update(self, actions::Message::CancelEdit, cx)
            }
            (_, Some(Event::EditLast)) => actions::update(self, actions::Message::EditLast, cx),
            (_, Some(Event::Mention(message))) => {
                let everyone = mention_room(cx.lists, &id);
                self.mention(field, message, cx.own_user, everyone)
            }
            (_, Some(Event::Attach(paths))) => attach(id, paths),
            (_, Some(Event::AttachImage(file))) => {
                Task::perform(media::attached(file), move |attached| {
                    Message::Attached(vec![attached]).to(&id)
                })
            }
            (_, Some(Event::EmojiShown)) => people::load_frequent_emoji(cx.session),
        };
        Task::batch([task, members, asked])
    }

    /// Esc in the room, if something in it takes it: the mention offered,
    /// the find bar, the selection, the reply (from the composer its own Esc
    /// does it first), the message edited in place.
    pub fn escape(&mut self, own_user: &UserId, lists: &RoomLists) -> Option<Task<app::Message>> {
        if self.completing(own_user, mention_room(lists, &self.id)) {
            self.close_completion();
        } else if self.find.is_some() {
            self.find = None;
        } else if self.selection.is_some() {
            self.selection = None;
        } else if self.compose.cancellable() {
            return Some(Task::done(app::Message::Shown(Message::Act(
                actions::Message::CancelCompose,
            ))));
        } else if self.editing.is_some() {
            self.editing = None;
        } else {
            return None;
        }
        Some(Task::none())
    }

    /// Load the room's members, for its member list and mentions.
    pub fn load_members(&mut self) -> Task<app::Message> {
        let (handle, id) = (self.handle.clone(), self.id.clone());
        self.tasks.load(
            Load::Members,
            async move { handle.members().await },
            move |result| Message::MembersLoaded(result).to(&id),
        )
    }

    /// The call service of the room's space.
    pub fn load_call_service(&mut self, session: &Arc<Session>) -> Task<app::Message> {
        let (session, id) = (session.clone(), self.id.clone());
        self.tasks.run(
            Load::CallService,
            Task::perform(
                {
                    let id = id.clone();
                    async move { session.room(&id).ok()?.call_service().await.ok().flatten() }
                },
                move |service| Message::Calls(calls::Message::SpaceService(service)).to(&id),
            ),
        )
    }

    /// Make the pinned messages' timeline, once the room has pins.
    pub fn load_pinned(&mut self, session: &Arc<Session>, lists: &RoomLists) -> Task<app::Message> {
        let pinned = lists.room(&self.id).map_or(&[][..], |r| &r.pinned);
        if !self.pins.request(pinned) {
            return Task::none();
        }
        let (session, id, reply) = (session.clone(), self.id.clone(), self.id.clone());
        self.tasks.load(
            Load::Pinned,
            async move { (session.pinned_timeline(&id).await).map(|t| Shared(Arc::new(t))) },
            move |result| Message::Pins(pins::Message::Loaded(result)).to(&reply),
        )
    }

    /// The draft kept for the room, to write on.
    pub fn load_draft(&mut self) -> Task<app::Message> {
        let (keeper, id) = (self.drafts.clone(), self.id.clone());
        self.tasks.run(
            Load::Draft,
            Task::perform(async move { keeper.restore().await }, move |draft| {
                Message::DraftLoaded(draft).to(&id)
            }),
        )
    }

    /// Load older events while too few are loaded to fill the view.
    pub fn fill(&mut self) -> Task<app::Message> {
        if self.wants_older() {
            self.paginate()
        } else {
            Task::none()
        }
    }

    /// Load older events. Whether the start was reached shows as
    /// [`mtrx_core::Row::Start`] in the rows.
    fn paginate(&mut self) -> Task<app::Message> {
        if self.tasks.running(&Load::Paginate) || self.at_start() {
            return Task::none();
        }
        let (id, timeline) = (self.id.clone(), self.timeline.clone());
        self.tasks.load(
            Load::Paginate,
            async move { timeline.paginate_back(30).await.map(|_| ()) },
            move |result| Message::Paginated(result).to(&id),
        )
    }

    pub fn mark_read(&self) -> Task<app::Message> {
        let timeline = self.timeline.clone();
        task::background(
            "mark the room read",
            async move { timeline.mark_read().await },
        )
    }

    /// Scroll to the message row `id` (rendered) and mark it for a moment.
    fn go_to(&mut self, id: String) -> Task<app::Message> {
        self.marked = Some(id.clone());
        let (unmark, room) = (id.clone(), self.id.clone());
        Task::batch([
            scroll_to_row(&id),
            Task::perform(tokio::time::sleep(Duration::from_secs(2)), move |()| {
                Message::Unmark(unmark.clone()).to(&room)
            }),
        ])
    }

    /// Load older events until message `event` is there, to go to it.
    fn seek(&mut self, event: OwnedEventId) -> Task<app::Message> {
        let (id, timeline) = (self.id.clone(), self.timeline.clone());
        self.tasks.load(
            Load::Seek,
            async move { timeline.seek(&event).await },
            move |result| Message::Sought(result).to(&id),
        )
    }
}

/// A message for room `id`, which is not open: the result of a request it
/// made (its loads ended with it). Those that still concern it act on it;
/// the others are dropped.
pub fn left(session: &Session, id: OwnedRoomId, message: Message) -> Task<app::Message> {
    match message {
        // The draft kept on leaving holds the text scheduled since.
        Message::Later(send_later::Message::Scheduled(Ok(()))) => {
            let keeper = session.draft_keeper(&id);
            Task::future(async move {
                keeper.restore().await;
                keeper.changed(Draft::default());
                keeper.flush().await;
            })
            .discard()
        }
        _ => Task::none(),
    }
}

/// Keep `draft`, a room's last, at once: the room was left, or the window
/// closes.
pub fn keep_now(keeper: Arc<DraftKeeper>, draft: Draft) -> Task<app::Message> {
    keeper.changed(draft);
    Task::future(async move { keeper.flush().await }).discard()
}

/// Whether we may mention everyone in `room` with `@room`.
pub fn mention_room(lists: &RoomLists, room: &OwnedRoomId) -> bool {
    lists.room(room).is_some_and(|r| r.rights.mention_room)
}

/// Put `copied` on the clipboard; a failure shows in the room.
pub fn copy(room: &OwnedRoomId, copied: Copied) -> Task<app::Message> {
    let room = room.clone();
    Task::perform(clipboard::write(copied), move |result| {
        actions::act(&room, actions::Message::Done(result))
    })
}

/// Read files to attach to the message being written in `room`.
fn attach(room: OwnedRoomId, paths: Vec<PathBuf>) -> Task<app::Message> {
    if paths.is_empty() {
        return Task::none();
    }
    Task::perform(media::read_attached(paths), move |attached| {
        Message::Attached(attached).to(&room)
    })
}
