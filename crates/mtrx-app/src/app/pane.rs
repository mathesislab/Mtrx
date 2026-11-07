//! The room pane, right of the room list: a room opening, open or invited
//! to, and its member list.

use std::sync::Arc;

use iced::{
    Element, Length, Task,
    widget::{button, column, container, row, text},
};
use mtrx_core::{
    Connection, Membership, OwnedEventId, OwnedRoomId, OwnedUserId, RoomLists, RoomOrAliasId,
    RoomSummary, RoomTimeline, Session,
};

use super::{Cx, Popup, Shared, account};
use crate::{
    app, fonts, info, members,
    room::{self, Load, Room},
    style, task,
};

#[derive(Debug, Clone)]
pub enum Message {
    Open(OwnedRoomId),
    /// Its timeline, or why it did not open.
    Opened(OwnedRoomId, Result<Shared<RoomTimeline>, String>),
    /// Accept (`true`) or decline an invite.
    Answer(OwnedRoomId, bool),
    Answered(OwnedRoomId, bool, Result<(), String>),
    ToggleMembers,
    /// Open the room with its members shown (where Invite is).
    ShowMembers(OwnedRoomId),
    MemberMenu(members::MenuKey),
    /// Give a member a power level, without asking.
    SetLevel(OwnedRoomId, OwnedUserId, i64),
    /// A room's members were changed by us, or not.
    MembersChanged(OwnedRoomId, Result<(), String>),
    /// Reload the open room's timeline (⌘R / Ctrl+R); while offline, try
    /// the server now.
    Reload,
    /// Show a message, opening its room ("Go to message").
    GoTo(OwnedRoomId, OwnedEventId),
}

/// The room pane and its member list.
#[derive(Default)]
pub struct Pane {
    pub shown: Shown,
    /// The member list is shown beside the open room, whichever it is.
    pub show_members: bool,
    /// A message to show once its room opens ("Go to message").
    pub go_to: Option<(OwnedRoomId, OwnedEventId)>,
}

/// What the room pane shows.
// One, in the boxed [`super::Main`]: boxing the room would only add
// indirection.
#[allow(clippy::large_enum_variant)]
#[derive(Default)]
pub enum Shown {
    #[default]
    Empty,
    /// Its timeline loads.
    Opening(OwnedRoomId),
    /// Accept and Decline instead of a timeline; `answering` while it is answered.
    Invite {
        id: OwnedRoomId,
        answering: bool,
    },
    Open(Room),
}

impl Pane {
    /// The open room.
    pub fn room(&self) -> Option<&Room> {
        match &self.shown {
            Shown::Open(room) => Some(room),
            _ => None,
        }
    }

    pub fn room_mut(&mut self) -> Option<&mut Room> {
        match &mut self.shown {
            Shown::Open(room) => Some(room),
            _ => None,
        }
    }

    /// Room `id`, if it is the one open.
    pub fn open(&self, id: &OwnedRoomId) -> Option<&Room> {
        self.room().filter(|room| room.id == *id)
    }

    pub fn open_mut(&mut self, id: &OwnedRoomId) -> Option<&mut Room> {
        self.room_mut().filter(|room| room.id == *id)
    }

    /// The room shown: opening, invited to or open.
    pub fn id(&self) -> Option<&OwnedRoomId> {
        self.shown.id()
    }

    /// The room to show again next time: opening or open.
    pub fn remembered(&self) -> Option<&OwnedRoomId> {
        match &self.shown {
            Shown::Opening(id) => Some(id),
            Shown::Open(room) => Some(&room.id),
            Shown::Empty | Shown::Invite { .. } => None,
        }
    }

    /// Show `shown`: the popups of the room shown before close, and its
    /// draft is kept now.
    pub fn set(&mut self, shown: Shown, popup: &mut Option<Popup>) -> Task<app::Message> {
        let left = std::mem::replace(&mut self.shown, shown);
        if let Some(room) = left.id() {
            popup.take_if(|popup| popup.of_room(room));
        }
        match left {
            Shown::Open(room) => room::keep_now(room.drafts.clone(), room.draft()),
            _ => Task::none(),
        }
    }

    /// A message for room `id`: to it while it is open, else what still
    /// concerns it.
    pub fn room_message(
        &mut self,
        id: OwnedRoomId,
        message: room::Message,
        cx: &mut Cx<'_>,
    ) -> Task<app::Message> {
        match self.open_mut(&id) {
            Some(room) => room.update(message, cx),
            None => room::left(cx.session, id, message),
        }
    }

