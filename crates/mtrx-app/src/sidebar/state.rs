//! What the space bar and the room list keep and do.

use std::sync::Arc;

use iced::Task;
use mtrx_core::{DirectoryRoom, OwnedRoomId, OwnedServerName, RoomListColours, Session, SpaceKey};

use super::{Go, OutOfView, SEARCH, SpaceMenu, scroll_to};
use crate::{
    app::{self, Cx, Popup, pane},
    arrange::{self, Arrange},
    directory, task, topic,
};

/// The space bar's and the room list's.
#[derive(Debug, Clone)]
pub enum Message {
    SelectSpace(Option<OwnedRoomId>),
    /// The room search text.
    Filter(String),
    /// Open the first room matching the search.
    OpenFirstMatch,
    /// Open the room above or below in the room list, or one by its row.
    GoTo(Go),
    FocusSearch,
    /// A space's rooms as its server listed them, or why not.
    SpaceRoomsLoaded(OwnedRoomId, Result<Vec<DirectoryRoom>, String>),
    /// Join a room the search found in our spaces, or ask to (`true`).
    JoinFound(OwnedRoomId, Vec<OwnedServerName>, bool),
    /// Joined it (or asked to, `true`), or not.
    FoundJoined(OwnedRoomId, bool, Result<(), String>),
    Scrolled,
    OutOfView(OutOfView),
    /// Scroll the room list to this offset.
    ScrollTo(f32),
    /// Open the room list's "+".
    AddMenu,
    SpaceMenu(SpaceMenu),
    /// The selected space's topic and links, in its topic panel.
    SpaceTopic(OwnedRoomId, topic::Message),
    Arrange(arrange::Message),
    ArrangeFailed(String),
    /// The room list's colours changed, here or on another device.
    Colours(RoomListColours),
    /// Colours from the settings file were moved to the account, or not.
    ColoursMoved(Result<(), String>),
}

/// The space bar and the room list: the space selected, the search, and
/// how the rooms are arranged.
pub struct Sidebar {
    /// The space selected; Home if none.
    pub space: Option<OwnedRoomId>,
    /// The room search text.
    pub filter: String,
    /// Categories, tags and dragging in the room list.
    pub arrange: Arrange,
    /// Unread rooms scrolled out of the room list's view.
    pub out_of_view: OutOfView,
    /// The selected space's topic panel, below its name.
    pub space_topic: topic::Panel,
    /// The rooms of our spaces, for the room search (and spaces'
    /// overviews).
    pub space_rooms: directory::SpaceRooms,
    /// The room list's colours, from the account.
    pub colours: RoomListColours,
}

impl Sidebar {
    pub fn new(session: &Session) -> Self {
        Self {
            space: None,
            filter: String::new(),
            arrange: Arrange::new(session),
            out_of_view: OutOfView::default(),
            space_topic: topic::Panel::default(),
            space_rooms: directory::SpaceRooms::default(),
            colours: session.room_list_colours(),
        }
    }

    /// Select `space` (Home if none), however chosen: another space's topic
    /// shows closed.
    pub fn select(&mut self, space: Option<OwnedRoomId>) {
        if space != self.space {
            self.space_topic = topic::Panel::default();
        }
        self.space = space;
    }

    /// Esc, after all else: a drag or a category's name ends, else the
    /// search is cleared.
    pub fn escape(&mut self) {
        if !self.arrange.escape() {
            self.filter.clear();
        }
    }

