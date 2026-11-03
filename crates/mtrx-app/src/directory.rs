//! Rooms and spaces to join, as a space's overview and the join dialog list
//! them: a picture, the name, what it is about, who can join, and Join, Ask
//! to join or Open.

use std::collections::{HashMap, HashSet};

use iced::{
    Alignment, Element, Length, Theme,
    widget::{button, column, container, hover, row, space, text},
};
use mtrx_core::{
    DirectoryRoom, JoinAccess, Membership, OwnedRoomId, Role, RoomKind, RoomLists, RoomSummary,
};

use crate::{
    app,
    avatar::{Avatars, Shape},
    dialog::Kind,
    ellipsis::ellipsis,
    fonts,
    hint::{Position, hint},
    icons, style,
};

pub const AVATAR: f32 = 28.0;
/// The buttons' column, so they line up.
const ACTION_WIDTH: f32 = 96.0;
const LEAVE_WIDTH: f32 = 72.0;
const ROLE_WIDTH: f32 = 44.0;

/// Our membership of a listed room: as the room list has it (it follows
/// joins from anywhere), else asked to join (`asked` here, or as the server
/// listed it).
pub fn membership(room: &DirectoryRoom, rooms: &[RoomSummary], asked: bool) -> Option<Membership> {
    match rooms.iter().find(|r| r.id == room.id) {
        Some(r) if r.invited => Some(Membership::Invited),
        Some(_) => Some(Membership::Joined),
        None if asked => Some(Membership::Asked),
        None => room.membership.filter(|m| *m == Membership::Asked),
    }
}

/// What a listed room's row needs besides the room.
pub struct Shown<'a> {
    pub avatars: &'a Avatars,
    /// Rooms show their pictures, as in the room list; else who can join.
    pub pictures: bool,
}

/// One room or space, after `lead` (an indent, say); `busy` while joining
/// or asking to. Our role, Leave and the main button line up in columns.
/// One we're in opens on a click; hovered, it offers Leave… and Open.
pub fn entry<'a>(
    room: &DirectoryRoom,
    membership: Option<Membership>,
    busy: bool,
    lead: Element<'a, app::Message>,
    shown: Shown<'_>,
    on_join: app::Message,
    on_knock: app::Message,
) -> Element<'a, app::Message> {
    let access = icon_for(room.access);
    let pictured = shown.pictures || room.is_space;
    let picture = if pictured {
        shown.avatars.view(
            room.id.as_str(),
            &room.name,
            room.avatar.as_deref(),
            AVATAR,
            if room.is_space {
                Shape::RoundedSquare
            } else {
                Shape::Circle
            },
        )
    } else {
        container(access).center(AVATAR).into()
    };
    let mut name = row![ellipsis(room.name.clone()).size(14)]
        .spacing(6)
        .align_y(Alignment::Center);
    if room.suggested {
        name = name.push(text("suggested").size(11).style(text::secondary));
    }
    let details = column![
        name,
        ellipsis(details(room)).size(12).style(text::secondary)
    ]
    .spacing(1)
    .width(Length::Fill);
    let mut line = row![lead, picture, details].spacing(8);
    if pictured {
        line = line.push(container(icon_for(room.access)).center_x(20));
    }
    let joined = membership == Some(Membership::Joined);
    let role = match room.role.filter(|_| joined) {
        Some(Role::Admin) => "Admin",
        Some(Role::Moderator) => "Mod",
        _ => "",
    };
    let line = line
        .push(container(text(role).size(11).style(text::secondary)).align_right(ROLE_WIDTH))
        .push(space().width(LEAVE_WIDTH))
        .push(container(action(room, membership, busy, on_join, on_knock)).width(ACTION_WIDTH))
        .align_y(Alignment::Center);
    let open = if room.is_space {
        app::Message::SelectSpace(Some(room.id.clone()))
    } else {
        app::Message::OpenRoom(room.id.clone())
    };
    let base = match membership {
        // Opens the room, or shows the invite.
        Some(Membership::Joined | Membership::Invited) => button(line)
            .width(Length::Fill)
            .padding([3, 4])
            .style(style::list_item(false))
            .on_press(open.clone()),
        _ => return container(line).padding([3, 4]).into(),
    };
    if !joined {
        return base.into();
    }
    let leave = button(text("Leave…").size(12).center().width(Length::Fill))
        .width(Length::Fill)
        .padding([4, 8])
        .style(style::secondary_button)
        .on_press(app::Message::OpenDialog(Kind::Leave {
            room: room.id.clone(),
            name: room.name.clone(),
        }));
    let top = row![
        space::horizontal(),
        container(leave).width(LEAVE_WIDTH),
        container(act("Open", true, open)).width(ACTION_WIDTH),
    ]
    .spacing(8)
    .padding([3, 4])
    .height(Length::Fill)
    .align_y(Alignment::Center);
    hover(base, top)
}

