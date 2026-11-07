//! Downloads: starting them (Download to the Downloads folder, Save as…,
//! and opening a file in its app), and the list of them behind an icon at
//! the bottom of the space bar, shown while there are any.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use iced::{
    Alignment, Element, Length, Subscription, Task, Theme,
    widget::{button, column, container, progress_bar, row, scrollable, space, stack, text},
};
use mtrx_core::{
    Download, DownloadId, DownloadState, Media, OwnedEventId, OwnedRoomId, SaveTo, Session,
};

use crate::{
    app::{self, Shared},
    context_menu::context_menu,
    ellipsis::ellipsis,
    files, fonts,
    hint::{Position, hint},
    icons,
    menu::{self, Entry, Item, Width},
    popover::{Edge, Side, popover},
    style,
};

/// Width of the list.
const PANEL_WIDTH: f32 = 340.0;
/// The list scrolls beyond this height.
const PANEL_HEIGHT: f32 = 420.0;
/// The icon in the space bar, and the progress ring around it.
const ICON: f32 = 20.0;
const RING: f32 = 30.0;

/// A message's file and where it was sent: what downloading it needs.
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub media: Media,
    pub room: OwnedRoomId,
    pub event: Option<OwnedEventId>,
}

/// The list as the core last sent it. Whether it is open, and a
/// download's menu in it, is the window's popup
/// ([`app::Popup::Downloads`]).
#[derive(Default)]
pub struct Downloads {
    /// Newest first.
    list: Vec<Download>,
    /// Why opening or showing a download's file failed.
    errors: HashMap<DownloadId, String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Updated(Vec<Download>),
    /// For the app: open the list.
    OpenList,
    /// For the app: open a download's menu in the list, or close it
    /// (`None`), the list staying open.
    Menu(Option<DownloadId>),
    /// Open a finished download's file in its app.
    Open(DownloadId),
    /// Show a finished download's file in its folder.
    Show(DownloadId),
    Cancel(DownloadId),
    Retry(DownloadId),
    Remove(DownloadId),
    /// Remove every download that doesn't run.
    Clear,
    /// Opening or showing a download's file failed.
    Failed(DownloadId, String),
    /// Download a message's file to the Downloads folder.
    Download(File),
    /// Ask where to save a message's file, then download it there.
    SaveAs(File),
    SaveTo(File, PathBuf),
    /// Open a message's file in its app: a finished download of it, else
    /// once downloaded.
    OpenFile(File),
    /// For the app: show the message in its room.
    GoTo(OwnedRoomId, OwnedEventId),
    /// For the app: something failed outside the list.
    Report(String),
}

/// The list, now and after every change.
pub fn subscription(session: &Arc<Session>) -> Subscription<Message> {
    Subscription::run_with((Shared(session.clone()), "downloads"), |(s, _)| {
        s.0.downloads().updates()
    })
    .map(Message::Updated)
}

