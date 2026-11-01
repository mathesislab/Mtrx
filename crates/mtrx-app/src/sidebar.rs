//! Space bar and room list.

use std::collections::BTreeSet;

use iced::{
    Alignment, Color, Element, Length, Padding, Rectangle, Task, Theme, Vector,
    advanced::widget::{Id, Operation, operation},
    keyboard::{
        Key, Modifiers,
        key::{self, Named},
    },
    widget::{
        button, checkbox, column, container, mouse_area, row, rule, scrollable, space, stack, text,
        text_input,
    },
};
use mtrx_core::{
    CallService, DirectoryRoom, Grouping, Layout, MxcUri, OwnedRoomId, OwnedUserId, RoomKind,
    RoomSummary, RoomTag, ScheduledMessage, Section, Sort, Topic, UserId, top_level_spaces,
};

use crate::{
    app::Message,
    arrange::{self, Arrange, NAME_INPUT},
    avatar::{Avatars, Shape},
    context_menu::{self, context_menu},
    dialog::Kind,
    directory,
    ellipsis::{ellipsis, followed_by},
    fonts,
    hint::{Position, hint, shortcut},
    icons, info, overview,
    popover::{Edge, Side, popover},
    presence::{self, Presences},
    spelling,
    style::{self, ROOM_AVATAR, ROOM_ROW, SPACE_AVATAR, SPACE_BAR_WIDTH},
    tint::Tint,
    topic,
};

/// The space's topic panel at most; longer, it scrolls.
const TOPIC_HEIGHT: f32 = 420.0;
/// The line previewing the space's topic.
const PREVIEW_HEIGHT: f32 = 18.0;

/// The room search field, focused by ⌘K / Ctrl+K.
pub const SEARCH: &str = "room-search";

/// Spaces containing `room`, nearest first (through nested spaces too).
pub fn ancestors(rooms: &[RoomSummary], room: &OwnedRoomId) -> Vec<OwnedRoomId> {
    let parents = |id: &OwnedRoomId| {
        rooms
            .iter()
            .find(|r| &r.id == id)
            .map(|r| r.parents.clone())
            .unwrap_or_default()
    };
    let mut found: Vec<OwnedRoomId> = Vec::new();
    let mut next = parents(room);
    while !next.is_empty() {
        let mut further = Vec::new();
        for space in next {
            if !found.contains(&space) {
                further.extend(parents(&space));
                found.push(space);
            }
        }
        next = further;
    }
    found
}

/// Whether anything `space` lists (Home for `None`) is unread: its dot in
/// the space bar. Direct chats light only Home's: a space lists the chats
/// with its members, and one message would light every space shared with
/// its sender (#45).
fn unread(
    rooms: &[RoomSummary],
    space: Option<&OwnedRoomId>,
    arrange: &Arrange,
    grouping: &Grouping,
) -> bool {
    arrange
        .listed(rooms, space, grouping)
        .iter()
        .filter(|r| space.is_none() || r.kind != RoomKind::Direct)
        .any(|r| is_unread(r))
}

/// The room list's sections and their rooms, in display order; sections
/// may be empty. Notes pinned at the top are not in them.
fn sections<'a>(view: &RoomListView<'a>) -> Vec<(Section, Vec<&'a RoomSummary>)> {
    let space = view.selected_space;
    let pinned = match view.notes {
        Some(Notes::Room(notes)) => Some(&notes.id),
        _ => None,
    };
    let listed = view
        .arrange
        .listed(view.rooms, space, &view.grouping)
        .into_iter()
        .filter(|r| Some(&r.id) != pinned);
    view.arrange
        .categories
        .layout(space.map(|s| s.as_ref()))
        .group(listed, &view.grouping)
}

/// The sections the room list shows, with their rooms: Invites when there
/// are some, categories always, other sections while they have rooms (or
/// while a room is dragged, to drop it there).
fn shown_sections<'a>(view: &RoomListView<'a>) -> Vec<(Section, Vec<&'a RoomSummary>)> {
    let dragging =
        (view.arrange.dragging()).is_some_and(|d| matches!(d.item, arrange::Dragged::Room(_)));
    sections(view)
        .into_iter()
        .filter(|(section, entries)| match section {
            Section::Invites => !entries.is_empty(),
            Section::Category(_) => true,
            _ => dragging || !entries.is_empty(),
        })
        .collect()
}

/// The sections shown above and below `section` in `order`, which it
/// moves past.
fn neighbours<'a>(
    order: &'a [Section],
    section: &Section,
) -> (Option<&'a Section>, Option<&'a Section>) {
    let Some(at) = order.iter().position(|s| s == section) else {
        return (None, None);
    };
    (
        at.checked_sub(1).and_then(|i| order.get(i)),
        order.get(at + 1),
    )
}

/// What the search finds in what `space` (or Home) lists: rooms and chats
/// whose name contains `filter` (lower case), names starting with it first.
pub fn search<'a>(
    rooms: &'a [RoomSummary],
    space: Option<&OwnedRoomId>,
    arrange: &Arrange,
    grouping: &Grouping,
    filter: &str,
) -> Vec<&'a RoomSummary> {
    let mut found: Vec<_> = arrange
        .listed(rooms, space, grouping)
        .into_iter()
        .filter(|r| r.kind != RoomKind::Space)
        .filter_map(|r| {
            let name = r.name.to_lowercase();
            name.find(filter).map(|at| (at > 0, r))
        })
        .collect();
    found.sort_by_key(|(later, r)| (*later, r.name.to_lowercase()));
    found.into_iter().map(|(_, r)| r).collect()
}

/// The room the search field's Enter opens.
pub fn first_match<'a>(
    rooms: &'a [RoomSummary],
    space: Option<&OwnedRoomId>,
    arrange: &Arrange,
    grouping: &Grouping,
    filter: &str,
) -> Option<&'a RoomSummary> {
    search(
        rooms,
        space,
        arrange,
        grouping,
        &filter.trim().to_lowercase(),
    )
    .into_iter()
    .next()
}

/// The rooms the room list shows (in collapsed sections too), or those
/// found by the search; not the pinned Notes.
fn listed<'a>(view: &RoomListView<'a>) -> Vec<&'a RoomSummary> {
    let filter = view.filter.trim().to_lowercase();
    if filter.is_empty() {
        sections(view)
            .into_iter()
            .flat_map(|(_, entries)| entries)
            .collect()
    } else {
        let (space, arrange) = (view.selected_space, view.arrange);
        search(view.rooms, space, arrange, &view.grouping, &filter)
    }
}

/// Rooms of our spaces (the selected one's, or all at Home) not joined yet
/// that the search finds.
fn not_joined<'a>(view: &RoomListView<'a>, filter: &str) -> Vec<&'a DirectoryRoom> {
    let spaces = directory::searched_spaces(view.rooms, view.selected_space);
    view.space_rooms.found(&spaces, view.rooms, filter)
}

/// The rooms with a row in the room list, in its order: the pinned Notes,
/// then each section's, or what the search found.
fn rows<'a>(view: &RoomListView<'a>) -> Vec<&'a RoomSummary> {
    if !view.filter.trim().is_empty() {
        return listed(view);
    }
    let layout = (view.arrange.categories).layout(view.selected_space.map(|s| s.as_ref()));
    let notes = match view.notes {
        Some(Notes::Room(notes)) => Some(notes),
        _ => None,
    };
    let sections = sections(view).into_iter().flat_map(|(section, entries)| {
        let collapsed = layout.is_collapsed(&section);
        entries
            .into_iter()
            .filter(move |room| view.shows(room, collapsed))
    });
    notes.into_iter().chain(sections).collect()
}

/// Going to another room from the keyboard, in the room list's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Go {
    /// ⌥⌘↑, ⌃⇧Tab.
    Above,
    /// ⌥⌘↓, ⌃Tab.
    Below,
    /// ⌘1 to ⌘9: the first to ninth row, counted from 0.
    Row(usize),
}

impl Go {
    /// Where the key goes. Also while a field has the focus: fields leave
    /// these keys (see the composer's key binding).
    pub fn from_key(key: &Key, physical_key: key::Physical, modifiers: Modifiers) -> Option<Self> {
        let up_or_down = |up| if up { Go::Above } else { Go::Below };
        match key.as_ref() {
            Key::Named(arrow @ (Named::ArrowUp | Named::ArrowDown))
                if modifiers.command() && modifiers.alt() && !modifiers.shift() =>
            {
                Some(up_or_down(arrow == Named::ArrowUp))
            }
            Key::Named(Named::Tab) if modifiers.control() => Some(up_or_down(modifiers.shift())),
            // By the key's place, also where digits need Shift (AZERTY).
            Key::Character(c) if modifiers.command() && !modifiers.alt() => {
                use key::Code::*;
                let row = match physical_key {
                    key::Physical::Code(code) => [
                        Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
                    ]
                    .iter()
                    .position(|digit| *digit == code),
                    key::Physical::Unidentified(_) => None,
                };
                let row = row.or_else(|| c.parse::<usize>().ok()?.checked_sub(1))?;
                (row < 9).then_some(Go::Row(row))
            }
            _ => None,
        }
    }
}

