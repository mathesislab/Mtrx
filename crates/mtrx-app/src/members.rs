//! The member list beside an open room.

use iced::{
    Alignment, Element, Length,
    widget::{button, column, container, hover, row, space, stack, text},
};
use mtrx_core::{Member, OwnedRoomId, OwnedUserId};

use crate::{
    app::Message,
    avatar::{Avatars, Shape},
    context_menu,
    dialog::{Kind, LevelChange},
    ellipsis::ellipsis,
    fonts, frame,
    hint::{Position, hint},
    icons,
    popover::{Edge, Side, popover},
    presence::{self, Presences},
    style,
};

const WIDTH: f32 = 250.0;
const AVATAR: f32 = 24.0;
/// The role's column, and the ⋯ one beside it.
const ROLE: f32 = 70.0;
const MORE: f32 = 26.0;
const MENU_WIDTH: f32 = 190.0;

/// A member's menu: their role, or the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    Role,
    More,
}

/// The open menu: of whom, in which room.
pub type MenuKey = (OwnedRoomId, OwnedUserId, Menu);

/// What the member rows need besides the members.
#[derive(Clone, Copy)]
pub struct Shown<'a> {
    pub room: &'a OwnedRoomId,
    pub avatars: &'a Avatars,
    pub presences: &'a Presences,
    /// The member whose menu is open, in any room.
    pub menu: Option<&'a MenuKey>,
}

/// The panel: the members once loaded (`list`), or why loading failed.
pub fn view<'a>(
    list: Option<&'a Result<Vec<Member>, String>>,
    room_name: &str,
    shown: Shown<'_>,
) -> Element<'a, Message> {
    let room = shown.room;
    let panel = column![
        row![
            text("Members")
                .size(14)
                .font(fonts::SEMIBOLD)
                .width(Length::Fill),
            button(text("Invite…").size(12))
                .padding([2, 8])
                .style(style::soft_button)
                .on_press(Message::OpenDialog(Kind::Invite {
                    room: room.clone(),
                    name: room_name.to_owned(),
                })),
            button(text("×").size(14))
                .padding([0, 6])
                .style(style::text_button)
                .on_press(Message::ToggleMembers),
        ]
        .spacing(4)
        .align_y(Alignment::Center)
    ]
    .spacing(2)
    .padding(8);
    container(frame::scroll(panel.push(rows(list, shown))))
        .width(WIDTH)
        .height(Length::Fill)
        .style(style::panel)
        .into()
}

/// The members once loaded (`list`), joined then invited, or why loading
/// failed; also in a space's overview.
pub fn rows<'a>(
    list: Option<&'a Result<Vec<Member>, String>>,
    shown: Shown<'_>,
) -> Element<'a, Message> {
    let mut rows = column![].spacing(2);
    match list {
        None => rows = rows.push(text("Loading…").size(12).style(text::secondary)),
        Some(Err(e)) => rows = rows.push(text(e).size(12).style(text::danger)),
        Some(Ok(members)) => {
            let (joined, invited): (Vec<_>, Vec<_>) = members.iter().partition(|m| !m.invited);
            let common = Common {
                more: members.iter().any(|m| m.manage.remove || m.manage.ban),
            };
            rows = rows.push(heading(format!("Joined · {}", joined.len())));
            for member in joined {
                rows = rows.push(entry(member, shown, common));
            }
            if !invited.is_empty() {
                rows = rows.push(heading(format!("Invited · {}", invited.len())));
                for member in invited {
                    rows = rows.push(entry(member, shown, common));
                }
            }
        }
    }
    rows.into()
}

/// Pictures of the listed members, with their size.
pub fn avatars(members: &[Member]) -> impl Iterator<Item = (&mtrx_core::MxcUri, f32)> {
    avatars_of(members)
}

/// Pictures of `members` at the list's size, also for mentions.
pub fn avatars_of<'a>(
    members: impl IntoIterator<Item = &'a Member>,
) -> impl Iterator<Item = (&'a mtrx_core::MxcUri, f32)> {
    members
        .into_iter()
        .filter_map(|m| Some((m.user.avatar.as_deref()?, AVATAR)))
}

fn heading<'a>(label: String) -> Element<'a, Message> {
    container(
        text(label.to_uppercase())
            .size(11)
            .font(fonts::SEMIBOLD)
            .style(text::secondary),
    )
    .padding([8, 4])
    .into()
}

/// What the rows have in common.
#[derive(Clone, Copy)]
struct Common {
    /// Some row has a ⋯ menu: all keep room for it, to line up.
    more: bool,
}