    /// A message for the room shown, if one is open.
    pub fn shown_message(&mut self, message: room::Message, cx: &mut Cx<'_>) -> Task<app::Message> {
        match self.room_mut() {
            Some(room) => room.update(message, cx),
            None => Task::none(),
        }
    }

    /// Make the open room's pinned messages' timeline, once it has pins.
    pub fn load_pinned(&mut self, session: &Arc<Session>, lists: &RoomLists) -> Task<app::Message> {
        match self.room_mut() {
            Some(room) => room.load_pinned(session, lists),
            None => Task::none(),
        }
    }

    pub fn update(&mut self, message: Message, cx: &mut Cx<'_>) -> Task<app::Message> {
        match message {
            Message::Open(id) => self.open_room(id, cx),
            Message::Opened(id, result) => {
                if !matches!(&self.shown, Shown::Opening(opening) if *opening == id) {
                    // Another room or an invite was chosen meanwhile.
                    return Task::none();
                }
                let opened = result.and_then(|timeline| {
                    let handle = cx.session.room(&id).map_err(|e| e.to_string())?;
                    Ok((handle, timeline))
                });
                match opened {
                    Ok((handle, Shared(timeline))) => {
                        // It opened after all.
                        cx.notices.solved(&id);
                        let drafts = cx.session.draft_keeper(&id);
                        let mut room = Room::new(handle, timeline, drafts);
                        let paginate = room.fill();
                        let read = room.mark_read();
                        // The member list stays open from the last room.
                        let members = if self.show_members {
                            room.load_members()
                        } else {
                            Task::none()
                        };
                        let draft = room.load_draft();
                        let call_service = room.load_call_service(cx.session);
                        let opening = self.set(Shown::Open(room), cx.popup);
                        let go_to = match self.go_to.take() {
                            Some((room, event)) if room == id => {
                                Task::done(room::Message::GoTo(event).to(&id))
                            }
                            _ => Task::none(),
                        };
                        Task::batch([
                            opening,
                            paginate,
                            read,
                            members,
                            draft,
                            go_to,
                            call_service,
                            self.load_pinned(cx.session, cx.lists),
                        ])
                    }
                    Err(e) => {
                        let opening = self.set(Shown::Empty, cx.popup);
                        cx.notices
                            .report(format!("Could not open the room: {e}"), Some(id));
                        opening
                    }
                }
            }
            Message::Answer(id, accept) => {
                let answering = self.set(
                    Shown::Invite {
                        id: id.clone(),
                        answering: true,
                    },
                    cx.popup,
                );
                let session = cx.session.clone();
                let answer = task::request(
                    {
                        let id = id.clone();
                        async move {
                            if accept {
                                session
                                    .join(<&RoomOrAliasId>::from(&*id), &[])
                                    .await
                                    .map(|_| ())
                            } else {
                                session.room(&id)?.leave().await
                            }
                        }
                    },
                    move |result| to(Message::Answered(id, accept, result)),
                );
                Task::batch([answering, answer])
            }
            Message::Answered(id, accept, result) => {
                let answered = if matches!(&self.shown, Shown::Invite { id: invite, .. } if *invite == id)
                {
                    self.set(Shown::Empty, cx.popup)
                } else {
                    Task::none()
                };
                let task = match result {
                    Ok(()) if accept => self.open_room(id, cx),
                    Ok(()) => Task::none(),
                    Err(e) => {
                        let answer = if accept { "accept" } else { "decline" };
                        cx.notices
                            .report(format!("Could not {answer} the invite: {e}"), None);
                        Task::none()
                    }
                };
                Task::batch([answered, task])
            }
            Message::ToggleMembers => {
                self.show_members = !self.show_members;
                let show = self.show_members;
                // Opening it shows the members anew.
                match self.room_mut() {
                    Some(room) if show && !room.tasks.running(&Load::Members) => {
                        room.load_members()
                    }
                    _ => Task::none(),
                }
            }
            Message::ShowMembers(id) => {
                self.show_members = true;
                cx.close_card();
                match self.open_mut(&id) {
                    Some(room) if room.tasks.running(&Load::Members) => Task::none(),
                    Some(room) => room.load_members(),
                    None => Task::done(to(Message::Open(id))),
                }
            }
            Message::MemberMenu(menu) => {
                *cx.popup = Some(Popup::MemberMenu(menu));
                Task::none()
            }
            Message::SetLevel(room, user, level) => {
                cx.popup.take_if(|p| matches!(p, Popup::MemberMenu(_)));
                let session = cx.session.clone();
                task::request(
                    {
                        let room = room.clone();
                        async move { session.room(&room)?.set_level(&user, level).await }
                    },
                    move |result| to(Message::MembersChanged(room, result)),
                )
            }
            // Shown anew where they are listed: beside the open room, and
            // in its space's overview.
            Message::MembersChanged(room, result) => {
                if let Err(e) = &result {
                    cx.notices
                        .report(format!("Could not change the members: {e}"), None);
                }
                let show = self.show_members;
                let beside = match self.open_mut(&room) {
                    Some(open) if show => open.load_members(),
                    _ => Task::none(),
                };
                let overview = Task::done(app::Message::Card(info::Message::MembersChanged(room)));
                Task::batch([beside, overview])
            }
            Message::GoTo(id, event) => {
                if self.open(&id).is_some() {
                    return Task::done(room::Message::GoTo(event).to(&id));
                }
                let open = self.open_room(id.clone(), cx);
                self.go_to = Some((id, event));
                open
            }
            Message::Reload => {
                match cx.connection {
                    Connection::Offline => {
                        return Task::done(app::Message::Account(account::Message::Reconnect));
                    }
                    // Its bar offers signing in again.
                    Connection::SignedOut => return Task::none(),
                    Connection::Online => {}
                }
                let Some(room) = self.room_mut() else {
                    return Task::none();
                };
                let (session, id) = (cx.session.clone(), room.id.clone());
                room.tasks.load(
                    Load::Reload,
                    {
                        let id = id.clone();
                        async move { session.timeline(&id).await.map(|t| Shared(Arc::new(t))) }
                    },
                    move |result| room::Message::Reloaded(result).to(&id),
                )
            }
        }
    }