/// The room `go` leads to from the open one (from outside the list: the
/// first or last); none past the ends.
pub fn go_to<'a>(view: &RoomListView<'a>, go: Go) -> Option<&'a RoomSummary> {
    let rows = rows(view);
    let at = (view.open).and_then(|open| rows.iter().position(|room| &room.id == open));
    let index = match (go, at) {
        (Go::Row(row), _) => row,
        (Go::Above, Some(at)) => at.checked_sub(1)?,
        (Go::Below, Some(at)) => at + 1,
        (Go::Above, None) => rows.len().checked_sub(1)?,
        (Go::Below, None) => 0,
    };
    rows.get(index).copied()
}

/// Pictures in the space bar and room list, with their sizes.
pub fn avatars<'a>(view: &RoomListView<'a>) -> Vec<(&'a MxcUri, f32)> {
    let spaces = top_level_spaces(view.rooms)
        .into_iter()
        .filter_map(|s| Some((s.avatar.as_deref()?, SPACE_AVATAR)));
    let listed = listed(view)
        .into_iter()
        .filter(|r| view.room_pictures || r.kind != RoomKind::Room)
        .filter_map(|r| Some((r.avatar.as_deref()?, ROOM_AVATAR)));
    let filter = view.filter.trim().to_lowercase();
    let unjoined = if filter.is_empty() {
        Vec::new()
    } else {
        not_joined(view, &filter)
    };
    let unjoined = unjoined
        .into_iter()
        .filter_map(|r| Some((r.avatar.as_deref()?, directory::AVATAR)));
    spaces.chain(listed).chain(unjoined).collect()
}

/// The people of the direct chats the room list shows, in its order,
/// without us (the Notes to self).
pub fn direct_users<'a>(view: &RoomListView<'a>, own: &UserId) -> Vec<&'a OwnedUserId> {
    listed(view)
        .into_iter()
        .filter(|r| r.kind == RoomKind::Direct)
        .filter_map(|r| r.direct_user.as_ref())
        .filter(|user| *user != own)
        .collect()
}

/// A menu of the space bar: its "+", or a space's (right-click).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpaceMenu {
    Add,
    Of(OwnedRoomId),
}

/// `menu`: the open one; `update`: a newer version not yet seen in
/// Settings, a dot on the gear.
pub fn space_bar<'a>(
    rooms: &[RoomSummary],
    selected: Option<&OwnedRoomId>,
    arrange: &Arrange,
    grouping: &Grouping,
    avatars: &Avatars,
    menu: Option<&SpaceMenu>,
    update: bool,
) -> Element<'a, Message> {
    let entry = |icon: Element<'a, Message>, title: String, target: Option<OwnedRoomId>, unread| {
        let is_selected = selected == target.as_ref();
        let mut icon = stack![icon];
        if unread {
            let dot = container(space())
                .width(10)
                .height(10)
                .style(style::unread_dot);
            icon = icon.push(container(dot).align_right(SPACE_AVATAR));
        }
        let entry_button = button(icon)
            .padding(4)
            .style(style::list_item(is_selected))
            .on_press(Message::SelectSpace(target.clone()));
        let entry_button = with_tooltip(entry_button, title.clone());
        // A space's own menu, on right-click.
        let entry_button: Element<'a, Message> = match target {
            Some(space) => {
                let open = menu == Some(&SpaceMenu::Of(space.clone()));
                context_menu(
                    entry_button,
                    open.then(|| space_menu(space.clone(), title)),
                    Message::SpaceMenu(Some(SpaceMenu::Of(space))),
                )
                .on_dismiss(Message::SpaceMenu(None))
                .into()
            }
            None => entry_button,
        };
        container(entry_button).center_x(Length::Fill)
    };

    let mut bar = column![entry(
        home_icon(),
        "Home".into(),
        None,
        unread(rooms, None, arrange, grouping)
    )];
    for space in top_level_spaces(rooms) {
        let icon = avatars.view(
            space.id.as_str(),
            &space.name,
            space.avatar.as_deref(),
            SPACE_AVATAR,
            Shape::RoundedSquare,
        );
        bar = bar.push(entry(
            icon,
            space.name.clone(),
            Some(space.id.clone()),
            unread(rooms, Some(&space.id), arrange, grouping),
        ));
    }
    let adding = menu == Some(&SpaceMenu::Add);
    let add = button(
        container(text("+").size(22).font(fonts::SEMIBOLD))
            .center(SPACE_AVATAR)
            .style(style::new_space_tile),
    )
    .padding(4)
    .style(style::list_item(adding))
    .on_press(Message::SpaceMenu((!adding).then_some(SpaceMenu::Add)));
    let add_menu = adding.then(|| {
        context_menu::menu([
            ("New space…".to_owned(), Message::OpenDialog(Kind::NewSpace)),
            ("Join a space or room…".to_owned(), Message::OpenJoin),
        ])
    });
    let add = popover(
        with_tooltip(add, "New space, or join one".into()),
        add_menu,
        Side::Below,
        Edge::Left,
    )
    .on_dismiss(Message::SpaceMenu(None));
    bar = bar.push(container(add).center_x(Length::Fill));
    let mut gear = stack![icons::gear(20.0, |theme| {
        theme
            .extended_palette()
            .background
            .base
            .text
            .scale_alpha(0.7)
    })];
    if update {
        let dot = container(space())
            .width(8)
            .height(8)
            .style(style::update_dot);
        gear = gear.push(container(dot).align_right(20.0));
    }
    let settings = button(gear)
        .padding(8)
        .style(style::list_item(false))
        .on_press(Message::ShowSettings(true));
    let tip = if update {
        "Settings: a new version of Mtrx is available"
    } else {
        "Settings"
    };
    container(column![
        scrollable(bar.spacing(4).width(Length::Fill).padding([8, 0])).height(Length::Fill),
        container(with_tooltip(settings, tip.into()))
            .center_x(Length::Fill)
            .padding([8, 0]),
    ])
    .width(SPACE_BAR_WIDTH)
    .height(Length::Fill)
    .style(style::space_bar)
    .into()
}

/// Home in the space bar: a tile like a space avatar.
fn home_icon<'a>() -> Element<'a, Message> {
    container(text("⌂").size(20).font(fonts::SEMIBOLD))
        .center(SPACE_AVATAR)
        .style(style::home_tile)
        .into()
}

/// What the room list shows.
pub struct RoomListView<'a> {
    pub rooms: &'a [RoomSummary],
    pub selected_space: Option<&'a OwnedRoomId>,
    /// The room shown on the right.
    pub open: Option<&'a OwnedRoomId>,
    /// The first sync finished.
    pub loaded: bool,
    pub avatars: &'a Avatars,
    pub scheduled: &'a [ScheduledMessage],
    /// The rooms with a draft on this device.
    pub drafts: &'a BTreeSet<OwnedRoomId>,
    /// The "+" menu is open.
    pub add_menu: bool,
    /// The search text.
    pub filter: &'a str,
    pub arrange: &'a Arrange,
    pub grouping: Grouping,
    pub out_of_view: OutOfView,
    pub width: f32,
    /// The list's colour here.
    pub tint: Tint,
    /// Rooms show their pictures rather than a public/private symbol.
    pub room_pictures: bool,
    /// The Notes to self, pinned at the top; `None` if not pinned.
    pub notes: Option<Notes<'a>>,
    /// Dots on the pictures of direct chats.
    pub presences: &'a Presences,
    /// The rooms of our spaces, for the search to find those not joined.
    pub space_rooms: &'a directory::SpaceRooms,
    /// The selected space's topic panel.
    pub space_topic: &'a topic::Panel,
    /// Our call services, naming calls in the topic.
    pub call_services: &'a [CallService],
    /// The topic panel's width: a room's, floating over the room.
    pub topic_width: f32,
}

/// Below the space's name, a caret and its topic's first line, opening the
/// topic in full and its links (or its editor) over the room list, as a
/// room's below its name.
fn space_topic<'a>(
    view: &RoomListView<'a>,
    space: &OwnedRoomId,
    top: Element<'a, Message>,
) -> Element<'a, Message> {
    let panel = view.space_topic;
    let synced = (view.rooms.iter())
        .find(|r| &r.id == space)
        .and_then(|r| r.topic.as_deref());
    let parsed = Topic::parse(panel.shown(synced));
    let can_edit = panel.can_edit == Some(true);
    let Some(preview) = topic::preview(&parsed, view.call_services, can_edit) else {
        return top;
    };
    let message = {
        let space = space.clone();
        move |message| Message::SpaceTopic(space.clone(), message)
    };
    let open = panel.open;
    let mut line = row![icons::chevron(11.0, open, |theme| {
        theme
            .extended_palette()
            .background
            .base
            .text
            .scale_alpha(0.7)
    })]
    .spacing(5)
    // As high with the preview as without (while open), so the list below
    // stays put.
    .height(PREVIEW_HEIGHT)
    .align_y(Alignment::Center);
    if !open {
        line = line.push(ellipsis(preview).size(12).style(text::secondary));
    }
    let header = column![
        top,
        button(line)
            .width(Length::Fill)
            .padding([2, 8])
            .style(style::list_item(false))
            .on_press(message(topic::Message::Toggle)),
    ]
    .spacing(2);
    let shown = open.then(|| {
        container(
            scrollable(
                container(panel.view(&parsed, view.call_services, message.clone())).padding(14),
            )
            .height(Length::Shrink),
        )
        .width(Length::Fill)
        .max_height(TOPIC_HEIGHT)
        .style(style::toolbar)
        .into()
    });
    popover(header, shown, Side::Below, Edge::Left)
        .width(view.topic_width)
        .on_dismiss(message(topic::Message::Dismiss))
        .into()
}

