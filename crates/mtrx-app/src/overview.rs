//! A space's overview, in its card: its rooms and subspaces to browse and
//! join, its members, and its settings (the room card's).

use std::{collections::HashSet, fmt};

use iced::{
    Alignment, Element, Length,
    widget::{button, column, container, pick_list, row, space, text, text_input},
};
use mtrx_core::{DirectoryRoom, Member, Membership, OwnedRoomId, OwnedServerName, RoomSummary};

use crate::{
    app,
    dialog::Kind,
    directory, frame,
    hint::{Position, hint},
    icons, info, style,
};

const MEMBERS_WIDTH: f32 = 460.0;
/// Each level of subspaces is indented by this much.
const INDENT: f32 = 22.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Rooms,
    Members,
    Settings,
}

/// Which of the space's rooms to list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Show {
    #[default]
    All,
    NotJoined,
    Joined,
}

impl Show {
    const ALL: [Self; 3] = [Self::All, Self::NotJoined, Self::Joined];
}

impl fmt::Display for Show {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::All => "All rooms",
            Self::NotJoined => "Not joined",
            Self::Joined => "Joined",
        })
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Tab(Tab),
    Filter(String),
    Show(Show),
    /// List the rooms again after the server didn't answer.
    TryAgain,
    /// Show or hide a subspace's rooms.
    Expand(OwnedRoomId),
    Join(OwnedRoomId, Vec<OwnedServerName>),
    /// Ask to join a room that takes requests.
    Knock(OwnedRoomId, Vec<OwnedServerName>),
    /// Joined, or asked to join, or not.
    Done(OwnedRoomId, Result<(), String>),
    MembersLoaded(Result<Vec<Member>, String>),
}

/// What the app should do after a [`Message`].
pub enum Action {
    None,
    /// List these spaces' rooms.
    Load(Vec<OwnedRoomId>),
    LoadMembers,
    Join(OwnedRoomId, Vec<OwnedServerName>),
    Knock(OwnedRoomId, Vec<OwnedServerName>),
}

pub struct Overview {
    pub space: OwnedRoomId,
    pub tab: Tab,
    filter: String,
    show: Show,
    expanded: HashSet<OwnedRoomId>,
    /// Joining or asking to join.
    busy: HashSet<OwnedRoomId>,
    /// Asked to join here, before the server's list says so.
    asked: HashSet<OwnedRoomId>,
    members: Option<Result<Vec<Member>, String>>,
    error: Option<String>,
}

impl Overview {
    pub fn new(space: OwnedRoomId, tab: Tab) -> Self {
        Self {
            space,
            tab,
            filter: String::new(),
            show: Show::default(),
            expanded: HashSet::new(),
            busy: HashSet::new(),
            asked: HashSet::new(),
            members: None,
            error: None,
        }
    }

    /// What to load when the overview opens.
    pub fn opened(&self) -> Action {
        match self.tab {
            Tab::Members => Action::LoadMembers,
            _ => Action::Load(vec![self.space.clone()]),
        }
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::Tab(tab) => {
                self.tab = tab;
                match tab {
                    Tab::Rooms => return Action::Load(vec![self.space.clone()]),
                    Tab::Members if self.members.is_none() => return Action::LoadMembers,
                    _ => {}
                }
            }
            Message::Filter(filter) => self.filter = filter,
            Message::Show(show) => self.show = show,
            Message::TryAgain => return Action::Load(self.shown_spaces()),
            Message::Expand(space) => {
                if !self.expanded.remove(&space) {
                    self.expanded.insert(space.clone());
                    return Action::Load(vec![space]);
                }
            }
            Message::Join(room, via) => {
                self.error = None;
                self.busy.insert(room.clone());
                return Action::Join(room, via);
            }
            Message::Knock(room, via) => {
                self.error = None;
                self.busy.insert(room.clone());
                return Action::Knock(room, via);
            }
            Message::Done(room, result) => {
                self.busy.remove(&room);
                match result {
                    Ok(()) => {
                        self.asked.insert(room);
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            Message::MembersLoaded(result) => self.members = Some(result),
        }
        Action::None
    }

    /// The space and its expanded subspaces, whose rooms are listed.
    fn shown_spaces(&self) -> Vec<OwnedRoomId> {
        std::iter::once(self.space.clone())
            .chain(self.expanded.iter().cloned())
            .collect()
    }

    /// Pictures of the members, with their size (the rooms' are the
    /// app's, with their lists).
    pub fn avatars(&self) -> impl Iterator<Item = (&mtrx_core::MxcUri, f32)> {
        self.members
            .iter()
            .filter_map(|m| m.as_ref().ok())
            .flat_map(|m| crate::members::avatars(m))
    }

    /// While the Rooms tab shows, a pill saying the rooms are being listed,
    /// or that the server didn't answer, to try again.
    pub fn status<'a>(&self, shown: info::Shown<'a>) -> Option<Element<'a, app::Message>> {
        if self.tab != Tab::Rooms {
            return None;
        }
        let spaces = self.shown_spaces();
        let rooms = shown.space_rooms;
        let (line, problem): (Element<'a, app::Message>, bool) = if rooms.any_loading(&spaces) {
            (text("Updating…").size(12).into(), false)
        } else if spaces.iter().any(|s| rooms.failed(s).is_some()) {
            (
                row![
                    text(format!("{} didn't answer", shown.own_user.server_name())).size(12),
                    button(text("Try again").size(12))
                        .padding([0, 6])
                        .style(style::text_button)
                        .on_press(message(Message::TryAgain)),
                ]
                .spacing(6)
                .align_y(Alignment::Center)
                .into(),
                true,
            )
        } else {
            return None;
        };
        Some(
            container(line)
                .padding([4, 12])
                .style(style::status_pill(problem))
                .into(),
        )
    }

