//! Calls through links: call cards in the timeline, and the room header's
//! split button: Join, for the room's call (the first in its topic), or
//! Start call; and the menu of the room's calls and links, which are
//! dragged to other places, of calls with other services, and to edit
//! them.

use iced::{
    Alignment, Element, Length, Theme,
    widget::{button, column, container, row, rule, text},
};
use mtrx_core::{CallService, OwnedRoomId, RoomLink, Topic, recognize_call_link};

use crate::{
    app,
    ellipsis::ellipsis,
    fonts,
    hint::{Position, hint, hint_with},
    icons,
    popover::{Edge, Side, popover},
    reorder::{self, Reorder},
    room, style, topic,
};

const MENU_WIDTH: f32 = 300.0;
/// The split button's height, both halves the same whatever they show.
const SPLIT_HEIGHT: f32 = 27.0;

#[derive(Debug, Clone)]
pub enum Message {
    /// The call service of the room's nearest space with one.
    SpaceService(Option<CallService>),
    OpenMenu,
    Close,
    /// Open a call or link from the menu.
    Open(String),
    /// Post a new call link of this service in the room.
    Start(CallService),
    Started(Result<(), String>),
    /// Drag the room's calls, the first being the room's.
    Calls(reorder::Message),
    Links(reorder::Message),
}

/// What starting a call with `service` does.
fn start_hint(service: &CallService) -> String {
    format!(
        "Start a {} call now: posts a new link in the chat",
        service.name
    )
}

/// Call state of the open room.
pub struct RoomCalls {
    room: OwnedRoomId,
    pub space_service: Option<CallService>,
    pub menu: bool,
    /// A call is being started.
    pub busy: bool,
    /// Starting a call failed.
    pub error: Option<String>,
    pub calls: Reorder,
    pub links: Reorder,
}

impl RoomCalls {
    pub fn new(room: OwnedRoomId) -> Self {
        Self {
            room,
            space_service: None,
            menu: false,
            busy: false,
            error: None,
            calls: Reorder::default(),
            links: Reorder::default(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.menu
    }

    pub fn close(&mut self) {
        self.menu = false;
        self.error = None;
        self.calls = Reorder::default();
        self.links = Reorder::default();
    }

    fn message(&self, message: Message) -> app::Message {
        room::Message::Calls(message).to(&self.room)
    }

    /// The service "Start call" uses: the space's, else the first own one.
    pub fn default_service<'a>(&'a self, own: &'a [CallService]) -> Option<&'a CallService> {
        self.space_service.as_ref().or(own.first())
    }

    /// Services offered in the start menu: the space's first.
    fn services<'a>(&'a self, own: &'a [CallService]) -> Vec<&'a CallService> {
        let mut all: Vec<_> = self.space_service.iter().collect();
        all.extend(
            own.iter()
                .filter(|s| Some(*s) != self.space_service.as_ref()),
        );
        all
    }

    /// The split button at the right of the room header: Join the room's
    /// call, or Start call; and how many calls and links the room has,
    /// opening the menu. `can_edit`: whether we may change the topic.
    pub fn header_controls<'a>(
        &'a self,
        topic: &Topic,
        own: &'a [CallService],
        can_edit: bool,
    ) -> Element<'a, app::Message> {
        let open = self.is_open();
        // Hints would cover the open menu.
        let tip = |control: Element<'a, app::Message>, label: Element<'a, app::Message>| {
            if open {
                control
            } else {
                hint_with(control, label, Position::Bottom)
            }
        };
        let main = match topic.default_call() {
            Some(call) => {
                let mut name = format!("Join {}", call.name(own));
                if let Some(passcode) = call.passcode() {
                    name.push_str(&format!(", passcode {passcode}"));
                }
                tip(
                    button(
                        row![icons::camera(14.0, on_primary), text("Join").size(13)]
                            .spacing(6)
                            .height(Length::Fill)
                            .align_y(Alignment::Center),
                    )
                    .height(Length::Fill)
                    .padding([0, 12])
                    .style(style::split_button(style::primary_button, true))
                    .on_press(app::Message::LinkClicked(call.url.to_string()))
                    .into(),
                    column![
                        text(name).size(12),
                        text(call.url.to_string()).size(11).style(text::secondary),
                    ]
                    .spacing(2)
                    .into(),
                )
            }
            None => {
                let service = self.default_service(own);
                let label = match service {
                    Some(service) => start_hint(service),
                    None => "Add a call service in Settings to start calls".to_owned(),
                };
                tip(
                    button(container(text("Start call").size(13)).center_y(Length::Fill))
                        .height(Length::Fill)
                        .padding([0, 10])
                        .style(style::split_button(style::secondary_button, true))
                        .on_press_maybe(
                            service
                                .filter(|_| !self.busy)
                                .map(|s| self.message(Message::Start(s.clone()))),
                        )
                        .into(),
                    text(label).size(12).into(),
                )
            }
        };
        let calls = topic.calls().count();
        let links = topic.links.len() - calls;
        let mut counts = row![].spacing(4).align_y(Alignment::Center);
        if calls > 0 {
            counts = counts.push(icons::camera(13.0, style::on_secondary));
            counts = counts.push(text(calls.to_string()).size(13));
        }
        if calls > 0 && links > 0 {
            counts = counts.push(text("·").size(13));
        }
        if links > 0 {
            counts = counts.push(icons::link(13.0, style::on_secondary));
            counts = counts.push(text(links.to_string()).size(13));
        }
        counts = counts.push(icons::chevron(10.0, true, style::on_secondary));
        // While open, a click on it closes the menu as any outside click.
        let toggle = tip(
            button(counts.height(Length::Fill))
                .height(Length::Fill)
                .padding([0, 8])
                .style(style::split_button(style::secondary_button, false))
                .on_press_maybe((!open).then(|| self.message(Message::OpenMenu)))
                .into(),
            text("The room's calls and links, and other call services")
                .size(12)
                .into(),
        );
        let menu = open.then(|| self.menu_view(topic, own, can_edit));
        popover(
            row![main, toggle].spacing(1).height(SPLIT_HEIGHT),
            menu,
            Side::Below,
            Edge::Right,
        )
        .on_dismiss(self.message(Message::Close))
        .into()
    }