/// The Notes to self at the top of the room list.
#[derive(Clone, Copy)]
pub enum Notes<'a> {
    /// Not started yet: the row starts them.
    Missing,
    Room(&'a RoomSummary),
}

pub fn room_list<'a>(view: RoomListView<'a>) -> Element<'a, Message> {
    let RoomListView {
        rooms,
        selected_space,
        add_menu,
        filter,
        ..
    } = view;
    let title = match selected_space {
        Some(id) => rooms
            .iter()
            .find(|r| &r.id == id)
            .map(|r| r.name.as_str())
            .unwrap_or("Space"),
        None => "Home",
    };

    let add = button(text("+").size(18).font(fonts::SEMIBOLD))
        .padding([0, 8])
        .style(style::list_item(add_menu))
        .on_press(Message::AddMenu(!add_menu));
    // A space's name opens its settings, with its address and link.
    let heading: Element<'a, Message> = match selected_space {
        Some(space) => hint(
            button(ellipsis(title).size(16).font(fonts::SEMIBOLD))
                .padding([4, 8])
                .style(style::list_item(false))
                .on_press(Message::ShowSpace(space.clone(), overview::Tab::Rooms)),
            "Rooms, members and settings of the space",
            Position::Bottom,
        ),
        None => container(ellipsis(title).size(16).font(fonts::SEMIBOLD))
            .padding([4, 8])
            .into(),
    };
    let top = row![
        container(heading).width(Length::Fill),
        with_tooltip(add, "New room, chat or category, or join one".into()),
    ]
    .align_y(Alignment::Center);
    let top = match selected_space {
        Some(space) => space_topic(&view, space, top.into()),
        None => top.into(),
    };
    let mut list = column![top].padding(8);
    if add_menu {
        let space = selected_space.map(|id| (id.clone(), title.to_owned()));
        list = list.push(add_menu_view(space));
    }
    list = list.push(
        container(
            text_input(
                &format!("Search rooms and chats  ({})", shortcut("K")),
                filter,
            )
            .id(SEARCH)
            .on_input(Message::RoomFilter)
            .on_submit(Message::OpenFirstMatch)
            .style(style::input)
            .padding([5, 8])
            .size(13),
        )
        .padding([4, 0]),
    );
    let filter = filter.trim().to_lowercase();
    if filter.is_empty() {
        if let Some(space) = selected_space {
            list = list.push(show_menu(&view, space));
        }
        if let Some(notes) = view.notes {
            list = list.push(view.notes_entry(notes));
        }
        list = list.extend(arranged(&view));
    } else {
        let found = search(rooms, selected_space, view.arrange, &view.grouping, &filter);
        let unjoined = not_joined(&view, &filter);
        let space_rooms = view.space_rooms;
        let looking = space_rooms.any_loading(&directory::searched_spaces(rooms, selected_space));
        if found.is_empty() && unjoined.is_empty() && !looking {
            let none = if selected_space.is_some() {
                "No rooms or chats in this space match."
            } else {
                "No rooms or chats at Home match."
            };
            list = list.push(container(text(none).size(12).style(text::secondary)).padding([8, 8]));
        } else if !found.is_empty() {
            list = list.push(
                container(section_title("Results")).padding(Padding::new(8.0).top(14).bottom(4)),
            );
        }
        for room in found {
            list = list.push(view.room_entry(room));
        }
        if !unjoined.is_empty() {
            list = list.push(
                container(section_title("Not joined")).padding(Padding::new(8.0).top(14).bottom(4)),
            );
        } else if looking {
            list = list.push(
                container(
                    text("Looking in your spaces…")
                        .size(12)
                        .style(text::secondary),
                )
                .padding([8, 8]),
            );
        }
        for room in unjoined {
            let membership =
                directory::membership(room, rooms, space_rooms.asked.contains(&room.id));
            list = list.push(directory::entry(
                room,
                membership,
                space_rooms.busy.contains(&room.id),
                space().width(0).into(),
                directory::Shown {
                    avatars: view.avatars,
                    pictures: view.room_pictures,
                },
                Message::JoinFound(room.id.clone(), room.via.clone(), false),
                Message::JoinFound(room.id.clone(), room.via.clone(), true),
            ));
        }
    }
    if !view.loaded {
        list =
            list.push(container(text("Syncing…").size(12).style(text::secondary)).padding([8, 8]));
    }
    let list = scrollable(list)
        .id(ROOM_LIST)
        .on_scroll(|_| Message::RoomListScrolled)
        .height(Length::Fill);
    container(out_of_view_arrows(list.into(), view.out_of_view))
        .width(view.width)
        .height(Length::Fill)
        .style(style::room_list(view.tint))
        .into()
}

/// The sections of the room list, with menus, the category being renamed,
/// and drop targets while dragging.
fn arranged<'a>(view: &RoomListView<'a>) -> Vec<Element<'a, Message>> {
    let arrange = view.arrange;
    let layout = arrange
        .categories
        .layout(view.selected_space.map(|s| s.as_ref()));
    let dragging = arrange.dragging().is_some();
    let room_dragged = dragging && arrange.dragged_section().is_none();
    let space = view.selected_space;
    let shown = shown_sections(view);
    let order: Vec<Section> = (shown.iter().map(|(s, _)| s.clone()))
        .filter(|s| *s != Section::Invites)
        .collect();
    let landing = landing(arrange, &order);
    let mut elements = Vec::new();
    for (section, entries) in shown {
        let at = order.iter().position(|s| *s == section);
        elements.push(drop_line(at.is_some() && landing == at));
        let collapsed = layout.is_collapsed(&section);
        let renaming = arrange
            .naming
            .as_ref()
            .filter(|n| matches!(&section, Section::Category(id) if n.category == *id));
        let mut block = column![match renaming {
            Some(naming) => name_input(&naming.name),
            None => section_header(&section, collapsed, space, arrange),
        }];
        if let Section::Category(id) = &section
            && arrange.category_menu.as_ref() == Some(id)
        {
            block = block.push(category_menu(layout, space, id, &order));
        }
        let empty = entries.is_empty();
        for room in entries {
            if !view.shows(room, collapsed) {
                continue;
            }
            block = block.push(view.room_entry(room));
            if arrange.room_menu.as_ref() == Some(&room.id) {
                block = block.push(room_menu(room, space, layout, &view.grouping));
            }
        }
        if empty && !collapsed && matches!(section, Section::Category(_)) {
            block = block.push(
                container(text("Drag rooms here").size(12).style(text::secondary)).padding([4, 8]),
            );
        }
        elements.push(if dragging && section != Section::Invites {
            // A room drops into the section, a section takes its place.
            let target = room_dragged && arrange.target.as_ref() == Some(&section);
            mouse_area(
                container(block)
                    .width(Length::Fill)
                    .style(move |theme| style::drop_target(theme, target)),
            )
            .on_enter(arrange_message(arrange::Message::Target(
                section.clone(),
                true,
            )))
            .on_exit(arrange_message(arrange::Message::Target(section, false)))
            .into()
        } else {
            block.into()
        });
    }
    elements.push(drop_line(landing == Some(order.len())));
    elements
}

