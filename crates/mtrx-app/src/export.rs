//! The export dialog: a room's or a space's chats, the space's rooms to
//! include, the format, which messages, files, and where to. The export
//! itself runs in core; its progress shows with the downloads.

use std::{collections::HashSet, path::PathBuf, time::SystemTime};

use chrono::{Local, NaiveDate, TimeZone};
use iced::{
    Alignment, Element, Length,
    widget::{button, checkbox, column, row, space, text, text_input, toggler},
};
use mtrx_core::{
    ExportFiles, ExportFormat, ExportOf, ExportRange, ExportRequest, Membership, OwnedRoomId,
    RoomKind, RoomLists, SaveTo,
};

use crate::{
    app, calendar,
    ellipsis::ellipsis,
    frame,
    hint::{Position, hint},
    style,
};

const WIDTH: f32 = 480.0;
/// The messages of each room a "Last" export takes at first.
const LAST: u32 = 1000;
/// The size above which files are left out at first, in MB.
const MAX_MB: u32 = 100;

/// What is exported.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Room(OwnedRoomId),
    /// The rooms below it, chosen in the dialog.
    Space(OwnedRoomId),
}

/// Which messages of each room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    All,
    Since,
    Last,
}

/// A line of a space's checklist.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    /// Levels of subspaces in.
    pub depth: usize,
    pub id: OwnedRoomId,
    pub name: String,
    pub kind: ChoiceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceKind {
    /// A subspace: a heading.
    Space,
    Room {
        joined: bool,
    },
}

#[derive(Debug, Clone)]
pub enum Message {
    /// The rooms' search field changed.
    Query(String),
    Toggle(OwnedRoomId),
    /// Tick all of these rooms.
    TickAll(Vec<OwnedRoomId>),
    Format(ExportFormat),
    Range(Range),
    /// Show the month this many months away.
    Month(i32),
    Day(NaiveDate),
    Last(String),
    Files(bool),
    MaxMb(String),
    /// Ask for another folder to save to.
    ChangeFolder,
    Folder(PathBuf),
    Submit,
    Cancel,
}

/// What the app should do after a [`Message`].
#[derive(Debug, PartialEq)]
pub enum Action {
    None,
    Close,
    /// Ask for a folder, then send [`Message::Folder`].
    PickFolder,
    Export(ExportRequest),
}

pub struct Dialog {
    target: Target,
    /// The room's or space's name.
    title: String,
    /// A space's rooms and subspaces, in its order.
    choices: Vec<Choice>,
    ticked: HashSet<OwnedRoomId>,
    query: String,
    format: ExportFormat,
    range: Range,
    since: NaiveDate,
    /// The month the calendar shows, as its first day.
    month: NaiveDate,
    last: String,
    files: bool,
    max_mb: String,
    /// `None`: the Downloads folder.
    folder: Option<PathBuf>,
    error: Option<String>,
}

impl Dialog {
    /// Export `target`, called `title`, as of `today`; a space's joined
    /// rooms ticked.
    pub fn new(target: Target, title: String, lists: &RoomLists, today: NaiveDate) -> Self {
        let choices = match &target {
            Target::Room(_) => Vec::new(),
            Target::Space(space) => choices(space, lists),
        };
        let ticked = (choices.iter())
            .filter(|c| c.kind == ChoiceKind::Room { joined: true })
            .map(|c| c.id.clone())
            .collect();
        let since = today - chrono::Days::new(30);
        Self {
            target,
            title,
            choices,
            ticked,
            query: String::new(),
            format: ExportFormat::Html,
            range: Range::All,
            since,
            month: calendar::first_of_month(since),
            last: LAST.to_string(),
            files: true,
            max_mb: MAX_MB.to_string(),
            folder: None,
            error: None,
        }
    }

    pub fn update(&mut self, message: Message, today: NaiveDate) -> Action {
        // What was missing goes with the next change.
        if !matches!(message, Message::Submit) {
            self.error = None;
        }
        match message {
            Message::Query(query) => self.query = query,
            Message::Toggle(room) => {
                if !self.ticked.remove(&room) {
                    self.ticked.insert(room);
                }
            }
            Message::TickAll(rooms) => self.ticked.extend(rooms),
            Message::Format(format) => self.format = format,
            Message::Range(range) => self.range = range,
            Message::Month(by) => {
                if let Some(month) = calendar::turn(self.month, by)
                    .filter(|month| *month <= calendar::first_of_month(today))
                {
                    self.month = month;
                }
            }
            Message::Day(day) => self.since = day.min(today),
            Message::Last(last) => self.last = last,
            Message::Files(on) => self.files = on,
            Message::MaxMb(max) => self.max_mb = max,
            Message::ChangeFolder => return Action::PickFolder,
            Message::Folder(folder) => self.folder = Some(folder),
            Message::Cancel => return Action::Close,
            Message::Submit => match self.request() {
                Ok(request) => return Action::Export(request),
                Err(e) => self.error = Some(e.into()),
            },
        }
        Action::None
    }