    fn menu_view<'a>(
        &'a self,
        topic: &Topic,
        own: &'a [CallService],
        can_edit: bool,
    ) -> Element<'a, app::Message> {
        let item = |label: String, message: Option<Message>| {
            button(ellipsis(label).size(13))
                .width(Length::Fill)
                .padding([4, 10])
                .style(style::list_item(false))
                .on_press_maybe(message.filter(|_| !self.busy).map(|m| self.message(m)))
        };
        let heading =
            |label| container(text(label).size(12).style(text::secondary)).padding([4, 10]);
        let separator = || container(rule::horizontal(1).style(style::separator)).padding([4, 0]);
        let mut menu = column![].spacing(1);
        let (calls, links): (Vec<&RoomLink>, Vec<&RoomLink>) =
            topic.links.iter().partition(|l| l.is_call());
        for (group, order, message) in [
            (calls, &self.calls, Message::Calls as fn(_) -> _),
            (links, &self.links, Message::Links),
        ] {
            let Some(first) = group.first() else {
                continue;
            };
            let call = first.is_call();
            menu = menu.push(heading(if call { "Calls" } else { "Links" }));
            let many = group.len() > 1;
            let rows = (group.iter().enumerate())
                .map(|(at, link)| self.link_item(link, own, call && many && at == 0))
                .collect();
            let room = self.room.clone();
            menu = menu.push(reorder::list(order, rows, can_edit && many, move |m| {
                room::Message::Calls(message(m)).to(&room)
            }));
        }
        if !topic.links.is_empty() {
            menu = menu.push(separator());
        }
        menu = menu.push(heading("Start a new call with"));
        let services = self.services(own);
        if services.is_empty() {
            menu = menu.push(heading("No call services yet: add one in Settings"));
        }
        for service in services {
            let label = if Some(service) == self.space_service.as_ref() {
                format!("{} (space)", service.name)
            } else {
                service.name.clone()
            };
            menu = menu.push(hint(
                item(label, Some(Message::Start(service.clone()))),
                start_hint(service),
                Position::Left,
            ));
        }
        if can_edit {
            menu = menu.push(separator()).push(
                button(ellipsis("Edit room links…").size(13))
                    .width(Length::Fill)
                    .padding([4, 10])
                    .style(style::list_item(false))
                    .on_press(room::Message::Topic(topic::Message::Edit).to(&self.room)),
            );
        }
        if let Some(error) = &self.error {
            menu = menu.push(container(text(error).size(12).style(text::danger)).padding([4, 10]));
        }
        container(menu)
            .width(MENU_WIDTH)
            .padding(4)
            .style(style::toolbar)
            .into()
    }

    /// A call or link in the menu, opening it; marked as the room's call
    /// when `default`.
    fn link_item<'a>(
        &self,
        link: &RoomLink,
        own: &[CallService],
        default: bool,
    ) -> Element<'a, app::Message> {
        let mut label = row![
            topic::kind_icon(link.is_call()),
            text(link.name(own)).size(13),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let detail = match link.passcode() {
            Some(passcode) => format!("passcode {passcode}"),
            None => link.host(),
        };
        label = label.push(
            ellipsis(detail)
                .size(12)
                .style(text::secondary)
                .width(Length::Fill),
        );
        if default {
            label = label.push(text("default").size(11).style(text::secondary));
        }
        hint(
            button(label)
                .width(Length::Fill)
                .padding([4, 8])
                .style(style::list_item(false))
                .on_press(self.message(Message::Open(link.url.to_string()))),
            link.url.to_string(),
            Position::Left,
        )
    }
}

/// Cards for the meeting links of a message, with a Join button each.
pub fn cards<'a>(links: &[&'a str], own: &[CallService]) -> Option<Element<'a, app::Message>> {
    let mut seen = Vec::new();
    let cards: Vec<Element<'a, app::Message>> = links
        .iter()
        .filter_map(|link| Some((*link, recognize_call_link(link, own)?)))
        .filter(|(link, _)| {
            let new = !seen.contains(link);
            seen.push(*link);
            new
        })
        .map(|(link, name)| card(link, name))
        .collect();
    (!cards.is_empty()).then(|| column(cards).spacing(4).into())
}

fn card<'a>(link: &'a str, service: String) -> Element<'a, app::Message> {
    let shown = link
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    container(
        row![
            container(icons::camera(18.0, on_primary))
                .center(32)
                .style(style::call_icon),
            column![
                text(format!("{service} call"))
                    .font(fonts::SEMIBOLD)
                    .size(14),
                ellipsis(shown).size(12).style(text::secondary),
            ]
            .spacing(1)
            .width(Length::Fill),
            button(text("Join").size(13))
                .padding([5, 14])
                .style(style::primary_button)
                .on_press(app::Message::LinkClicked(link.to_owned())),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding([8, 10])
    .max_width(460)
    .style(style::card)
    .into()
}

fn on_primary(theme: &Theme) -> iced::Color {
    theme.extended_palette().primary.base.text
}
