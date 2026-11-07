//! How an open room shows: its header, timeline and composer.

use std::{collections::HashMap, rc::Rc, time::SystemTime};

use iced::{
    Alignment, Element, Length, Padding, Theme, mouse,
    widget::{
        button, column, container, image, mouse_area, row, rule, scrollable, space, span, stack,
        text,
    },
};
use mtrx_core::{
    CallService, Event, EventContent, MessageKind, MessageRow, OwnedRoomId, OwnedUserId, RoomKind,
    RoomSummary, Row, Topic, UserId,
};

use crate::{
    actions::{self, Editing, InlineEdit, PickerAt, Target, act},
    app::{self, Popup},
    avatar::{self, Avatars, Shape},
    calls::{self},
    context_menu::context_menu,
    drag_scroll::drag_scroll,
    ellipsis::ellipsis,
    emoji,
    find::Find,
    fonts,
    hint::{Position, hint},
    icons, info,
    math::{self, Math, Rendered},
    media::{self, Previews},
    members,
    menu::{self, Width},
    message,
    message_field::{self, Field},
    message_menu::{self, MenuAt},
    popover::{Edge, Side, popover},
    presence::{self, Presences},
    rows::{self, Part, time},
    selectable::{self, image::selectable_image, text::selectable},
    selection::{self, Bounds, Point, Shown},
    send_later,
    status::{self},
    style::{
        self, GUTTER_SPACING, HEADER_AVATAR, MESSAGE_PADDING, SENDER_AVATAR, TEXT_MARGIN,
        TIMELINE_PADDING, TOOLBAR_HEIGHT,
    },
    topic,
    want::wanting,
};

use super::{
    COMPOSER_INSET, Load, Message, Room, Sending, TEXT_INDENT, TIMELINE, TOPIC_SHARE, first_line,
    row_id,
};