    /// The export could not start: why.
    pub fn failed(&mut self, error: String) {
        self.error = Some(error);
    }

    /// The export the dialog's choices ask for, or what is missing.
    pub fn request(&self) -> Result<ExportRequest, &'static str> {
        let of = match &self.target {
            Target::Room(room) => ExportOf::Room(room.clone()),
            Target::Space(space) => {
                let mut seen = HashSet::new();
                let rooms: Vec<OwnedRoomId> = (self.choices.iter())
                    .filter(|c| self.ticked.contains(&c.id) && seen.insert(&c.id))
                    .map(|c| c.id.clone())
                    .collect();
                if rooms.is_empty() {
                    return Err("Choose at least one room.");
                }
                ExportOf::Space {
                    space: space.clone(),
                    rooms,
                }
            }
        };
        let range = match self.range {
            Range::All => ExportRange::All,
            Range::Since => ExportRange::Since(start_of(self.since)),
            Range::Last => ExportRange::Last(
                (self.last.trim().parse().ok())
                    .filter(|n: &u32| *n > 0)
                    .ok_or("Type how many messages of each room.")?,
            ),
        };
        let max = self.max_mb.trim();
        let files = ExportFiles {
            include: self.files,
            max_file: match max {
                "" => None,
                _ if !self.files => None,
                max => Some(
                    (max.parse::<f64>().ok())
                        .filter(|mb| mb.is_finite() && *mb > 0.0)
                        .map(|mb| (mb * 1e6) as u64)
                        .ok_or("Type the largest file size in MB.")?,
                ),
            },
        };
        let to = match &self.folder {
            Some(folder) => SaveTo::File(folder.clone()),
            None => SaveTo::Downloads,
        };
        Ok(ExportRequest {
            of,
            format: self.format,
            range,
            files,
            to,
        })
    }

    pub fn view<'a>(&'a self, today: NaiveDate) -> Element<'a, app::Message> {
        let mut body = column![].spacing(16);
        if matches!(self.target, Target::Space(_)) {
            body = body.push(section("Rooms", self.rooms_view()));
        }
        body = body.push(section(
            "Format",
            segments(
                &ExportFormat::ALL,
                self.format,
                |f| f.name(),
                Message::Format,
            ),
        ));
        let mut messages = column![segments(
            &[Range::All, Range::Since, Range::Last],
            self.range,
            |range| match range {
                Range::All => "All",
                Range::Since => "Since a day",
                Range::Last => "The last",
            },
            Message::Range,
        )]
        .spacing(10);
        match self.range {
            Range::All => {}
            Range::Since => {
                messages = messages.push(
                    text(format!("From {}", self.since.format("%A %-d %B %Y")))
                        .size(13)
                        .style(text::secondary),
                );
                messages = messages.push(calendar::view(
                    self.month,
                    self.since,
                    today,
                    |day| day <= today,
                    |by| {
                        (by < 0 || self.month < calendar::first_of_month(today))
                            .then(|| message(Message::Month(by)))
                    },
                    |day| message(Message::Day(day)),
                ));
            }
            Range::Last => {
                messages = messages.push(
                    row![
                        text_input("1000", &self.last)
                            .on_input(|v| message(Message::Last(v)))
                            .on_submit(message(Message::Submit))
                            .style(style::input)
                            .padding([5, 8])
                            .width(90),
                        text("messages of each room").size(13),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
            }
        }
        body = body.push(section("Messages", messages));
        let mut files = column![
            row![
                text("Include files").width(Length::Fill),
                toggler(self.files).on_toggle(|on| message(Message::Files(on))),
            ]
            .align_y(Alignment::Center)
        ]
        .spacing(8);
        if self.files {
            files = files.push(
                row![
                    text("Leave out files over").size(13),
                    text_input("no limit", &self.max_mb)
                        .on_input(|v| message(Message::MaxMb(v)))
                        .on_submit(message(Message::Submit))
                        .style(style::input)
                        .padding([5, 8])
                        .width(70),
                    text("MB").size(13),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        body = body.push(section("Files", files));
        let folder = match &self.folder {
            Some(folder) => folder.display().to_string(),
            None => "Downloads".to_owned(),
        };
        body = body.push(section(
            "Save to",
            row![
                ellipsis(folder).size(13).width(Length::Fill),
                button(text("Change…").size(13))
                    .padding([4, 10])
                    .style(style::secondary_button)
                    .on_press(message(Message::ChangeFolder)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ));
        if let Some(error) = &self.error {
            body = body.push(text(error).size(12).style(text::danger));
        }
        body = body.push(
            row![
                space::horizontal(),
                button(text("Cancel"))
                    .style(style::text_button)
                    .on_press(message(Message::Cancel)),
                button(text("Export"))
                    .style(style::primary_button)
                    .on_press(message(Message::Submit)),
            ]
            .spacing(8),
        );
        frame::frame(
            frame::title(format!("Export {}", self.title)),
            None,
            message(Message::Cancel),
            body,
            WIDTH,
        )
    }

    /// A space's rooms that the search finds, to tick, under their
    /// subspaces; those we're not in can't be.
    fn rooms_view(&self) -> Element<'_, app::Message> {
        let query = self.query.trim().to_lowercase();
        let found = |choice: &Choice| {
            choice.kind != ChoiceKind::Space && choice.name.to_lowercase().contains(&query)
        };
        let unticked: Vec<OwnedRoomId> = (self.choices.iter())
            .filter(|c| {
                found(c)
                    && c.kind == ChoiceKind::Room { joined: true }
                    && !self.ticked.contains(&c.id)
            })
            .map(|c| c.id.clone())
            .collect();
        let chosen = match self.ticked.len() {
            0 => "No rooms chosen".to_owned(),
            1 => "1 room chosen".to_owned(),
            n => format!("{n} rooms chosen"),
        };
        let tick_all = button(text("Choose all shown").size(13))
            .padding([3, 8])
            .style(style::secondary_button)
            .on_press_maybe((!unticked.is_empty()).then(|| message(Message::TickAll(unticked))));
        let mut list = column![].spacing(2);
        for choice in &self.choices {
            let indent = space().width(choice.depth as f32 * 18.0);
            match choice.kind {
                // Subspaces show while searching for nothing.
                ChoiceKind::Space if query.is_empty() => {
                    list = list.push(
                        row![indent, text(&choice.name).size(13).style(text::secondary)]
                            .padding([4, 8]),
                    );
                }
                ChoiceKind::Space => {}
                ChoiceKind::Room { joined } if found(choice) => {
                    let toggle = joined.then(|| message(Message::Toggle(choice.id.clone())));
                    let mut tick = checkbox(self.ticked.contains(&choice.id));
                    if let Some(toggle) = toggle.clone() {
                        tick = tick.on_toggle(move |_| toggle.clone());
                    }
                    let mut line = row![indent, tick, text(&choice.name).width(Length::Fill)]
                        .spacing(8)
                        .align_y(Alignment::Center);
                    if !joined {
                        line = line.push(text("not joined").size(12).style(text::secondary));
                    }
                    let line = button(line)
                        .width(Length::Fill)
                        .padding([4, 8])
                        .style(style::list_item(false))
                        .on_press_maybe(toggle);
                    list = list.push(if joined {
                        Element::from(line)
                    } else {
                        hint(line, "Join it to export it", Position::Top)
                    });
                }
                ChoiceKind::Room { .. } => {}
            }
        }
        column![
            text_input("Search the space's rooms", &self.query)
                .on_input(|q| message(Message::Query(q)))
                .style(style::input)
                .padding(8),
            row![
                text(chosen)
                    .size(12)
                    .style(text::secondary)
                    .width(Length::Fill),
                tick_all,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            frame::scroll(list).height(220),
        ]
        .spacing(8)
        .into()
    }
}

/// The rooms and subspaces below `space` that the room lists know, depth
/// first in each space's order; a room in two subspaces under both.
fn choices(space: &OwnedRoomId, lists: &RoomLists) -> Vec<Choice> {
    (lists.graph.tree(space).into_iter())
        .filter_map(|(depth, id)| {
            let room = lists.room(&id);
            let is_space = room.is_some_and(|r| r.kind == RoomKind::Space)
                || !lists.graph.children(&id).is_empty();
            let kind = if is_space {
                ChoiceKind::Space
            } else {
                ChoiceKind::Room {
                    joined: room?.membership == Membership::Joined,
                }
            };
            let name = room.map_or_else(|| id.to_string(), |r| r.name.clone());
            Some(Choice {
                depth,
                id,
                name,
                kind,
            })
        })
        .collect()
}

/// Today, here.
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Local midnight at the start of `day`.
fn start_of(day: NaiveDate) -> SystemTime {
    (Local.from_local_datetime(&day.and_time(chrono::NaiveTime::MIN)))
        .earliest()
        .map_or(SystemTime::UNIX_EPOCH, SystemTime::from)
}

/// A labelled part of the dialog.
fn section<'a>(
    label: &'a str,
    content: impl Into<Element<'a, app::Message>>,
) -> Element<'a, app::Message> {
    column![
        text(label).size(13).font(crate::fonts::SEMIBOLD),
        content.into()
    ]
    .spacing(6)
    .into()
}

/// One choice of `options`, as a row of buttons.
fn segments<'a, T: Copy + PartialEq + 'a>(
    options: &[T],
    selected: T,
    label: impl Fn(T) -> &'static str,
    on: impl Fn(T) -> Message,
) -> Element<'a, app::Message> {
    row(options.iter().map(|&option| {
        button(text(label(option)).size(13))
            .padding([4, 12])
            .style(style::list_item(option == selected))
            .on_press(message(on(option)))
            .into()
    }))
    .spacing(4)
    .into()
}

fn message(message: Message) -> app::Message {
    app::Message::Modal(app::modal::Message::Export(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> OwnedRoomId {
        format!("!{name}:example.org").try_into().unwrap()
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn space_dialog() -> Dialog {
        let mut dialog = Dialog::new(
            Target::Space(id("ice")),
            "Ice sheets".into(),
            &RoomLists::default(),
            day(2026, 11, 7),
        );
        let room = |name: &str, joined| Choice {
            depth: 1,
            id: id(name),
            name: name.into(),
            kind: ChoiceKind::Room { joined },
        };
        dialog.choices = vec![
            room("lab", true),
            Choice {
                depth: 0,
                id: id("field"),
                name: "Field".into(),
                kind: ChoiceKind::Space,
            },
            room("trip", true),
            room("old", false),
            room("lab", true),
        ];
        dialog.ticked = [id("lab"), id("trip")].into();
        dialog
    }

    #[test]
    fn the_choices_make_the_request() {
        let today = day(2026, 11, 7);
        let mut dialog = space_dialog();
        let request = dialog.request().unwrap();
        assert_eq!(
            request.of,
            ExportOf::Space {
                space: id("ice"),
                rooms: vec![id("lab"), id("trip")],
            }
        );
        assert_eq!(request.format, ExportFormat::Html);
        assert_eq!(request.range, ExportRange::All);
        assert_eq!(request.files, ExportFiles::default());
        assert_eq!(request.to, SaveTo::Downloads);

        for message in [
            Message::Format(ExportFormat::Json),
            Message::Range(Range::Last),
            Message::Last(" 50 ".into()),
            Message::MaxMb("2.5".into()),
            Message::Folder("/tmp/out".into()),
            Message::Toggle(id("trip")),
        ] {
            assert_eq!(dialog.update(message, today), Action::None);
        }
        let request = dialog.request().unwrap();
        assert_eq!(request.format, ExportFormat::Json);
        assert_eq!(request.range, ExportRange::Last(50));
        assert_eq!(request.files.max_file, Some(2_500_000));
        assert_eq!(request.to, SaveTo::File("/tmp/out".into()));
        assert!(matches!(request.of, ExportOf::Space { rooms, .. } if rooms == [id("lab")]));

        dialog.update(Message::Files(false), today);
        assert_eq!(
            dialog.request().unwrap().files,
            ExportFiles {
                include: false,
                max_file: None
            }
        );
        dialog.update(Message::Range(Range::Since), today);
        dialog.update(Message::Day(day(2026, 10, 1)), today);
        assert_eq!(
            dialog.request().unwrap().range,
            ExportRange::Since(start_of(day(2026, 10, 1)))
        );
        // Not a day to come.
        dialog.update(Message::Day(day(2026, 12, 1)), today);
        assert_eq!(dialog.since, today);
    }

    #[test]
    fn what_is_missing_is_said() {
        let today = day(2026, 11, 7);
        let mut dialog = space_dialog();
        dialog.update(Message::Range(Range::Last), today);
        dialog.update(Message::Last("lots".into()), today);
        assert_eq!(dialog.update(Message::Submit, today), Action::None);
        assert!(dialog.error.is_some());
        dialog.update(Message::Last("10".into()), today);
        assert!(dialog.error.is_none(), "goes with the next change");
        dialog.ticked.clear();
        assert_eq!(dialog.request(), Err("Choose at least one room."));
        let mut room = Dialog::new(
            Target::Room(id("lab")),
            "Lab".into(),
            &RoomLists::default(),
            today,
        );
        assert!(matches!(
            room.update(Message::Submit, today),
            Action::Export(ExportRequest {
                of: ExportOf::Room(_),
                ..
            })
        ));
        assert_eq!(
            room.update(Message::ChangeFolder, today),
            Action::PickFolder
        );
        assert_eq!(room.update(Message::Cancel, today), Action::Close);
    }

    #[test]
    fn the_calendar_stays_in_the_past() {
        let today = day(2026, 11, 7);
        let mut dialog = space_dialog();
        assert_eq!(dialog.month, day(2026, 10, 1));
        dialog.update(Message::Month(1), today);
        assert_eq!(dialog.month, day(2026, 11, 1));
        dialog.update(Message::Month(1), today);
        assert_eq!(dialog.month, day(2026, 11, 1));
        dialog.update(Message::Month(-12), today);
        assert_eq!(dialog.month, day(2025, 11, 1));
    }
}