/// "Show 4 of 6": the button across two thirds of the room list, opening
/// the menu of which of a space's sections are expanded, the others shown
/// collapsed.
fn show_menu<'a>(view: &RoomListView<'a>, space: &OwnedRoomId) -> Element<'a, Message> {
    let arrange = view.arrange;
    let layout = arrange.categories.layout(Some(space));
    let sections: Vec<Section> = (shown_sections(view).into_iter())
        .map(|(section, _)| section)
        .filter(|s| *s != Section::Invites)
        .collect();
    let expanded = sections.iter().filter(|s| !layout.is_collapsed(s)).count();
    let open = arrange.show_menu;
    // While open, a click on it closes the menu as any outside click.
    let toggle = button(
        container(
            row![
                text(format!("Show {expanded} of {}", sections.len())).size(12),
                icons::chevron(10.0, true, |theme| {
                    theme
                        .extended_palette()
                        .background
                        .base
                        .text
                        .scale_alpha(0.7)
                }),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
        .center_x(Length::Fill),
    )
    .width(Length::FillPortion(4))
    .padding([2, 8])
    .style(style::secondary_button)
    .on_press_maybe((!open).then(|| arrange_message(arrange::Message::ShowMenu(true))));
    let notes = view.notes.is_some();
    let menu = open.then(|| show_menu_panel(arrange, Some(space), layout, sections, notes));
    let margin = || iced::widget::space().width(Length::FillPortion(1));
    container(row![
        margin(),
        popover(toggle, menu, Side::Below, Edge::Left)
            .on_dismiss(arrange_message(arrange::Message::ShowMenu(false))),
        margin(),
    ])
    .padding(Padding::new(0.0).top(2).bottom(4))
    .into()
}

/// A space's sections, ticked while expanded and dragged by their grips to
/// other places, then showing or hiding them all; with the Notes pinned at
/// the top (`notes`), where they are set.
fn show_menu_panel<'a>(
    arrange: &Arrange,
    space: Option<&OwnedRoomId>,
    layout: &Layout,
    sections: Vec<Section>,
    notes: bool,
) -> Element<'a, Message> {
    let collapse = |collapsed: &dyn Fn(&Section) -> bool| {
        arrange_message(collapse_sections(&sections, collapsed))
    };
    let landing = landing(arrange, &sections);
    let mut menu = column![];
    for (at, section) in sections.iter().enumerate() {
        let ticked = !layout.is_collapsed(section);
        let toggle = arrange_message(arrange::Message::ToggleCollapsed(section.clone()));
        let name = section_name(arrange, space, section).to_owned();
        let line = row![
            checkbox(ticked).on_toggle({
                let toggle = toggle.clone();
                move |_| toggle.clone()
            }),
            ellipsis(name.clone()).size(13).width(Length::Fill),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let item = button(line)
            .width(Length::Fill)
            .padding([4, 8])
            .style(style::list_item(false))
            .on_press(toggle);
        // The user's names may be cut short.
        let item = match section {
            Section::Category(_) => hint(item, name, Position::Right),
            _ => item.into(),
        };
        let row_message = |entered| match arrange.dragged_section() {
            Some(_) => arrange::Message::Target(section.clone(), entered),
            None => arrange::Message::SectionHovered(section.clone(), entered),
        };
        let line = mouse_area(
            row![item, container(grip(arrange, section)).padding(4)]
                .spacing(1)
                .align_y(Alignment::Center),
        )
        .on_enter(arrange_message(row_message(true)))
        .on_exit(arrange_message(row_message(false)));
        menu = menu.push(drop_line(landing == Some(at))).push(line);
    }
    menu = menu.push(drop_line(landing == Some(sections.len())));
    let note = |line| {
        container(text(line).size(11).style(text::secondary))
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding([2, 8])
    };
    let footer = |label| {
        button(text(label).size(13).width(Length::Fill).center())
            .width(Length::Fill)
            .padding([4, 8])
            .style(style::list_item(false))
    };
    menu = menu
        .push(container(rule::horizontal(1).style(style::separator)).padding([4, 0]))
        .push(
            row![
                footer("Show all").on_press(collapse(&|_| false)),
                footer("Hide all").on_press(collapse(&|_| true))
            ]
            .spacing(4),
        );
    if notes {
        menu = menu.push(note(NOTES_PINNED));
    }
    container(menu)
        .width(SHOW_MENU_WIDTH)
        .padding(4)
        .style(style::toolbar)
        .into()
}

/// The grip dragging `section`, showing while its heading or row in the
/// Show menu is under the cursor, or it is dragged.
fn grip<'a>(arrange: &Arrange, section: &Section) -> Element<'a, Message> {
    let shown = arrange.section_hovered.as_ref() == Some(section)
        || arrange.dragged_section() == Some(section);
    let color: fn(&Theme) -> Color = if shown {
        |theme| {
            theme
                .extended_palette()
                .background
                .base
                .text
                .scale_alpha(0.6)
        }
    } else {
        |_| Color::TRANSPARENT
    };
    let grip = |entered| arrange_message(arrange::Message::Grip(section.clone(), entered));
    mouse_area(icons::grip(14.0, color))
        .on_enter(grip(true))
        .on_exit(grip(false))
        .interaction(iced::mouse::Interaction::Grab)
        .into()
}

/// Where the section being dragged would land among `order`: the gap
/// before the section counted from 0 (the gap after the last for its
/// length); none while it is over its own place.
fn landing(arrange: &Arrange, order: &[Section]) -> Option<usize> {
    let place = |section| order.iter().position(|s| s == section);
    let from = place(arrange.dragged_section()?)?;
    let to = place(arrange.target.as_ref()?)?;
    match to.cmp(&from) {
        std::cmp::Ordering::Less => Some(to),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(to + 1),
    }
}

/// The gap between two sections, a line where a dragged one would land.
fn drop_line<'a>(active: bool) -> Element<'a, Message> {
    container(space())
        .width(Length::Fill)
        .height(2)
        .style(move |theme| style::drop_line(theme, active))
        .into()
}

/// Collapse the `sections` for which `collapsed` holds, expand the others.
fn collapse_sections(
    sections: &[Section],
    collapsed: &dyn Fn(&Section) -> bool,
) -> arrange::Message {
    let sections = (sections.iter())
        .map(|section| (section.clone(), collapsed(section)))
        .collect();
    arrange::Message::Collapse(sections)
}

/// Where the Notes pinned at the top are set, below the Show menu.
const NOTES_PINNED: &str = "Notes at the top: in Settings";

/// The width of the menu of which sections are expanded.
const SHOW_MENU_WIDTH: f32 = 280.0;

fn arrange_message(message: arrange::Message) -> Message {
    Message::Arrange(message)
}

/// A section's name in the room list of `space` (Home for `None`). Home's
/// rooms not in a category are those in no space, unless it lists all.
fn section_name<'a>(
    arrange: &'a Arrange,
    space: Option<&OwnedRoomId>,
    section: &Section,
) -> &'a str {
    let categories = &arrange.categories;
    match section {
        Section::Invites => "Invites",
        Section::Favourites => "Favourites",
        Section::Category(id) => (categories.layout(space.map(|s| s.as_ref())).category(id))
            .map_or("", |c| c.name.as_str()),
        Section::Rooms if space.is_none() && !categories.at_home.all_rooms => "Rooms with no space",
        Section::Rooms => "Rooms",
        Section::People => "Direct messages",
        Section::LowPriority => "Low priority",
    }
}

/// A section's name, which collapses and expands it, and its grip; a
/// category's also has its menu.
fn section_header<'a>(
    section: &Section,
    collapsed: bool,
    space: Option<&OwnedRoomId>,
    arrange: &Arrange,
) -> Element<'a, Message> {
    // The grey of the section's name.
    let chevron = icons::chevron(12.0, !collapsed, |theme| {
        text::secondary(theme).color.unwrap_or(theme.palette().text)
    });
    let title = button(
        row![
            chevron,
            section_title(section_name(arrange, space, section))
        ]
        .spacing(5)
        .align_y(Alignment::Center),
    )
    .padding(0)
    .width(Length::Fill)
    .style(style::text_button)
    .on_press(arrange_message(arrange::Message::ToggleCollapsed(
        section.clone(),
    )));
    let mut header = row![title].spacing(2).align_y(Alignment::Center);
    if let Section::Category(id) = section {
        let open = arrange.category_menu.as_ref() == Some(id);
        header = header.push(with_tooltip(
            button(text("⋯").size(13).style(text::secondary))
                .padding([0, 6])
                .style(style::list_item(open))
                .on_press(arrange_message(arrange::Message::CategoryMenu(
                    (!open).then(|| id.clone()),
                ))),
            "Rename, sort, move or delete the category".into(),
        ));
    }
    if *section != Section::Invites {
        header = header.push(grip(arrange, section));
    }
    let hovered =
        |entered| arrange_message(arrange::Message::SectionHovered(section.clone(), entered));
    // 14 above with the line where a dragged section would land.
    mouse_area(container(header).padding(Padding::new(8.0).top(12).bottom(4)))
        .on_enter(hovered(true))
        .on_exit(hovered(false))
        .into()
}

/// Small upper-case label above a group of rooms.
fn section_title<'a>(title: &str) -> Element<'a, Message> {
    text(title.to_uppercase())
        .size(11)
        .font(fonts::SEMIBOLD)
        .style(text::secondary)
        .into()
}

/// The field renaming a category: Enter saves, Esc cancels.
fn name_input<'a>(name: &str) -> Element<'a, Message> {
    container(
        spelling::checked(text_input("Category name", name), name)
            .id(NAME_INPUT)
            .on_input(|name| arrange_message(arrange::Message::Name(name)))
            .on_submit(arrange_message(arrange::Message::SaveName))
            .style(style::input)
            .padding([4, 8])
            .size(13),
    )
    .padding(Padding::new(0.0).top(8).bottom(4))
    .into()
}

fn menu<'a>(items: Vec<(String, Message)>) -> Element<'a, Message> {
    let items = items.into_iter().map(|(label, message)| {
        button(text(label).size(13))
            .width(Length::Fill)
            .padding([4, 10])
            .style(style::list_item(false))
            .on_press(message)
            .into()
    });
    container(column(items).spacing(1))
        .padding(4)
        .width(Length::Fill)
        .style(style::code_block)
        .into()
}

/// Settings, low priority and categories for a room (the star next to the
/// menu makes it a favourite).
fn room_menu<'a>(
    room: &RoomSummary,
    space: Option<&OwnedRoomId>,
    layout: &Layout,
    grouping: &Grouping,
) -> Element<'a, Message> {
    let move_to = |section| arrange_message(arrange::Message::MoveTo(room.id.clone(), section));
    let mut items = Vec::new();
    if room.kind == RoomKind::Direct {
        items.push(("Info".to_owned(), Message::ShowInfo(info::target_of(room))));
    }
    items.push((
        "Settings".to_owned(),
        Message::ShowInfo(info::Target::Room(room.id.clone())),
    ));
    if grouping.low_priority {
        items.push(match room.tag {
            RoomTag::LowPriority => ("Remove from low priority".into(), move_to(Section::Rooms)),
            _ => ("Low priority".into(), move_to(Section::LowPriority)),
        });
    }
    let current = layout.category_of(&room.id).map(|c| c.id.as_str());
    for category in &layout.categories {
        items.push(if Some(category.id.as_str()) == current {
            (
                format!("Remove from {}", category.name),
                move_to(Section::Rooms),
            )
        } else {
            (
                format!("Move to {}", category.name),
                move_to(Section::Category(category.id.clone())),
            )
        });
    }
    items.push((
        "New category…".into(),
        Message::OpenDialog(Kind::Category {
            space: space.cloned(),
            category: None,
            room: Some(room.id.clone()),
        }),
    ));
    menu(items)
}