    pub fn update(&mut self, message: Message, cx: &mut Cx<'_>) -> Task<app::Message> {
        match message {
            Message::SelectSpace(space) => {
                cx.cover();
                cx.close_card();
                self.select(space);
                self.arrange.escape();
                Task::none()
            }
            Message::Filter(filter) => {
                // A search starting looks through our spaces' rooms anew.
                let load =
                    if self.filter.trim().is_empty() && !filter.trim().is_empty() {
                        let spaces = directory::searched_spaces(cx.lists, None);
                        Task::batch((spaces.iter()).map(|space| {
                            list_space_rooms(cx.session, &mut self.space_rooms, space)
                        }))
                    } else {
                        Task::none()
                    };
                self.filter = filter;
                load
            }
            Message::OpenFirstMatch => {
                let key = (self.space.clone()).map_or(SpaceKey::Home, SpaceKey::Space);
                let found = (cx.lists).search(Some(&key), &cx.settings.room_list, &self.filter);
                match found.first() {
                    Some(room) => {
                        let id = room.id.clone();
                        self.filter.clear();
                        Task::done(app::Message::Pane(pane::Message::Open(id)))
                    }
                    None => Task::none(),
                }
            }
            // `Main`'s, which has the room list as shown.
            Message::GoTo(_) => Task::none(),
            Message::FocusSearch => iced::widget::operation::focus(SEARCH),
            Message::SpaceRoomsLoaded(space, result) => {
                self.space_rooms.loaded(space, result);
                Task::none()
            }
            Message::JoinFound(room, via, knock) => {
                self.space_rooms.busy.insert(room.clone());
                let join = directory::join(cx.session.clone(), room.clone().into(), via, knock);
                task::request(join, move |result| {
                    to(Message::FoundJoined(room, knock, result.map(|_| ())))
                })
            }
            Message::FoundJoined(room, knock, result) => {
                self.space_rooms.busy.remove(&room);
                match result {
                    Ok(()) if knock => Task::none(),
                    Ok(()) => Task::done(app::Message::Pane(pane::Message::Open(room))),
                    Err(e) => {
                        cx.notices.report(format!("Could not join: {e}"), None);
                        Task::none()
                    }
                }
            }
            Message::Scrolled => Task::none(),
            Message::OutOfView(out_of_view) => {
                self.out_of_view = out_of_view;
                Task::none()
            }
            Message::ScrollTo(offset) => scroll_to(offset),
            Message::AddMenu => {
                *cx.popup = Some(Popup::AddMenu);
                Task::none()
            }
            Message::SpaceMenu(menu) => {
                *cx.popup = Some(Popup::SpaceMenu(menu));
                Task::none()
            }
            Message::SpaceTopic(space, message) => {
                if self.space.as_ref() != Some(&space) {
                    return Task::none();
                }
                let listed = (cx.lists.room(&space)).and_then(|r| r.topic.as_deref());
                topic::update(
                    cx.session,
                    &mut self.space_topic,
                    space.clone(),
                    listed,
                    message,
                    move |message| to(Message::SpaceTopic(space.clone(), message)),
                )
            }
            Message::Arrange(message) => {
                let context = arrange::Context {
                    session: cx.session,
                    space: self.space.as_ref(),
                    rooms: &cx.lists.rooms,
                    grouping: cx.settings.room_list,
                    popup: cx.popup,
                };
                self.arrange.update(message, context)
            }
            Message::ArrangeFailed(e) => {
                cx.notices
                    .report(format!("Could not rearrange the room list: {e}"), None);
                Task::none()
            }
            Message::Colours(colours) => {
                self.colours = colours;
                Task::none()
            }
            Message::ColoursMoved(result) => {
                match result {
                    Ok(()) => {
                        cx.settings.legacy_home_tint = None;
                        cx.settings.legacy_space_tints.clear();
                        cx.save_settings();
                    }
                    Err(e) => tracing::warn!("could not move room list colours: {e}"),
                }
                Task::none()
            }
        }
    }
}

/// List `space`'s rooms (the core asks its server at most once a minute),
/// unless being listed.
pub fn list_space_rooms(
    session: &Arc<Session>,
    space_rooms: &mut directory::SpaceRooms,
    space: &OwnedRoomId,
) -> Task<app::Message> {
    let session = session.clone();
    space_rooms.list(space, |space| {
        task::request(
            {
                let space = space.clone();
                async move { session.space_children(&space).await }
            },
            move |result| to(Message::SpaceRoomsLoaded(space, result)),
        )
    })
}

/// `message` for the space bar or room list, as the app gets it.
pub fn to(message: Message) -> app::Message {
    app::Message::Sidebar(message)
}