    /// Open room `id`, chosen: what floats over the window closes.
    fn open_room(&mut self, id: OwnedRoomId, cx: &mut Cx<'_>) -> Task<app::Message> {
        cx.cover();
        cx.close_card();
        self.go_to.take_if(|(room, _)| *room != id);
        if self.open(&id).is_some() {
            return Task::none();
        }
        // An invite shows Accept and Decline instead of a timeline.
        let invited = |r: &RoomSummary| r.membership == Membership::Invited;
        if cx.lists.room(&id).is_some_and(invited) {
            return self.set(
                Shown::Invite {
                    id,
                    answering: false,
                },
                cx.popup,
            );
        }
        let left = self.set(Shown::Opening(id.clone()), cx.popup);
        let session = cx.session.clone();
        let open = task::request(
            {
                let id = id.clone();
                async move { session.timeline(&id).await.map(|t| Shared(Arc::new(t))) }
            },
            move |result| to(Message::Opened(id, result)),
        );
        Task::batch([left, open])
    }
}

impl Shown {
    /// The room shown: opening, invited to or open.
    fn id(&self) -> Option<&OwnedRoomId> {
        match self {
            Shown::Empty => None,
            Shown::Opening(id) | Shown::Invite { id, .. } => Some(id),
            Shown::Open(room) => Some(&room.id),
        }
    }
}

/// An invite, with Accept and Decline.
pub fn invite_view<'a>(
    summary: Option<&'a RoomSummary>,
    id: &'a OwnedRoomId,
    answering: bool,
) -> Element<'a, app::Message> {
    let name = summary.map_or(id.as_str(), |s| s.name.as_str());
    let actions: Element<'a, app::Message> = if answering {
        text("Answering the invite…").style(text::secondary).into()
    } else {
        row![
            button(text("Decline"))
                .style(style::secondary_button)
                .on_press(to(Message::Answer(id.clone(), false))),
            button(text("Accept"))
                .style(style::primary_button)
                .on_press(to(Message::Answer(id.clone(), true))),
        ]
        .spacing(8)
        .into()
    };
    container(
        column![
            text(format!("You are invited to {name}"))
                .size(18)
                .font(fonts::SEMIBOLD),
            text("Its messages show once you accept.").style(text::secondary),
            actions,
        ]
        .spacing(12)
        .align_x(iced::Alignment::Center),
    )
    .center(Length::Fill)
    .into()
}

/// `message` for the room pane, as the app gets it.
pub fn to(message: Message) -> app::Message {
    app::Message::Pane(message)
}