/// A member, with their presence and role; clicking shows who they are.
/// Hovered, the role becomes a menu to change it and ⋯ one to remove or
/// ban them, where we may.
fn entry<'a>(member: &'a Member, shown: Shown<'_>, common: Common) -> Element<'a, Message> {
    let user = &member.user;
    let name = user.display_name();
    let avatar = shown.avatars.view(
        user.id.as_str(),
        &name,
        user.avatar.as_deref(),
        AVATAR,
        Shape::Circle,
    );
    let presence = shown.presences.get(&user.id);
    let role = member.label();
    let line = row![
        presence::with_dot(avatar, presence, AVATAR, |theme| {
            theme.extended_palette().background.weak.color
        }),
        ellipsis(name.clone()).size(13).width(Length::Fill),
        container(
            text(role.clone().unwrap_or_default())
                .size(11)
                .style(text::secondary)
        )
        .align_right(ROLE),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let menu_of = |menu| (shown.room.clone(), user.id.clone(), menu);
    let opened = |menu| shown.menu == Some(&menu_of(menu));
    // An open menu shows alone, without the row's hint.
    let menu_open = opened(Menu::Role) || opened(Menu::More);
    let who: Element<'a, Message> = if member.own {
        container(line).width(Length::Fill).padding([3, 4]).into()
    } else {
        let card = button(line)
            .width(Length::Fill)
            .padding([3, 4])
            .style(style::list_item(menu_open))
            .on_press(Message::ShowInfo(crate::info::Target::User(user.clone())));
        if menu_open {
            card.into()
        } else {
            hint(card, user.id.to_string(), Position::Left)
        }
    };
    let slot = |content: Element<'a, Message>| -> Element<'a, Message> {
        container(content).center_x(MORE).into()
    };
    let mut base = row![who].spacing(2).align_y(Alignment::Center);
    if common.more {
        base = base.push(slot(space().into()));
    }
    if !member.manageable() {
        return base.into();
    }
    let manage = &member.manage;
    let toggle = |menu| Message::MemberMenu((!opened(menu)).then(|| menu_of(menu)));
    let chip: Element<'a, Message> = match manage.up_to {
        Some(_) => {
            let open = opened(Menu::Role);
            let label = role.unwrap_or_else(|| "Member".into());
            popover(
                button(
                    row![
                        text(label).size(11),
                        icons::chevron(9.0, open, style::on_secondary)
                    ]
                    .spacing(3)
                    .align_y(Alignment::Center),
                )
                .padding([1, 6])
                .style(style::secondary_button)
                .on_press(toggle(Menu::Role)),
                open.then(|| role_menu(member, shown.room)),
                Side::Below,
                Edge::Right,
            )
            .on_dismiss(Message::MemberMenu(None))
            .into()
        }
        None => space().into(),
    };
    let more: Element<'a, Message> = if manage.remove || manage.ban {
        let open = opened(Menu::More);
        popover(
            button(text("⋯").size(14))
                .padding([0, 6])
                .style(style::list_item(open))
                .on_press(toggle(Menu::More)),
            open.then(|| more_menu(member, &name, shown.room)),
            Side::Below,
            Edge::Right,
        )
        .on_dismiss(Message::MemberMenu(None))
        .into()
    } else {
        space().into()
    };
    // Over the row, lined up with its role and the room kept for ⋯.
    let mut top = row![
        row![space::horizontal(), container(chip).align_right(ROLE)]
            .padding([3, 4])
            .height(Length::Fill)
            .align_y(Alignment::Center)
    ]
    .spacing(2)
    .height(Length::Fill)
    .align_y(Alignment::Center);
    if common.more {
        top = top.push(slot(more));
    }
    // Hovered, the menus' buttons show; one open stays, whatever the
    // cursor does.
    if menu_open {
        stack![base, top].into()
    } else {
        hover(base, top)
    }
}

/// The levels we may give a member, theirs ticked, and a level of our
/// choosing. Asks first where it can't be undone: our own, or theirs as
/// high as ours.
fn role_menu<'a>(member: &Member, room: &OwnedRoomId) -> Element<'a, Message> {
    let change = |asked| LevelChange {
        room: room.clone(),
        member: member.clone(),
        asked,
    };
    let mut items = column![].spacing(1);
    for choice in member.level_choices() {
        let on_press = (!choice.current).then(|| match choice.confirmation {
            Some(asked) => Message::OpenDialog(Kind::Level(change(Some((choice.level, asked))))),
            None => Message::SetLevel(room.clone(), member.user.id.clone(), choice.level),
        });
        items = items.push(role_item(
            choice.title,
            choice.level,
            choice.current,
            on_press,
        ));
    }
    items = items.push(
        button(text("Custom level…").size(13))
            .width(Length::Fill)
            .padding([4, 10])
            .style(style::list_item(false))
            .on_press(Message::OpenDialog(Kind::Level(change(None)))),
    );
    container(items)
        .width(MENU_WIDTH)
        .padding(4)
        .style(style::toolbar)
        .into()
}

/// A role in [`role_menu`]: ticked if theirs.
fn role_item<'a>(
    label: String,
    level: i64,
    current: bool,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let tick: Element<'a, Message> = if current {
        icons::check(12.0, |theme| theme.extended_palette().background.base.text)
    } else {
        space().width(12).into()
    };
    button(
        row![
            tick,
            text(label).size(13).width(Length::Fill),
            text(level.to_string()).size(12).style(text::secondary),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([4, 10])
    .style(style::list_item(false))
    .on_press_maybe(on_press)
    .into()
}

/// Remove or ban a member, as far as we may; both ask first.
fn more_menu<'a>(member: &Member, name: &str, room: &OwnedRoomId) -> Element<'a, Message> {
    let mut items = Vec::new();
    let remove = |ban| {
        Message::OpenDialog(Kind::RemoveMember {
            room: room.clone(),
            user: member.user.id.clone(),
            name: name.to_owned(),
            ban,
        })
    };
    if member.manage.remove {
        items.push(("Remove…".to_owned(), remove(false)));
    }
    if member.manage.ban {
        items.push(("Ban…".to_owned(), remove(true)));
    }
    context_menu::menu(items)
}