/// Rename, choose rooms, sort, reorder and delete a category, one of the
/// sections shown in `order`.
fn category_menu<'a>(
    layout: &Layout,
    in_space: Option<&OwnedRoomId>,
    id: &str,
    order: &[Section],
) -> Element<'a, Message> {
    let Some(category) = layout.category(id) else {
        return space().into();
    };
    let (above, below) = neighbours(order, &Section::Category(id.to_owned()));
    let id = id.to_owned();
    let item = |label: &str, message| (label.to_owned(), arrange_message(message));
    let mut items = vec![
        item("Rename…", arrange::Message::Rename(id.clone())),
        (
            "Choose rooms…".to_owned(),
            Message::OpenDialog(Kind::Category {
                space: in_space.cloned(),
                category: Some(id.clone()),
                room: None,
            }),
        ),
    ];
    items.push(match category.sort {
        Sort::Activity => item("Sort A–Z", arrange::Message::Sort(id.clone(), Sort::Name)),
        Sort::Name => item(
            "Sort by recent activity",
            arrange::Message::Sort(id.clone(), Sort::Activity),
        ),
    });
    let section = Section::Category(id.clone());
    if let Some(above) = above {
        items.push(item(
            "Move up",
            arrange::Message::MoveSection(section.clone(), above.clone()),
        ));
    }
    if let Some(below) = below {
        items.push(item(
            "Move down",
            arrange::Message::MoveSection(section, below.clone()),
        ));
    }
    items.push(item("Delete category", arrange::Message::Delete(id)));
    menu(items)
}

/// The room or section being dragged in the room list of `space` (Home for
/// `None`), following the cursor.
pub fn drag_preview<'a>(
    rooms: &'a [RoomSummary],
    space: Option<&OwnedRoomId>,
    arrange: &Arrange,
    avatars: &Avatars,
) -> Option<Element<'a, Message>> {
    let drag = arrange.dragging()?;
    let at = drag.at?;
    let content: Element<'a, Message> = match &drag.item {
        arrange::Dragged::Room(id) => {
            let room = rooms.iter().find(|r| r.id == *id)?;
            row![
                avatars.view(
                    room.id.as_str(),
                    &room.name,
                    room.avatar.as_deref(),
                    ROOM_AVATAR,
                    Shape::Circle,
                ),
                text(&room.name).size(14),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        }
        arrange::Dragged::Section(section) => section_title(section_name(arrange, space, section)),
    };
    let chip = container(content).padding([4, 10]).style(style::toolbar);
    Some(
        container(chip)
            .padding(Padding::ZERO.left(at.x + 12.0).top(at.y + 6.0))
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
    )
}

/// A space's menu in the space bar.
fn space_menu<'a>(space: OwnedRoomId, name: String) -> Element<'a, Message> {
    context_menu::menu([
        (
            "Overview".to_owned(),
            Message::ShowSpace(space.clone(), overview::Tab::Rooms),
        ),
        (
            "Invite people…".to_owned(),
            Message::OpenDialog(Kind::Invite {
                room: space.clone(),
                name: name.clone(),
            }),
        ),
        (
            "Space settings…".to_owned(),
            Message::ShowSpace(space.clone(), overview::Tab::Settings),
        ),
        (
            "Leave the space…".to_owned(),
            Message::OpenDialog(Kind::Leave { room: space, name }),
        ),
    ])
}

/// What the room list's "+" offers; more inside a space.
fn add_menu_view<'a>(space: Option<(OwnedRoomId, String)>) -> Element<'a, Message> {
    let item = |label: String, message: Message| {
        button(text(label).size(14))
            .width(Length::Fill)
            .padding([5, 10])
            .style(style::list_item(false))
            .on_press(message)
    };
    let dialog = |label: String, kind: Kind| item(label, Message::OpenDialog(kind));
    let mut menu = column![].spacing(1);
    menu = menu.push(dialog(
        match &space {
            Some((_, name)) => format!("New room in {name}…"),
            None => "New room…".into(),
        },
        Kind::NewRoom {
            space: space.clone(),
        },
    ));
    // In a space, its own rooms to join; at Home, any.
    if let Some((id, name)) = &space {
        menu = menu.push(item(
            format!("Browse rooms in {name}…"),
            Message::ShowSpace(id.clone(), overview::Tab::Rooms),
        ));
    }
    menu = menu
        .push(dialog("New direct chat…".into(), Kind::DirectChat))
        .push(dialog(
            "New category…".into(),
            Kind::Category {
                space: space.as_ref().map(|(id, _)| id.clone()),
                category: None,
                room: None,
            },
        ));
    match space {
        None => menu = menu.push(item("Join a space or room…".into(), Message::OpenJoin)),
        Some((id, name)) => {
            menu = menu
                .push(container(rule::horizontal(1).style(style::separator)).padding([4, 6]))
                .push(dialog(
                    "Invite people…".into(),
                    Kind::Invite {
                        room: id.clone(),
                        name: name.clone(),
                    },
                ))
                .push(item(
                    "Space settings…".into(),
                    Message::ShowSpace(id.clone(), overview::Tab::Settings),
                ))
                .push(dialog(
                    "Leave the space…".into(),
                    Kind::Leave { room: id, name },
                ));
        }
    }
    container(menu)
        .padding(4)
        .width(Length::Fill)
        .style(style::code_block)
        .into()
}

/// A room's scheduled messages, as shown in the room list.
struct Later {
    count: usize,
    overdue: usize,
}

impl Later {
    fn of(scheduled: &[ScheduledMessage], room: &OwnedRoomId) -> Option<Self> {
        let mut later = Self {
            count: 0,
            overdue: 0,
        };
        for message in scheduled.iter().filter(|m| &m.room_id == room) {
            later.count += 1;
            later.overdue += usize::from(message.overdue);
        }
        (later.count > 0).then_some(later)
    }

    /// A clock; warning-coloured if any message is overdue.
    fn view<'a>(&self) -> Element<'a, Message> {
        let (color, tip): (fn(&Theme) -> Color, _) = if self.overdue > 0 {
            (
                |theme| theme.extended_palette().warning.strong.color,
                format!("{} overdue scheduled message(s)", self.overdue),
            )
        } else {
            (
                |theme| {
                    theme
                        .extended_palette()
                        .background
                        .strong
                        .text
                        .scale_alpha(0.6)
                },
                format!("{} scheduled message(s)", self.count),
            )
        };
        with_tooltip(icons::clock(14.0, color), tip)
    }
}