/// "Space · 4 rooms · the topic's first line · 12 members".
fn details(room: &DirectoryRoom) -> String {
    let mut parts = Vec::new();
    if room.is_space {
        parts.push(match room.children {
            0 => "Space".to_owned(),
            1 => "Space · 1 room".to_owned(),
            n => format!("Space · {n} rooms"),
        });
    }
    if let Some(line) =
        (room.topic.as_deref()).and_then(|t| t.lines().map(str::trim).find(|l| !l.is_empty()))
    {
        parts.push(line.to_owned());
    }
    parts.push(match room.members {
        1 => "1 member".to_owned(),
        n => format!("{n} members"),
    });
    parts.join(" · ")
}

/// Join, Ask to join or Open, as the room allows.
fn action<'a>(
    room: &DirectoryRoom,
    membership: Option<Membership>,
    busy: bool,
    on_join: app::Message,
    on_knock: app::Message,
) -> Element<'a, app::Message> {
    let quiet = |label: &'a str| -> Element<'a, app::Message> {
        container(text(label).size(12).style(text::secondary))
            .center_x(Length::Fill)
            .padding([4, 0])
            .into()
    };
    if busy {
        return quiet(if room.access == JoinAccess::Knock {
            "Asking…"
        } else {
            "Joining…"
        });
    }
    let open = if room.is_space {
        app::Message::SelectSpace(Some(room.id.clone()))
    } else {
        app::Message::OpenRoom(room.id.clone())
    };
    match membership {
        // Open shows while hovered.
        Some(Membership::Joined) => space().into(),
        // Shows the invite, to accept or decline.
        Some(Membership::Invited) => act("Invited", true, open),
        Some(Membership::Asked) => quiet("Asked"),
        None => match room.access {
            JoinAccess::Public | JoinAccess::SpaceMembers => act("Join", true, on_join),
            JoinAccess::Knock => act("Ask to join", true, on_knock),
            JoinAccess::Invite => quiet("Invite only"),
        },
    }
}

/// A row's button: joining stands out, Open is quiet.
fn act<'a>(label: &'a str, primary: bool, on_press: app::Message) -> Element<'a, app::Message> {
    button(text(label).size(12).center().width(Length::Fill))
        .width(Length::Fill)
        .padding([4, 8])
        .style(if primary {
            style::primary_button
        } else {
            style::secondary_button
        })
        .on_press(on_press)
        .into()
}

/// Who can join, as an icon with its hint.
fn icon_for<'a>(access: JoinAccess) -> Element<'a, app::Message> {
    const SIZE: f32 = 15.0;
    let (icon, label) = match access {
        JoinAccess::Public => (icons::hash(SIZE + 2.0, secondary), "Anyone can join"),
        JoinAccess::SpaceMembers => (icons::people(SIZE, secondary), "Space members can join"),
        JoinAccess::Knock => (icons::door(SIZE, secondary), "Ask to join"),
        JoinAccess::Invite => (icons::lock(SIZE, secondary), "Invited people only"),
    };
    hint(icon, label, Position::Left)
}