    /// The space's topic and links    /// The tabs, above the scrolling content.
    pub fn tabs<'a>(&self) -> Element<'a, app::Message> {
        directory::tabs(
            &[
                ("Rooms", Tab::Rooms),
                ("Members", Tab::Members),
                ("Settings", Tab::Settings),
            ],
            self.tab,
            |tab| message(Message::Tab(tab)),
        )
    }

    /// The chosen tab's content; `settings` is the room card's.
    pub fn view<'a>(
        &'a self,
        space: &'a RoomSummary,
        shown: info::Shown<'a>,
        can_add_rooms: bool,
        can_invite: bool,
        settings: Element<'a, app::Message>,
    ) -> Element<'a, app::Message> {
        match self.tab {
            Tab::Rooms => self.rooms_view(space, shown, can_add_rooms),
            Tab::Members => self.members_view(space, shown, can_invite),
            Tab::Settings => settings,
        }
    }

    fn rooms_view<'a>(
        &'a self,
        space: &'a RoomSummary,
        shown: info::Shown<'a>,
        can_add_rooms: bool,
    ) -> Element<'a, app::Message> {
        let mut tools = row![
            text_input("Filter rooms", &self.filter)
                .on_input(|f| message(Message::Filter(f)))
                .style(style::input)
                .padding([5, 8])
                .size(13)
                .width(Length::Fill),
            pick_list(Show::ALL, Some(self.show), |s| message(Message::Show(s)))
                .text_size(13)
                .padding([5, 8]),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if can_add_rooms {
            tools = tools.push(
                button(text("New room…").size(13))
                    .padding([5, 10])
                    .style(style::secondary_button)
                    .on_press(app::Message::OpenDialog(Kind::NewRoom {
                        space: Some((space.id.clone(), space.name.clone())),
                    })),
            );
            tools = tools.push(hint(
                button(text("Add yours…").size(13))
                    .padding([5, 10])
                    .style(style::secondary_button)
                    .on_press(app::Message::OpenDialog(Kind::AddExisting {
                        space: space.id.clone(),
                        name: space.name.clone(),
                    })),
                "Add rooms you're in to this space",
                Position::Bottom,
            ));
        }
        let mut list = column![].spacing(2);
        list = self.push_rooms(list, &self.space, 0, shown);
        let mut body = column![tools, list].spacing(10);
        if let Some(error) = &self.error {
            body = body.push(text(error).size(12).style(text::danger));
        }
        body.push(
            row![
                icons::lock(13.0, directory::secondary),
                text("Rooms only invited people can join aren't listed.")
                    .size(12)
                    .style(text::secondary),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .into()
    }

    /// The rooms `of` lists that the filter shows, with the rooms of
    /// expanded subspaces below each, indented by `depth`.
    fn push_rooms<'a>(
        &'a self,
        mut list: iced::widget::Column<'a, app::Message>,
        of: &OwnedRoomId,
        depth: u16,
        shown: info::Shown<'a>,
    ) -> iced::widget::Column<'a, app::Message> {
        let indent = f32::from(depth) * INDENT;
        let listing = shown.space_rooms;
        let rooms = listing.rooms(of);
        // Nothing to show yet: room for what comes, or why it didn't.
        if rooms.is_empty() && !listing.listed(of) {
            let waiting = listing.failed(of).is_none();
            if depth == 0 && waiting {
                return list.push(frame::loading());
            }
            let line = if waiting {
                text("Loading…").size(12).style(text::secondary)
            } else {
                text("Couldn't list the rooms.")
                    .size(12)
                    .style(text::secondary)
            };
            return list.push(container(line).padding(iced::Padding::new(4.0).left(indent + 4.0)));
        }
        let filter = self.filter.trim().to_lowercase();
        let mut any = false;
        for room in rooms {
            let membership =
                directory::membership(room, shown.rooms, self.asked.contains(&room.id));
            let joined = membership == Some(Membership::Joined);
            let expanded = room.is_space && self.expanded.contains(&room.id);
            // A subspace stays while open, for its rooms.
            let matches = (filter.is_empty() || room.name.to_lowercase().contains(&filter))
                && match self.show {
                    Show::All => true,
                    Show::Joined => joined,
                    Show::NotJoined => !joined,
                };
            if !matches && !expanded {
                continue;
            }
            any = true;
            list = list.push(self.room_row(room, membership, indent, shown));
            if expanded {
                list = self.push_rooms(list, &room.id, depth + 1, shown);
            }
        }
        if !any {
            let empty = if rooms.is_empty() {
                "No rooms you can see."
            } else {
                "No rooms match."
            };
            list = list.push(
                container(text(empty).size(12).style(text::secondary))
                    .padding(iced::Padding::new(4.0).left(indent + 4.0)),
            );
        }
        list
    }

    fn room_row<'a>(
        &'a self,
        room: &DirectoryRoom,
        membership: Option<Membership>,
        indent: f32,
        shown: info::Shown<'a>,
    ) -> Element<'a, app::Message> {
        let expander: Element<'a, app::Message> = if room.is_space {
            button(icons::chevron(
                12.0,
                self.expanded.contains(&room.id),
                directory::secondary,
            ))
            .padding(4)
            .style(style::list_item(false))
            .on_press(message(Message::Expand(room.id.clone())))
            .into()
        } else {
            space().width(20).into()
        };
        directory::entry(
            room,
            membership,
            self.busy.contains(&room.id),
            row![space().width(indent), expander].into(),
            directory::Shown {
                avatars: shown.avatars,
                pictures: shown.room_pictures,
            },
            message(Message::Join(room.id.clone(), room.via.clone())),
            message(Message::Knock(room.id.clone(), room.via.clone())),
        )
    }

    fn members_view<'a>(
        &'a self,
        space: &'a RoomSummary,
        shown: info::Shown<'a>,
        can_invite: bool,
    ) -> Element<'a, app::Message> {
        let mut top = row![
            text(match space.joined_members {
                1 => "1 member".to_owned(),
                n => format!("{n} members"),
            })
            .size(13)
            .style(text::secondary)
            .width(Length::Fill)
        ]
        .align_y(Alignment::Center);
        if can_invite {
            top = top.push(
                button(text("Invite…").size(13))
                    .padding([5, 10])
                    .style(style::soft_button)
                    .on_press(app::Message::OpenDialog(Kind::Invite {
                        room: space.id.clone(),
                        name: space.name.clone(),
                    })),
            );
        }
        let members: Element<'a, app::Message> = match &self.members {
            None => frame::loading(),
            Some(_) => crate::members::rows(
                self.members.as_ref(),
                crate::members::Shown {
                    room: &space.id,
                    avatars: shown.avatars,
                    presences: shown.presences,
                    menu: shown.member_menu,
                },
            ),
        };
        // Names and roles close together, not across the whole card.
        container(column![top, members].spacing(8).max_width(MEMBERS_WIDTH))
            .center_x(Length::Fill)
            .into()
    }
}

fn message(message: Message) -> app::Message {
    app::Message::Info(info::Message::Overview(message))
}