impl Downloads {
    /// Handles all but [`Message::OpenList`], [`Message::Menu`],
    /// [`Message::GoTo`] and [`Message::Report`], which are the app's.
    pub fn update(&mut self, session: &Arc<Session>, message: Message) -> Task<Message> {
        match message {
            Message::Updated(list) => {
                self.errors.retain(|id, _| list.iter().any(|d| d.id == *id));
                self.list = list;
            }
            Message::Open(id) => {
                if let Some(path) = self.done(id) {
                    return open_file(id, path);
                }
            }
            Message::Show(id) => {
                if let Some(path) = self.done(id) {
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || files::show_in_folder(&path))
                                .await
                                .map_err(|e| e.to_string())?
                        },
                        move |shown| shown.err().map(|e| Message::Failed(id, e)),
                    )
                    .and_then(Task::done);
                }
            }
            Message::Cancel(id) => session.cancel_download(id),
            Message::Retry(id) => {
                self.errors.remove(&id);
                session.retry_download(id);
            }
            Message::Remove(id) => session.remove_download(id),
            Message::Clear => session.clear_downloads(),
            Message::Failed(id, error) => {
                self.errors.insert(id, error);
            }
            Message::Download(file) => {
                if let Err(e) = start(session, &file, SaveTo::Downloads) {
                    return Task::done(Message::Report(e));
                }
            }
            Message::SaveAs(file) => {
                let dialog = rfd::AsyncFileDialog::new().set_file_name(&file.media.filename);
                return Task::perform(
                    async move { dialog.save_file().await.map(|f| f.path().to_owned()) },
                    move |path| path.map(|path| Message::SaveTo(file.clone(), path)),
                )
                .and_then(Task::done);
            }
            Message::SaveTo(file, path) => {
                if let Err(e) = start(session, &file, SaveTo::File(path)) {
                    return Task::done(Message::Report(e));
                }
            }
            Message::OpenFile(file) => {
                // A finished download of it, if its file is still there.
                let done = self.list.iter().find_map(|d| match &d.state {
                    DownloadState::Done { path }
                        if d.uri() == file.media.source.uri() && path.exists() =>
                    {
                        Some((d.id, path.clone()))
                    }
                    _ => None,
                });
                if let Some((id, path)) = done {
                    return open_file(id, path);
                }
                let id = match start(session, &file, SaveTo::Downloads) {
                    Ok(id) => id,
                    Err(e) => return Task::done(Message::Report(e)),
                };
                let session = session.clone();
                // A failed download says so in the list.
                return Task::future(async move { session.finished(id).await.ok() })
                    .and_then(move |path| open_file(id, path));
            }
            Message::OpenList | Message::Menu(_) | Message::GoTo(..) | Message::Report(_) => {}
        }
        Task::none()
    }

    /// What stays open of the list, `menu` open in it before, as the list
    /// changed: nothing once it is empty, and no menu of a download gone.
    pub fn kept(&self, menu: Option<DownloadId>) -> Option<Option<DownloadId>> {
        if self.list.is_empty() {
            return None;
        }
        Some(menu.filter(|id| self.list.iter().any(|d| d.id == *id)))
    }

    /// Whether `message` is a choice in a download's row, which closes its
    /// menu.
    pub fn chosen(message: &Message) -> bool {
        matches!(
            message,
            Message::Open(_) | Message::Show(_) | Message::Remove(_)
        )
    }

    /// The path of download `id`, if done.
    fn done(&self, id: DownloadId) -> Option<PathBuf> {
        self.list.iter().find_map(|d| match &d.state {
            DownloadState::Done { path } if d.id == id => Some(path.clone()),
            _ => None,
        })
    }

    /// The icon, with the list above it while `open` and `menu`'s menu in
    /// it; nothing while there are no downloads.
    pub fn view(&self, open: bool, menu: Option<DownloadId>) -> Option<Element<'_, app::Message>> {
        if self.list.is_empty() {
            return None;
        }
        let running = self.list.iter().filter(|d| d.is_running()).count();
        let mut icon = stack![
            container(icons::download(ICON, |theme| {
                theme
                    .extended_palette()
                    .background
                    .base
                    .text
                    .scale_alpha(0.7)
            }))
            .center(RING)
        ];
        if running > 0 {
            icon = icon.push(icons::ring(RING, overall(&self.list)));
        }
        // While open, a click on it closes the list as any outside click.
        let anchor = button(icon)
            .padding(3)
            .style(style::list_item(open))
            .on_press_maybe((!open).then_some(Message::OpenList));
        let tip = match running {
            0 => "Downloads".to_owned(),
            1 => "Downloading 1 file…".to_owned(),
            n => format!("Downloading {n} files…"),
        };
        let anchor: Element<'_, Message> = hint(anchor, tip, Position::Right);
        let panel = open.then(|| self.panel(menu).map(app::Message::Downloads));
        Some(
            popover(
                anchor.map(app::Message::Downloads),
                panel,
                Side::Above,
                Edge::Left,
            )
            .width(PANEL_WIDTH)
            .on_dismiss(app::Message::ClosePopup)
            .into(),
        )
    }

    fn panel(&self, menu: Option<DownloadId>) -> Element<'_, Message> {
        let rows = column(self.list.iter().map(|d| self.row(d, menu == Some(d.id)))).spacing(2);
        let clear = button(text("Clear").size(13))
            .padding([4, 10])
            .style(style::secondary_button)
            .on_press_maybe(
                (self.list.iter())
                    .any(|d| !d.is_running())
                    .then_some(Message::Clear),
            );
        container(
            column![
                text("Downloads").size(14).font(fonts::SEMIBOLD),
                scrollable(rows).height(Length::Shrink),
                row![space::horizontal(), clear],
            ]
            .spacing(6),
        )
        .padding(8)
        .max_height(PANEL_HEIGHT)
        .style(style::dialog)
        .into()
    }

    /// A download, with its `menu` open or not.
    fn row<'a>(&'a self, download: &'a Download, menu: bool) -> Element<'a, Message> {
        let id = download.id;
        let status = text(download.status()).size(12);
        let status = match download.state {
            DownloadState::Failed(_) => status.style(style::problem_text),
            _ => status.style(text::secondary),
        };
        let mut about = column![ellipsis(&download.file_name).size(13), status]
            .spacing(1)
            .width(Length::Fill);
        if let Some(error) = self.errors.get(&id) {
            about = about.push(ellipsis(error).size(12).style(style::problem_text));
        }
        if download.is_running() {
            // Empty while the length is unknown.
            about =
                about.push(progress_bar(0.0..=1.0, download.fraction().unwrap_or(0.0)).girth(3));
        }
        let action = match download.state {
            DownloadState::Downloading { .. } => {
                small_button(icons::cross(12.0, secondary), Message::Cancel(id), "Cancel")
            }
            DownloadState::Done { .. } => small_button(
                icons::folder(14.0, secondary),
                Message::Show(id),
                "Show in folder",
            ),
            DownloadState::Failed(_) | DownloadState::Cancelled => small_button(
                text("↻").size(14).style(text::secondary),
                Message::Retry(id),
                "Try again",
            ),
        };
        let done = matches!(download.state, DownloadState::Done { .. });
        let content = row![
            container(icons::file(14.0, |theme| theme
                .extended_palette()
                .primary
                .base
                .text))
            .center(28)
            .style(style::call_icon),
            about,
            action,
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let entry = button(content)
            .padding([6, 8])
            .width(Length::Fill)
            .style(style::list_item(false))
            .on_press_maybe(done.then_some(Message::Open(id)));
        let menu = menu.then(|| menu::view(entries(download), Width::Menu));
        context_menu(entry, menu, Message::Menu(Some(id)))
            .on_dismiss(Message::Menu(None))
            .into()
    }
}