/// A row of tabs, `chosen` marked.
pub fn tabs<'a, T: Copy + PartialEq + 'a>(
    tabs: &[(&'a str, T)],
    chosen: T,
    on_press: impl Fn(T) -> app::Message,
) -> Element<'a, app::Message> {
    row(tabs.iter().map(|&(label, tab)| {
        let is = tab == chosen;
        button(
            text(label)
                .size(14)
                .font(if is { fonts::SEMIBOLD } else { fonts::INTER })
                .style(if is { text::default } else { text::secondary }),
        )
        .padding([4, 10])
        .style(style::list_item(is))
        .on_press(on_press(tab))
        .into()
    }))
    .spacing(4)
    .into()
}

/// The colour of quiet icons.
pub fn secondary(theme: &Theme) -> iced::Color {
    theme
        .extended_palette()
        .background
        .base
        .text
        .scale_alpha(0.6)
}

/// Each space's rooms as last listed ([`mtrx_core::Session::space_children`]:
/// as its server lists them, with the joined rooms it leaves out), for its
/// overview and the room search. Kept while the app runs, so they show at
/// once; opening the space lists them again behind what is shown.
#[derive(Default)]
pub struct SpaceRooms {
    by_space: HashMap<OwnedRoomId, Listed>,
    /// Joining or asking to join, from the search.
    pub busy: HashSet<OwnedRoomId>,
    pub asked: HashSet<OwnedRoomId>,
}

#[derive(Default)]
struct Listed {
    /// As last listed.
    rooms: Option<Vec<DirectoryRoom>>,
    loading: bool,
    /// Why the last attempt failed.
    failed: Option<String>,
}

impl SpaceRooms {
    /// Start listing `space`'s rooms, unless being listed: whether to ask
    /// for them.
    pub fn start(&mut self, space: &OwnedRoomId) -> bool {
        let listed = self.by_space.entry(space.clone()).or_default();
        if listed.loading {
            return false;
        }
        listed.loading = true;
        true
    }

    /// The answer; on failure, the last list stays.
    pub fn loaded(&mut self, space: OwnedRoomId, result: Result<Vec<DirectoryRoom>, String>) {
        let listed = self.by_space.entry(space).or_default();
        listed.loading = false;
        match result {
            Ok(rooms) => {
                listed.rooms = Some(rooms);
                listed.failed = None;
            }
            Err(e) => listed.failed = Some(e),
        }
    }

    pub fn loading(&self, space: &OwnedRoomId) -> bool {
        self.by_space.get(space).is_some_and(|l| l.loading)
    }

    /// Whether any of `spaces` is being listed.
    pub fn any_loading(&self, spaces: &[OwnedRoomId]) -> bool {
        spaces.iter().any(|s| self.loading(s))
    }

    pub fn failed(&self, space: &OwnedRoomId) -> Option<&str> {
        self.by_space.get(space)?.failed.as_deref()
    }

    /// The space's rooms have been listed.
    pub fn listed(&self, space: &OwnedRoomId) -> bool {
        self.by_space.get(space).is_some_and(|l| l.rooms.is_some())
    }

    /// The space's rooms as last listed (none yet before): suggested first,
    /// then by name.
    pub fn rooms(&self, space: &OwnedRoomId) -> &[DirectoryRoom] {
        (self.by_space.get(space))
            .and_then(|l| l.rooms.as_deref())
            .unwrap_or_default()
    }

    /// Pictures of the listed rooms, with their size.
    pub fn avatars(&self) -> impl Iterator<Item = (&mtrx_core::MxcUri, f32)> {
        self.by_space
            .values()
            .filter_map(|l| l.rooms.as_ref())
            .flatten()
            .filter_map(|r| Some((r.avatar.as_deref()?, AVATAR)))
    }

    /// Rooms of `spaces` not in `rooms` (not joined, nor invited) whose
    /// name contains `filter` (lower case), names starting with it first;
    /// each once.
    pub fn found<'a>(
        &'a self,
        spaces: &[OwnedRoomId],
        rooms: &[RoomSummary],
        filter: &str,
    ) -> Vec<&'a DirectoryRoom> {
        let mut seen = HashSet::new();
        let mut found: Vec<_> = spaces
            .iter()
            .filter_map(|space| self.by_space.get(space)?.rooms.as_ref())
            .flatten()
            .filter(|r| !rooms.iter().any(|known| known.id == r.id))
            .filter(|r| seen.insert(&r.id))
            .filter_map(|r| {
                let name = r.name.to_lowercase();
                name.find(filter).map(|at| (at > 0, r))
            })
            .collect();
        found.sort_by_key(|(later, r)| (*later, r.name.to_lowercase()));
        found.into_iter().map(|(_, r)| r).collect()
    }
}