impl<'a> RoomListView<'a> {
    /// What a row ends with: a pencil for a draft (in a room not open, where
    /// it shows), a clock for scheduled messages.
    fn marks(&self, room: &RoomSummary) -> Vec<Element<'a, Message>> {
        let mut marks = Vec::new();
        if self.drafts.contains(&room.id) && self.open != Some(&room.id) {
            marks.push(with_tooltip(
                icons::pencil(12.0, style::muted),
                "Draft".into(),
            ));
        }
        if let Some(later) = Later::of(self.scheduled, &room.id) {
            marks.push(later.view());
        }
        marks
    }

    /// Whether `room`, in a section `collapsed` or not, has its row:
    /// collapsed sections still show the open room and unread ones.
    fn shows(&self, room: &RoomSummary, collapsed: bool) -> bool {
        !collapsed || self.open == Some(&room.id) || is_unread(room)
    }

    /// The pinned Notes row: like a room's, but not dragged or tagged, and
    /// greyed until the notes are started, which pressing it does.
    fn notes_entry(&self, notes: Notes<'a>) -> Element<'a, Message> {
        let room = match notes {
            Notes::Room(room) => room,
            Notes::Missing => {
                let line = row![
                    container(icons::notebook(16.0, faint)).center(ROOM_AVATAR),
                    text("Notes").size(14).style(|theme: &Theme| text::Style {
                        color: Some(faint(theme)),
                    }),
                ]
                .spacing(8)
                .align_y(Alignment::Center);
                return hint(
                    button(container(line).center_y(Length::Fill))
                        .width(Length::Fill)
                        .height(ROOM_ROW)
                        .padding([0, 8])
                        .style(style::list_item(false))
                        .on_press(Message::StartNotes),
                    "Start your notes to self",
                    Position::Bottom,
                );
            }
        };
        let selected = self.open == Some(&room.id);
        let menu_open = self.arrange.room_menu.as_ref() == Some(&room.id);
        let font = if is_unread(room) {
            fonts::SEMIBOLD
        } else {
            fonts::INTER
        };
        let name = ellipsis("Notes").size(14).font(font);
        let name = match unread_count(room) {
            Some(count) => followed_by(name, count, 6.0),
            None => name.width(Length::Fill).into(),
        };
        let mut line = row![
            container(icons::notebook(16.0, |theme| {
                theme
                    .extended_palette()
                    .background
                    .base
                    .text
                    .scale_alpha(0.7)
            }))
            .center(ROOM_AVATAR),
            name
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let toggle_menu = arrange_message(arrange::Message::RoomMenu(
            (!menu_open).then(|| room.id.clone()),
        ));
        if self.arrange.notes_hovered || menu_open {
            line = line.push(with_tooltip(
                button(text("⋯").size(13))
                    .padding([0, 4])
                    .style(style::list_item(menu_open))
                    .on_press(toggle_menu.clone()),
                "Settings (or right-click)".into(),
            ));
        } else {
            line = line.extend(self.marks(room));
        }
        let entry = button(container(line).center_y(Length::Fill))
            .width(Length::Fill)
            .height(ROOM_ROW)
            .padding([0, 8])
            .style(style::list_item(selected))
            .on_press(Message::OpenRoom(room.id.clone()));
        let area = mouse_area(entry)
            .on_enter(arrange_message(arrange::Message::NotesRow(true)))
            .on_exit(arrange_message(arrange::Message::NotesRow(false)))
            .on_right_press(toggle_menu);
        let mut block = column![area];
        if menu_open {
            block = block.push(menu(vec![(
                "Settings".to_owned(),
                Message::ShowInfo(info::Target::Room(room.id.clone())),
            )]));
        }
        // Found by `find_out_of_view` when unread.
        if is_unread(room) {
            container(block).id(unread_id(&room.id)).into()
        } else {
            block.into()
        }
    }

    /// A room's row: opens the room on click, can be dragged to another
    /// section, and has a menu on right click or its "⋯".
    fn room_entry(&self, room: &'a RoomSummary) -> Element<'a, Message> {
        let selected = self.open == Some(&room.id);
        let menu_open = self.arrange.room_menu.as_ref() == Some(&room.id);
        let hovered =
            matches!(&self.arrange.hovered, Some(arrange::Dragged::Room(id)) if *id == room.id);
        let font = if is_unread(room) {
            fonts::SEMIBOLD
        } else {
            fonts::INTER
        };
        // People keep their pictures; rooms may show whether they are open.
        let avatar = if self.room_pictures || room.kind != RoomKind::Room {
            let picture = self.avatars.view(
                room.id.as_str(),
                &room.name,
                room.avatar.as_deref(),
                ROOM_AVATAR,
                Shape::Circle,
            );
            let person = room
                .direct_user
                .as_ref()
                .filter(|_| room.kind == RoomKind::Direct);
            let presence = person.and_then(|user| self.presences.get(user));
            let tint = self.tint;
            presence::with_dot(picture, presence, ROOM_AVATAR, move |theme| {
                tint.color(theme)
            })
        } else {
            access_symbol(room.public)
        };
        let name = ellipsis(&room.name).size(14).font(font);
        // Right after the name, so nothing moves when the star and menu
        // show on hover.
        let name = match unread_count(room) {
            Some(count) => followed_by(name, count, 6.0),
            None => name.width(Length::Fill).into(),
        };
        let mut line = row![avatar, name].spacing(8).align_y(Alignment::Center);
        let menu = arrange_message(arrange::Message::RoomMenu(
            (!menu_open).then(|| room.id.clone()),
        ));
        let dragging = self.arrange.dragging().is_some();
        let actions = (hovered || menu_open) && !room.invited && !dragging;
        // On hover the star and menu take the place of the draft's pencil
        // and the scheduled-message clock, so nothing else moves. Favourites have their own section.
        if actions {
            line = line.push(star_button(&room.id, room.tag == RoomTag::Favourite));
            line = line.push(with_tooltip(
                button(text("⋯").size(13))
                    .padding([0, 4])
                    .style(style::list_item(menu_open))
                    .on_press(menu.clone()),
                "Settings, low priority, categories (or right-click)".into(),
            ));
        } else {
            line = line.extend(self.marks(room));
        }
        let entry = button(container(line).center_y(Length::Fill))
            .width(Length::Fill)
            .height(ROOM_ROW)
            .padding([0, 8])
            .style(style::list_item(selected))
            .on_press(Message::OpenRoom(room.id.clone()));
        let row = |enter| arrange_message(arrange::Message::Row(room.id.clone(), enter));
        let mut area = mouse_area(entry).on_enter(row(true)).on_exit(row(false));
        if !room.invited {
            area = area.on_right_press(menu);
        }
        // Found by `find_out_of_view` when unread.
        if is_unread(room) {
            container(area).id(unread_id(&room.id)).into()
        } else {
            area.into()
        }
    }
}

/// Greyed text and icons, e.g. of Notes not started yet.
fn faint(theme: &Theme) -> Color {
    theme
        .extended_palette()
        .background
        .base
        .text
        .scale_alpha(0.45)
}

/// In place of a room's picture: # for a public room, a lock for one only
/// invited people (or a space's members) may join.
fn access_symbol<'a>(public: bool) -> Element<'a, Message> {
    let symbol: Element<'a, Message> = if public {
        icons::hash(16.0, |theme| {
            theme
                .extended_palette()
                .background
                .base
                .text
                .scale_alpha(0.55)
        })
    } else {
        icons::lock(14.0, |theme| {
            theme
                .extended_palette()
                .background
                .base
                .text
                .scale_alpha(0.5)
        })
    };
    container(symbol).center(ROOM_AVATAR).into()
}

/// Whether the room has unread messages or notifications: bold, shown in
/// a collapsed section, and found when out of view.
fn is_unread(room: &RoomSummary) -> bool {
    room.unread_messages > 0 || room.unread_notifications > 0
}

/// Its notifications (the dock's count adds these up), in red when they
/// mention the user.
fn unread_count<'a>(room: &RoomSummary) -> Option<Element<'a, Message>> {
    let highlight = room.unread_mentions > 0;
    let count = if highlight {
        room.unread_mentions
    } else {
        room.unread_notifications
    };
    (count > 0).then(|| {
        container(text(count.to_string()).size(11).font(fonts::SEMIBOLD))
            .padding([1, 6])
            .style(move |t| style::badge(t, highlight))
            .into()
    })
}

/// A gold star on favourites; an outline making a room one.
pub fn star_button<'a>(room: &OwnedRoomId, favourite: bool) -> Element<'a, Message> {
    let tag = if favourite {
        RoomTag::None
    } else {
        RoomTag::Favourite
    };
    star_for(
        arrange_message(arrange::Message::Tag(room.clone(), tag)),
        favourite,
    )
}

/// A star sending `message` when clicked.
pub fn star_for<'a>(message: Message, favourite: bool) -> Element<'a, Message> {
    let tip = if favourite {
        "Remove from favourites"
    } else {
        "Favourite"
    };
    with_tooltip(
        button(icons::star(14.0, favourite, |theme| {
            theme
                .extended_palette()
                .background
                .base
                .text
                .scale_alpha(0.55)
        }))
        .padding([2, 3])
        .style(style::list_item(false))
        .on_press(message),
        tip.to_owned(),
    )
}

fn with_tooltip<'a>(content: impl Into<Element<'a, Message>>, tip: String) -> Element<'a, Message> {
    hint(content, tip, Position::Right)
}

/// The room list's scrollable.
const ROOM_LIST: &str = "room-list";
/// Room left visible beyond an unread room scrolled to (the arrow covers
/// the list's edge).
const SCROLL_MARGIN: f32 = 44.0;

fn unread_id(room: &OwnedRoomId) -> Id {
    Id::from(format!("unread:{room}"))
}

/// Unread rooms scrolled out of the room list's view, above and below:
/// how many, and the offset showing the nearest.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct OutOfView {
    pub above: Option<(usize, f32)>,
    pub below: Option<(usize, f32)>,
}

/// Look where the unread rooms' rows are in the room list.
pub fn find_out_of_view(rooms: &[RoomSummary]) -> Task<OutOfView> {
    let ids = rooms
        .iter()
        .filter(|r| r.kind != RoomKind::Space && is_unread(r))
        .map(|r| unread_id(&r.id))
        .collect();
    iced::advanced::widget::operate(FindUnread {
        ids,
        list: None,
        rows: Vec::new(),
    })
}

/// Scroll the room list to `offset`.
pub fn scroll_to(offset: f32) -> Task<Message> {
    iced::widget::operation::scroll_to(
        ROOM_LIST,
        iced::widget::operation::AbsoluteOffset {
            x: None,
            y: Some(offset.max(0.0)),
        },
    )
}

struct FindUnread {
    ids: Vec<Id>,
    /// The list's bounds and scroll offset.
    list: Option<(Rectangle, Vector)>,
    /// Unread rows, in content positions.
    rows: Vec<Rectangle>,
}

impl Operation<OutOfView> for FindUnread {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<OutOfView>)) {
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
        if id == Some(&Id::new(ROOM_LIST)) {
            self.list = Some((bounds, translation));
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id.is_some_and(|id| self.ids.contains(id)) {
            self.rows.push(bounds);
        }
    }

    fn finish(&self) -> operation::Outcome<OutOfView> {
        operation::Outcome::Some(match self.list {
            Some((list, scrolled)) => out_of_view(list, scrolled.y, &self.rows),
            None => OutOfView::default(),
        })
    }
}