impl Room {
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
        let mention_room = summary.is_some_and(|s| s.rights.mention_room);
        let fields = message_field::Context {
            mentions: self.mentionable(look.own_user, mention_room),
            avatars: look.avatars,
            frequent: look.frequent_emoji,
            popup,
        };
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
            fields,
        };

        let mut timeline = column![].padding([8.0, TIMELINE_PADDING]);
        let all_shown = self.shown >= self.rows.len();
        let paginating = self.tasks.running(&Load::Paginate);
        if self.tasks.running(&Load::Reload) {
            timeline = timeline.push(notice("Reloading…"));
        } else if !(self.at_start() && all_shown) && (paginating || !all_shown) {
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
        let composer = self.composer.view(
            message_field::Look {
                placeholder: Some("Message…  (Markdown, $…$ for math; Shift+Enter for a new line)"),
                cancellable: self.compose.cancellable(),
                edit_last: true,
                side: Side::Above,
                compact: false,
                padding: Padding::new(10.0).right(COMPOSER_INSET).top(
                    if self.attachments.is_empty() {
                        10.0
                    } else {
                        media::CHIP_HEIGHT + 18.0
                    },
                ),
                max_height: 200.0,
                tools: Some(attach),
            },
            fields,
            move |message| Message::Field(Field::Composer, message).to(id),
        );
        // The attached files sit inside the field, above the text.
        let mut composer = stack![composer];
        if !self.attachments.is_empty() {
            composer = composer.push(
                container(media::attached_view(&self.attachments, |index| {
                    Message::Unattach(index).to(id)
                }))
                .padding(Padding::new(8.0).right(COMPOSER_INSET)),
            );
        }
        let composer_empty = self.composer.is_empty() && self.attachments.is_empty();
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
        .on_dismiss(app::Message::ClosePopup);

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
                fields,
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
                summary.is_some_and(|s| s.rights.invite),
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
        let topic = Topic::parse(summary.and_then(|s| s.topic.as_deref()).unwrap_or_default());
        let rights = summary.map(|s| s.rights).unwrap_or_default();
        let can_edit = rights.change_topic;
        // A direct chat's person, as long as their presence is known.
        let presence = summary
            .filter(|s| s.kind == RoomKind::Direct && !s.is_notes(look.own_user))
            .and_then(|s| s.direct_user.as_ref())
            .and_then(|user| look.presences.get(&user.id));
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
                    .on_press(info::show(info.clone())),
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
                        .on_press(app::Message::Pane(app::pane::Message::ToggleMembers)),
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
                    .on_press(info::show(info::Target::Room(self.id.clone()))),
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
            let topic = self
                .topic
                .view(&topic, look.call_services, can_edit, move |message| {
                    Message::Topic(message).to(&self.id)
                });
            let mut content = column![topic].spacing(14);
            // Not while the topic is edited.
            if self.topic.editor.is_none() {
                content = content.push(self.pins.view(&self.id, pinned, rights.pin));
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
    /// The row whose unsent message is offered to retry or take back.
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
    /// What the message edited shows of the room and the window.
    fields: message_field::Context<'a, 'b>,
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
                    let key = Math::key(&m.document, params);
                    match math.get(&key) {
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
                        // Typeset once rendered; as text until then, or if
                        // it failed.
                        _ => {
                            as_text = true;
                            let want = math.wanted(&key).then(|| {
                                app::Message::Media(media::Message::MathWanted(math::Request {
                                    document: m.document.clone(),
                                    params: *params,
                                }))
                            });
                            wanting(
                                message::view(
                                    &m.document,
                                    theme,
                                    context.own_user,
                                    links(),
                                    select(),
                                    m.edited,
                                ),
                                want,
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
                        event.event_id.as_ref(),
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
        Some(edit) => edit.view(context.room, context.fields),
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
    // Its entries are made only while it is open.
    let has_menu = message_menu::offers(shown, math_shown);
    let menu_view = |at| {
        let entries = match menu_at {
            Some(MenuAt::Link(url)) if at == MenuAt::Cursor => {
                message_menu::link_entries(context.room, url)
            }
            Some(open) if *open == at => {
                message_menu::entries(context.room, shown, math_shown, in_selection)
            }
            _ => return None,
        };
        Some(menu::view(entries, Width::Menu))
    };
    if context.confirm_delete == Some(event.id.as_str()) {
        item = item.push(actions::confirm_delete(context.room, event));
    }

    // The avatar starts a group; follow-ups show their time there on hover.
    let hovered = context.hovered == Some(event.id.as_str());
    let gutter: Element<'a, app::Message> = if with_header {
        mouse_area(context.avatars.view(
            event.sender.id.as_str(),
            &event.sender.display_name(),
            event.sender.avatar.as_deref(),
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
            && (actions::has_tools(&shown.actions) || has_menu)
            && edit.is_none();
    let toolbar = toolbar_shown.then(|| {
        let target = message.and_then(|m| {
            let preview = first_line(&m.document.to_plain_text());
            Target::of(event, event.sender.display_name(), preview)
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
        let menu = has_menu.then(|| actions::Menu(menu_view(MenuAt::Toolbar)));
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
    let row: Element<'a, app::Message> = if has_menu {
        context_menu(
            row,
            menu_view(MenuAt::Cursor),
            act(
                context.room,
                actions::Message::OpenMenu(event.id.clone(), MenuAt::Cursor),
            ),
        )
        .on_dismiss(app::Message::ClosePopup)
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
        name = name.color(avatar::name_color(event.sender.id.as_str(), context.theme));
    }
    let time = span(units[1].text.clone());
    row![
        select
            .apply(selectable(vec![name], 0), 0)
            .on_click(open.clone()),
        select
            .apply(selectable(vec![time], 1).size(11).style(text::secondary), 1)
            .on_click(open),
    ]
    .spacing(8)
    .align_y(Alignment::End)
    .into()
}

/// Opens the sender's info card.
fn sender_info(event: &Event) -> app::Message {
    info::show(info::Target::User(event.sender.clone()))
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
) -> selectable::text::Selectable<'a, app::Message> {
    let line = part.units(&Shown::default()).remove(0).text;
    let unit = selectable(vec![span(line)], 0)
        .size(size)
        .style(text::secondary);
    context.select(&part.key).apply(unit, 0)
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
