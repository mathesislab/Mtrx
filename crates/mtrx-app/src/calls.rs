//! Calls through links: call cards in the timeline, and the room header's
//! split button: Join, for the room's call (the first in its topic), or
//! Start call; and the menu of the room's calls and links, which are
//! dragged to other places, of calls with other services, and to edit
//! them.

use iced::Task;
use iced::{
    Alignment, Element, Length, Theme,
    widget::{button, column, container, row, text},
};
use mtrx_core::{CallService, OwnedRoomId, RoomLink, ServiceKind, Topic, Url, recognize_call_link};
use mtrx_google::Google;

use crate::{
    app::{self, Cx, Popup},
    ellipsis::ellipsis,
    fonts,
    hint::{Position, hint, hint_with},
    icons,
    menu::{self, Entry, Item, Width},
    popover::{Edge, Side, popover},
    reorder::{self, Reorder},
    room::{self, Room},
    style, topic,
};

const MENU_WIDTH: f32 = 300.0;
/// The split button's height, both halves the same whatever they show.
const SPLIT_HEIGHT: f32 = 27.0;

#[derive(Debug, Clone)]
pub enum Message {
    /// The call service of the room's nearest space with one.
    SpaceService(Option<CallService>),
    OpenMenu,
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
    /// A call is being started.
    pub busy: bool,
}

/// The menu of the room's calls and links, while open (the window's
/// popup): their drags.
#[derive(Default)]
pub struct Menu {
    pub calls: Reorder,
    pub links: Reorder,
    /// Starting a call failed.
    pub error: Option<String>,
}

impl RoomCalls {
    pub fn new(room: OwnedRoomId) -> Self {
        Self {
            room,
            space_service: None,
            busy: false,
        }
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
    /// opening the menu (`menu`, while open). `can_edit`: whether we may
    /// change the topic.
    pub fn header_controls<'a>(
        &'a self,
        topic: &Topic,
        own: &'a [CallService],
        can_edit: bool,
        menu: Option<&'a Menu>,
    ) -> Element<'a, app::Message> {
        let open = menu.is_some();
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
        let links = topic.others().count();
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
        let menu = menu.map(|menu| {
            menu::view(
                self.entries(menu, topic, own, can_edit),
                Width::At(MENU_WIDTH),
            )
        });
        popover(
            row![main, toggle].spacing(1).height(SPLIT_HEIGHT),
            menu,
            Side::Below,
            Edge::Right,
        )
        .on_dismiss(app::Message::ClosePopup)
        .into()
    }

    /// The menu: the room's calls and links, which those who may edit
    /// them reorder, then the services to start a call with; nothing to
    /// choose while a call is being started (`busy`).
    fn entries<'a>(
        &'a self,
        state: &'a Menu,
        topic: &Topic,
        own: &'a [CallService],
        can_edit: bool,
    ) -> Vec<Entry<'a, app::Message>> {
        let mut entries = Vec::new();
        let (calls, links): (Vec<&RoomLink>, Vec<&RoomLink>) =
            (topic.calls().collect(), topic.others().collect());
        for (group, order, message) in [
            (calls, &state.calls, Message::Calls as fn(_) -> _),
            (links, &state.links, Message::Links),
        ] {
            let Some(first) = group.first() else {
                continue;
            };
            let call = first.is_call();
            entries.push(Entry::Heading(
                if call { "Calls" } else { "Links" }.to_owned(),
            ));
            let many = group.len() > 1;
            let rows = (group.iter().enumerate())
                .map(|(at, link)| self.link_item(link, own, call && many && at == 0))
                .collect();
            let room = self.room.clone();
            entries.push(Entry::Custom(reorder::list(
                order,
                rows,
                can_edit && many,
                move |m| room::Message::Calls(message(m)).to(&room),
            )));
        }
        if !topic.links.is_empty() {
            entries.push(Entry::Separator);
        }
        entries.push(Entry::Heading("Start a new call with".to_owned()));
        let services = self.services(own);
        if services.is_empty() {
            entries.push(Entry::Heading(
                "No call services yet: add one in Settings".to_owned(),
            ));
        }
        for service in services {
            let label = if Some(service) == self.space_service.as_ref() {
                format!("{} (space)", service.name)
            } else {
                service.name.clone()
            };
            let item = if self.busy {
                Item::disabled(label)
            } else {
                Item::new(label, self.message(Message::Start(service.clone())))
            };
            entries.push(item.hint(start_hint(service)).into());
        }
        if can_edit {
            entries.push(Entry::Separator);
            entries.push(
                Item::new(
                    "Edit room links…",
                    room::Message::Topic(topic::Message::Edit).to(&self.room),
                )
                .into(),
            );
        }
        if let Some(error) = &state.error {
            entries.push(Entry::Custom(
                container(text(error).size(12).style(text::danger))
                    .padding([4, 10])
                    .into(),
            ));
        }
        entries
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

/// A message to the room's calls and links.
pub fn update(room: &mut Room, message: Message, cx: &mut Cx<'_>) -> Task<app::Message> {
    let id = room.id.clone();
    let google = cx.google.clone();
    let state = &mut room.calls;
    let menu = |popup: &mut Popup| matches!(popup, Popup::Calls(_));
    let moved = match message {
        Message::SpaceService(service) => {
            state.space_service = service;
            return Task::none();
        }
        Message::OpenMenu => {
            *cx.popup = Some(Popup::Calls(Menu::default()));
            return Task::none();
        }
        Message::Open(url) => {
            cx.popup.take_if(menu);
            return Task::done(app::Message::LinkClicked(url));
        }
        Message::Start(service) => {
            state.busy = true;
            cx.popup.take_if(menu);
            let room = room.handle.clone();
            return Task::perform(
                async move {
                    let link = new_call_link(&service, &google).await?;
                    (room.post_call(&service.name, &link).await).map_err(|e| e.to_string())
                },
                move |result| room::Message::Calls(Message::Started(result)).to(&id),
            );
        }
        Message::Started(result) => {
            state.busy = false;
            // Shown below the composer, like other sending errors.
            if let Err(e) = result {
                room.error = Some(e);
            }
            return Task::none();
        }
        Message::Calls(order) => match cx.popup {
            Some(Popup::Calls(menu)) => menu.calls.update(order).map(|moved| (true, moved)),
            _ => None,
        },
        Message::Links(order) => match cx.popup {
            Some(Popup::Calls(menu)) => menu.links.update(order).map(|moved| (false, moved)),
            _ => None,
        },
    };
    let Some((calls, moved)) = moved else {
        return Task::none();
    };
    // A call moved to the top becomes the room's: the topic changes.
    let (from, to) = moved;
    let listed = topic::of(cx.lists.room(&id)).clone();
    let topic = listed.moved(calls, from, to).to_topic();
    topic::save(
        cx.session,
        &mut room.topic,
        id.clone(),
        topic,
        move |message| room::Message::Topic(message).to(&id),
    )
}

/// A link for a new call with `service`.
async fn new_call_link(service: &CallService, google: &Google) -> Result<Url, String> {
    match service.kind {
        ServiceKind::GoogleMeet => google.create_meeting().await.map_err(|e| match e {
            mtrx_google::Error::NotSignedIn => {
                "Connect your Google account in Settings to create Meet links.".to_owned()
            }
            e => e.to_string(),
        }),
        _ => service
            .new_link()
            .ok_or_else(|| format!("{} cannot make call links", service.name)),
    }
}