/// Which `rows` (in window positions, unscrolled) of a `list` scrolled by
/// `scrolled` lie wholly above or below its view.
fn out_of_view(list: Rectangle, scrolled: f32, rows: &[Rectangle]) -> OutOfView {
    let (top, bottom) = (scrolled, scrolled + list.height);
    // Row tops and bottoms relative to the list's content.
    let rows = rows
        .iter()
        .map(|row| (row.y - list.y, row.y - list.y + row.height));
    let above: Vec<f32> = rows
        .clone()
        .filter(|(_, end)| *end <= top)
        .map(|(y, _)| y)
        .collect();
    let below: Vec<f32> = rows
        .filter(|(y, _)| *y >= bottom)
        .map(|(_, end)| end)
        .collect();
    let nearest_above = above.iter().copied().reduce(f32::max);
    let nearest_below = below.iter().copied().reduce(f32::min);
    OutOfView {
        above: nearest_above.map(|y| (above.len(), y - SCROLL_MARGIN)),
        below: nearest_below.map(|end| (below.len(), end - list.height + SCROLL_MARGIN)),
    }
}

/// Arrows over the list's edges pointing at unread rooms out of view.
fn out_of_view_arrows<'a>(
    list: Element<'a, Message>,
    out_of_view: OutOfView,
) -> Element<'a, Message> {
    let arrow = |up: bool, (count, offset): (usize, f32)| {
        let label = match count {
            1 => "1 unread".to_owned(),
            n => format!("{n} unread"),
        };
        let tip = if up {
            "Scroll up to unread rooms"
        } else {
            "Scroll down to unread rooms"
        };
        hint(
            button(
                row![
                    icons::arrow(12.0, up, |theme| theme.extended_palette().primary.base.text),
                    text(label).size(12).font(fonts::SEMIBOLD),
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            )
            .padding([3, 10])
            .style(|theme, status| {
                let style = style::primary_button(theme, status);
                button::Style {
                    border: style.border.rounded(12),
                    shadow: iced::Shadow {
                        color: Color::BLACK.scale_alpha(0.2),
                        offset: Vector::new(0.0, 1.0),
                        blur_radius: 4.0,
                    },
                    ..style
                }
            })
            .on_press(Message::ScrollRoomList(offset)),
            tip,
            Position::Right,
        )
    };
    let mut layers = stack![list];
    if let Some(above) = out_of_view.above {
        layers = layers.push(
            container(arrow(true, above))
                .center_x(Length::Fill)
                .padding(Padding::ZERO.top(6)),
        );
    }
    if let Some(below) = out_of_view.below {
        layers = layers.push(
            container(arrow(false, below))
                .center_x(Length::Fill)
                .align_bottom(Length::Fill)
                .padding(Padding::ZERO.bottom(8)),
        );
    }
    layers.into()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use mtrx_core::OwnedUserId;

    use super::*;

    static NO_DRAFTS: BTreeSet<OwnedRoomId> = BTreeSet::new();

    fn room(id: &str, kind: RoomKind, parents: &[&str]) -> RoomSummary {
        RoomSummary {
            id: id.try_into().unwrap(),
            name: id.into(),
            avatar: None,
            topic: None,
            kind,
            invited: false,
            encrypted: false,
            public: false,
            unread_messages: 0,
            unread_mentions: 0,
            unread_notifications: 0,
            last_activity: None,
            parents: parents.iter().map(|p| (*p).try_into().unwrap()).collect(),
            tag: RoomTag::None,
            direct_user: None,
            alias: None,
            joined_members: 0,
            pinned: Vec::new(),
        }
    }

    /// A space with a room below it, a direct chat with one of its members
    /// and a room outside it.
    fn rooms() -> Vec<RoomSummary> {
        vec![
            room("!space:x", RoomKind::Space, &[]),
            room("!inside:x", RoomKind::Room, &["!space:x"]),
            RoomSummary {
                direct_user: Some("@bob:x".try_into().unwrap()),
                ..room("!bob:x", RoomKind::Direct, &[])
            },
            room("!loose:x", RoomKind::Room, &[]),
        ]
    }

    fn unread_in(rooms: &mut [RoomSummary], id: &str) {
        let room = rooms.iter_mut().find(|r| r.id == id).unwrap();
        room.unread_messages = 1;
    }

    fn space() -> OwnedRoomId {
        "!space:x".try_into().unwrap()
    }

    #[test]
    fn an_unread_direct_chat_lights_only_home() {
        let mut rooms = rooms();
        unread_in(&mut rooms, "!bob:x");
        let mut arrange = Arrange::default();
        // Loaded for every top-level space, selected or not.
        assert_eq!(arrange.space_members.due(&rooms), [(space(), 0)]);
        let bob: OwnedUserId = "@bob:x".try_into().unwrap();
        arrange
            .space_members
            .loaded(space(), 0, HashSet::from([bob]));
        // The space lists the chat with its member, unread, but its dot
        // stays off.
        let listed = arrange.listed(&rooms, Some(&space()), &Grouping::default());
        assert!(
            listed
                .iter()
                .any(|r| r.id == "!bob:x" && r.unread_messages > 0)
        );
        assert!(!unread(
            &rooms,
            Some(&space()),
            &arrange,
            &Grouping::default()
        ));
        assert!(unread(&rooms, None, &arrange, &Grouping::default()));
    }

    /// A room with notifications the client doesn't count as unread
    /// messages (e.g. one it couldn't decrypt) still shows them, as the
    /// dock counts them.
    #[test]
    fn notifications_show_without_unread_messages() {
        let mut room = rooms().remove(0);
        room.unread_messages = 0;
        room.unread_notifications = 2;
        assert!(is_unread(&room));
        assert!(unread_count(&room).is_some());
        room.unread_notifications = 0;
        assert!(!is_unread(&room) && unread_count(&room).is_none());
    }

    #[test]
    fn search_finds_what_the_space_or_home_lists() {
        let rooms = rooms();
        let mut arrange = Arrange::default();
        let grouping = Grouping::default();
        let found = |space: Option<&OwnedRoomId>, arrange: &Arrange| -> Vec<String> {
            search(&rooms, space, arrange, &grouping, "")
                .iter()
                .map(|r| r.id.to_string())
                .collect()
        };
        let first = |space: Option<&OwnedRoomId>, arrange: &Arrange, filter| {
            first_match(&rooms, space, arrange, &grouping, filter).map(|r| r.id.to_string())
        };
        assert_eq!(found(Some(&space()), &arrange), ["!inside:x"]);
        assert_eq!(first(Some(&space()), &arrange, " LOO"), None);
        // Home leaves out the space's room.
        assert_eq!(found(None, &arrange), ["!bob:x", "!loose:x"]);
        assert_eq!(first(None, &arrange, " LOO").as_deref(), Some("!loose:x"));
        assert_eq!(first(None, &arrange, "ins"), None);
        // Unless it lists all rooms.
        arrange.categories.at_home.all_rooms = true;
        assert_eq!(found(None, &arrange), ["!bob:x", "!inside:x", "!loose:x"]);
        assert_eq!(first(None, &arrange, "ins").as_deref(), Some("!inside:x"));
    }

    #[test]
    fn home_lists_chats_invites_and_rooms_in_no_space_unless_set() {
        let mut rooms = rooms();
        rooms.push(RoomSummary {
            tag: RoomTag::Favourite,
            ..room("!star:x", RoomKind::Room, &["!space:x"])
        });
        rooms.push(RoomSummary {
            invited: true,
            ..room("!offer:x", RoomKind::Room, &["!space:x"])
        });
        unread_in(&mut rooms, "!inside:x");
        let (avatars, presences) = (Avatars::new(), Presences::default());
        let mut arrange = Arrange::default();
        let grouping = Grouping::default();
        // The rooms of each non-empty section, with its name.
        let home = |arrange: &Arrange| -> Vec<(String, Vec<String>)> {
            let view = list(&rooms, None, "", arrange, &avatars, &presences);
            sections(&view)
                .into_iter()
                .filter(|(_, entries)| !entries.is_empty())
                .map(|(section, entries)| {
                    let name = section_name(arrange, None, &section).to_owned();
                    (name, entries.iter().map(|r| r.id.to_string()).collect())
                })
                .collect()
        };
        let section = |name: &str, ids: &[&str]| -> (String, Vec<String>) {
            (name.into(), ids.iter().map(|id| id.to_string()).collect())
        };

        assert_eq!(
            home(&arrange),
            [
                section("Invites", &["!offer:x"]),
                section("Direct messages", &["!bob:x"]),
                section("Rooms with no space", &["!loose:x"]),
            ]
        );
        // The space's unread room does not light Home.
        assert!(!unread(&rooms, None, &arrange, &grouping));

        arrange.categories.at_home.favourites = true;
        assert_eq!(home(&arrange)[2], section("Favourites", &["!star:x"]));
        assert!(!unread(&rooms, None, &arrange, &grouping));

        arrange.categories.at_home.all_rooms = true;
        assert_eq!(
            home(&arrange),
            [
                section("Invites", &["!offer:x"]),
                section("Direct messages", &["!bob:x"]),
                section("Favourites", &["!star:x"]),
                section("Rooms", &["!inside:x", "!loose:x"]),
            ]
        );
        assert!(unread(&rooms, None, &arrange, &grouping));
        // A space's rooms are always just "Rooms".
        arrange.categories.at_home.all_rooms = false;
        assert_eq!(
            section_name(&arrange, Some(&space()), &Section::Rooms),
            "Rooms"
        );
    }

    #[test]
    fn an_unread_room_in_a_category_of_the_space_lights_it() {
        let mut rooms = rooms();
        unread_in(&mut rooms, "!loose:x");
        let mut arrange = Arrange::default();
        assert!(!unread(
            &rooms,
            Some(&space()),
            &arrange,
            &Grouping::default()
        ));
        let layout = arrange.categories.layout_mut(Some(&space()));
        let category = layout.add_category("Mine");
        let loose: OwnedRoomId = "!loose:x".try_into().unwrap();
        layout.place(&loose, Some(&category));
        assert!(unread(
            &rooms,
            Some(&space()),
            &arrange,
            &Grouping::default()
        ));
    }

    /// The room list in `space` (Home for `None`), searching `filter`.
    fn list<'a>(
        rooms: &'a [RoomSummary],
        space: Option<&'a OwnedRoomId>,
        filter: &'a str,
        arrange: &'a Arrange,
        avatars: &'a Avatars,
        presences: &'a Presences,
    ) -> RoomListView<'a> {
        RoomListView {
            rooms,
            selected_space: space,
            open: None,
            loaded: true,
            avatars,
            scheduled: &[],
            drafts: &NO_DRAFTS,
            add_menu: false,
            filter,
            arrange,
            grouping: Grouping::default(),
            out_of_view: OutOfView::default(),
            width: 260.0,
            tint: Tint::default(),
            room_pictures: false,
            notes: None,
            presences,
            space_rooms: Box::leak(Box::default()),
            space_topic: Box::leak(Box::default()),
            call_services: &[],
            topic_width: 440.0,
        }
    }

    #[test]
    fn follows_the_people_of_the_direct_chats_listed() {
        let own: OwnedUserId = "@me:x".try_into().unwrap();
        let mut rooms = rooms();
        rooms.push(RoomSummary {
            direct_user: Some("@carol:x".try_into().unwrap()),
            ..room("!carol:x", RoomKind::Direct, &[])
        });
        // The Notes to self, not pinned, are listed but not followed.
        rooms.push(RoomSummary {
            direct_user: Some(own.clone()),
            ..room("!notes:x", RoomKind::Direct, &[])
        });
        let (avatars, presences) = (Avatars::new(), Presences::default());
        let mut arrange = Arrange::default();
        let people = |space, filter, arrange: &Arrange| {
            let view = list(&rooms, space, filter, arrange, &avatars, &presences);
            let mut people: Vec<String> = direct_users(&view, &own)
                .into_iter()
                .map(|u| u.to_string())
                .collect();
            people.sort();
            people
        };
        assert_eq!(people(None, "", &arrange), ["@bob:x", "@carol:x"]);
        assert_eq!(people(None, "car", &arrange), ["@carol:x"]);
        let space = space();
        assert!(people(Some(&space), "", &arrange).is_empty());
        assert_eq!(arrange.space_members.due(&rooms), [(space.clone(), 0)]);
        let bob: OwnedUserId = "@bob:x".try_into().unwrap();
        arrange
            .space_members
            .loaded(space.clone(), 0, HashSet::from([bob]));
        assert_eq!(people(Some(&space), "", &arrange), ["@bob:x"]);
    }

    #[test]
    fn keys_go_to_the_rooms_in_the_order_shown() {
        let mut rooms = rooms();
        rooms.push(RoomSummary {
            tag: RoomTag::Favourite,
            ..room("!fav:x", RoomKind::Room, &[])
        });
        rooms.push(room("!notes:x", RoomKind::Direct, &[]));
        let (avatars, presences) = (Avatars::new(), Presences::default());
        let mut arrange = Arrange::default();
        let ids = |view: &RoomListView| -> Vec<String> {
            rows(view).iter().map(|r| r.id.to_string()).collect()
        };
        let id = |id: &str| -> OwnedRoomId { id.try_into().unwrap() };

        // What Home shows: not the space's room.
        let home = list(&rooms, None, "", &arrange, &avatars, &presences);
        assert_eq!(ids(&home), ["!bob:x", "!notes:x", "!fav:x", "!loose:x"]);
        assert_eq!(
            go_to(&home, Go::Row(2))
                .map(|r| r.id.to_string())
                .as_deref(),
            Some("!fav:x")
        );
        arrange.categories.at_home.all_rooms = true;

        // Sections in their order, the pinned Notes first.
        let mut home = list(&rooms, None, "", &arrange, &avatars, &presences);
        home.notes = Some(Notes::Room(&rooms[5]));
        assert_eq!(
            ids(&home),
            ["!notes:x", "!bob:x", "!fav:x", "!inside:x", "!loose:x"]
        );
        let to = |view: &RoomListView, go| go_to(view, go).map(|r| r.id.to_string());
        // From outside the list, the first or last.
        assert_eq!(to(&home, Go::Below).as_deref(), Some("!notes:x"));
        assert_eq!(to(&home, Go::Above).as_deref(), Some("!loose:x"));
        let inside = id("!inside:x");
        home.open = Some(&inside);
        assert_eq!(to(&home, Go::Above).as_deref(), Some("!fav:x"));
        assert_eq!(to(&home, Go::Below).as_deref(), Some("!loose:x"));
        assert_eq!(to(&home, Go::Row(1)).as_deref(), Some("!bob:x"));
        assert_eq!(to(&home, Go::Row(8)), None);
        // Not past the ends.
        let loose = id("!loose:x");
        home.open = Some(&loose);
        assert_eq!(to(&home, Go::Below), None);

        // A collapsed section shows only the open room and unread ones. The
        // Notes not pinned are a direct chat.
        (arrange.categories.layout_mut(None)).toggle_collapsed(&Section::Rooms);
        unread_in(&mut rooms, "!loose:x");
        let mut home = list(&rooms, None, "", &arrange, &avatars, &presences);
        assert_eq!(ids(&home), ["!bob:x", "!notes:x", "!fav:x", "!loose:x"]);
        home.open = Some(&inside);
        assert_eq!(
            ids(&home),
            ["!bob:x", "!notes:x", "!fav:x", "!inside:x", "!loose:x"]
        );

        // While searching, what it found.
        let found = list(&rooms, None, "o", &arrange, &avatars, &presences);
        assert_eq!(ids(&found), ["!bob:x", "!loose:x", "!notes:x"]);
    }

    #[test]
    fn keys_for_going_to_rooms() {
        use iced::keyboard::key::{Code, NativeCode, Physical};

        let none = Physical::Unidentified(NativeCode::Unidentified);
        let go = |key: Key, physical, modifiers| Go::from_key(&key, physical, modifiers);
        let command_alt = Modifiers::COMMAND | Modifiers::ALT;
        let arrow = |named| Key::Named(named);
        assert_eq!(
            go(arrow(Named::ArrowUp), none, command_alt),
            Some(Go::Above)
        );
        assert_eq!(
            go(arrow(Named::ArrowDown), none, command_alt),
            Some(Go::Below)
        );
        assert_eq!(go(arrow(Named::ArrowDown), none, Modifiers::ALT), None);
        let tab = arrow(Named::Tab);
        assert_eq!(go(tab.clone(), none, Modifiers::CTRL), Some(Go::Below));
        let back = Modifiers::CTRL | Modifiers::SHIFT;
        assert_eq!(go(tab.clone(), none, back), Some(Go::Above));
        assert_eq!(go(tab, none, Modifiers::empty()), None);
        let digit = |c: &str| Key::Character(c.into());
        assert_eq!(go(digit("1"), none, Modifiers::COMMAND), Some(Go::Row(0)));
        assert_eq!(go(digit("9"), none, Modifiers::COMMAND), Some(Go::Row(8)));
        assert_eq!(go(digit("0"), none, Modifiers::COMMAND), None);
        assert_eq!(go(digit("1"), none, Modifiers::empty()), None);
        // On AZERTY ⌘ and the key of 1 gives "&".
        let one = Physical::Code(Code::Digit1);
        assert_eq!(go(digit("&"), one, Modifiers::COMMAND), Some(Go::Row(0)));
    }

    #[test]
    fn finds_unread_rooms_out_of_view() {
        // A list at y 100, 300 high, scrolled down by 200: it shows
        // content 200..500.
        let list = Rectangle::new(iced::Point::new(0.0, 100.0), iced::Size::new(260.0, 300.0));
        let row = |y: f32| {
            Rectangle::new(
                iced::Point::new(0.0, 100.0 + y),
                iced::Size::new(260.0, 32.0),
            )
        };
        let rows = [
            row(0.0),
            row(150.0),
            row(190.0),
            row(400.0),
            row(520.0),
            row(700.0),
        ];
        let found = out_of_view(list, 200.0, &rows);
        // 0 and 150 are above (190 shows partly); the nearest is 150.
        assert_eq!(found.above, Some((2, 150.0 - SCROLL_MARGIN)));
        // 520 and 700 are below; scrolling shows 520's bottom (552).
        assert_eq!(found.below, Some((2, 552.0 - 300.0 + SCROLL_MARGIN)));
        assert_eq!(
            out_of_view(list, 200.0, &[row(300.0)]),
            OutOfView::default()
        );
    }
}