/// A download's menu: what can be done with it.
fn entries(download: &Download) -> Vec<Entry<'static, Message>> {
    let mut items = Vec::new();
    if matches!(download.state, DownloadState::Done { .. }) {
        items.push(Item::new("Open", Message::Open(download.id)));
        items.push(Item::new("Show in folder", Message::Show(download.id)));
    }
    if let Some(event) = &download.event_id {
        items.push(Item::new(
            "Go to message",
            Message::GoTo(download.room_id.clone(), event.clone()),
        ));
    }
    items.push(Item::new("Remove from list", Message::Remove(download.id)));
    items.into_iter().map(Entry::Item).collect()
}

/// A small borderless button with a hint.
fn small_button<'a>(
    content: impl Into<Element<'a, Message>>,
    message: Message,
    tip: &'a str,
) -> Element<'a, Message> {
    hint(
        button(container(content).center(18))
            .padding(4)
            .style(style::secondary_button)
            .on_press(message),
        tip,
        Position::Top,
    )
}

fn secondary(theme: &Theme) -> iced::Color {
    theme
        .extended_palette()
        .background
        .base
        .text
        .scale_alpha(0.75)
}

/// Start downloading `file` to `to`.
fn start(session: &Arc<Session>, file: &File, to: SaveTo) -> Result<DownloadId, String> {
    let room = (session.room(&file.room))
        .map_err(|e| format!("Could not download {}: {e}", file.media.filename))?;
    Ok(room.download(&file.media, file.event.as_deref(), to))
}

/// Open download `id`'s file at `path` in its app; a failure shows in its
/// row.
fn open_file(id: DownloadId, path: PathBuf) -> Task<Message> {
    match open::that_detached(&path) {
        Ok(()) => Task::none(),
        Err(e) => Task::done(Message::Failed(id, format!("Could not open it: {e}"))),
    }
}

/// How far the running downloads with a known length are, together.
fn overall(list: &[Download]) -> Option<f32> {
    let (received, total) = (list.iter())
        .filter_map(|d| match d.state {
            DownloadState::Downloading {
                received,
                total: Some(total),
            } if total > 0 => Some((received, total)),
            _ => None,
        })
        .fold((0, 0), |(r, t), (received, total)| {
            (r + received, t + total)
        });
    (total > 0).then(|| (received as f64 / total as f64).min(1.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(id: DownloadId, received: u64, total: Option<u64>) -> Download {
        let mut download = test_download(id);
        download.state = DownloadState::Downloading { received, total };
        download
    }

    fn test_download(id: DownloadId) -> Download {
        Download::example(id, "report.pdf")
    }

    #[test]
    fn overall_progress_counts_known_lengths() {
        assert_eq!(overall(&[]), None);
        assert_eq!(overall(&[running(1, 5, None)]), None);
        let list = [
            running(1, 25, Some(100)),
            running(2, 75, Some(100)),
            running(3, 5, None),
        ];
        assert_eq!(overall(&list), Some(0.5));
    }

    #[test]
    fn menus_offer_what_fits_the_state() {
        let labels = |download: &Download| -> Vec<String> {
            entries(download)
                .iter()
                .filter_map(Entry::label)
                .map(str::to_owned)
                .collect()
        };
        let mut done = test_download(1);
        done.state = DownloadState::Done {
            path: "/tmp/report.pdf".into(),
        };
        done.event_id = Some("$event".try_into().unwrap());
        assert_eq!(
            labels(&done),
            [
                "Open",
                "Show in folder",
                "Go to message",
                "Remove from list"
            ]
        );
        assert_eq!(labels(&running(2, 0, None)), ["Remove from list"]);
    }

    #[test]
    fn the_icon_shows_while_there_are_downloads() {
        let mut downloads = Downloads::default();
        assert!(downloads.view(false, None).is_none());
        downloads.list = vec![running(1, 0, None)];
        assert!(downloads.view(false, None).is_some());
    }

    /// The list stays open while there are downloads, a menu while its
    /// download is listed.
    #[test]
    fn what_stays_open_as_the_list_changes() {
        let mut downloads = Downloads::default();
        downloads.list = vec![running(1, 0, None), running(2, 0, None)];
        assert_eq!(downloads.kept(Some(2)), Some(Some(2)));
        assert_eq!(downloads.kept(Some(3)), Some(None));
        assert_eq!(downloads.kept(None), Some(None));
        downloads.list.clear();
        assert_eq!(downloads.kept(None), None);
    }
}