/// The joined spaces whose rooms the search looks through: those in the
/// selected space (and it), or all at Home.
pub fn searched_spaces(lists: &RoomLists, selected: Option<&OwnedRoomId>) -> Vec<OwnedRoomId> {
    let is_space = |r: &&RoomSummary| r.kind == RoomKind::Space && !r.invited;
    match selected {
        Some(space) => {
            let below = lists.graph.descendants(space);
            std::iter::once(space.clone())
                .chain(
                    (lists.rooms.iter())
                        .filter(is_space)
                        .filter(|r| below.contains(&r.id))
                        .map(|r| r.id.clone()),
                )
                .collect()
        }
        None => (lists.rooms.iter())
            .filter(is_space)
            .map(|r| r.id.clone())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(id: &str, name: &str) -> DirectoryRoom {
        DirectoryRoom {
            id: format!("!{id}:x").try_into().unwrap(),
            alias: None,
            name: name.into(),
            topic: None,
            avatar: None,
            members: 3,
            is_space: false,
            children: 0,
            access: JoinAccess::Public,
            membership: None,
            via: Vec::new(),
            suggested: false,
            role: None,
        }
    }

    fn summary(id: &str, name: &str) -> RoomSummary {
        RoomSummary {
            id: format!("!{id}:x").try_into().unwrap(),
            name: name.into(),
            avatar: None,
            topic: None,
            kind: RoomKind::Room,
            invited: false,
            encrypted: false,
            public: false,
            unread_messages: 0,
            unread_mentions: 0,
            unread_notifications: 0,
            last_activity: None,
            tag: mtrx_core::RoomTag::None,
            direct_user: None,
            alias: None,
            joined_members: 0,
            pinned: Vec::new(),
        }
    }

    #[test]
    fn the_last_list_outlasts_failures() {
        let space: OwnedRoomId = "!space:x".try_into().unwrap();
        let mut rooms = SpaceRooms::default();
        let names = |rooms: &SpaceRooms| -> Vec<String> {
            (rooms.rooms(&space).iter())
                .map(|r| r.name.clone())
                .collect()
        };
        assert!(rooms.start(&space));
        assert!(!rooms.start(&space), "already listing");
        assert!(names(&rooms).is_empty() && !rooms.listed(&space));
        rooms.loaded(space.clone(), Ok(vec![listed("a", "Ice sheets")]));
        assert_eq!(names(&rooms), ["Ice sheets"]);
        assert!(rooms.start(&space), "listed again");
        rooms.loaded(space.clone(), Err("timed out".into()));
        assert_eq!(names(&rooms), ["Ice sheets"]);
        assert_eq!(rooms.failed(&space), Some("timed out"));
    }

    #[test]
    fn finds_rooms_not_joined_once_best_first() {
        let space: OwnedRoomId = "!space:x".try_into().unwrap();
        let other: OwnedRoomId = "!other:x".try_into().unwrap();
        let mut found = SpaceRooms::default();
        found.loaded(
            space.clone(),
            Ok(vec![
                listed("a", "Ice sheets"),
                listed("b", "Sea ice"),
                listed("c", "Joined ice"),
            ]),
        );
        found.loaded(other.clone(), Ok(vec![listed("b", "Sea ice")]));
        let joined = summary("c", "Joined ice");
        let names: Vec<_> = found
            .found(&[space, other], &[joined], "ice")
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(names, ["Ice sheets", "Sea ice"]);
    }
}
