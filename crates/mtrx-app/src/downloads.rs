//! Downloads: starting them (Download to the Downloads folder, Save as…,
//! and opening a file in its app), and the list of them behind an icon at
//! the bottom of the space bar, shown while there are any. Exports of
//! chats show in the same list, above the downloads.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use iced::{
    Alignment, Element, Length, Subscription, Task, Theme,
    widget::{button, column, container, progress_bar, row, scrollable, space, stack, text},
};
use mtrx_core::{
    Download, DownloadId, DownloadState, Export, ExportId, ExportState, Media, OwnedEventId,
    OwnedRoomId, SaveTo, Session,
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

/// A row of the list: a download or an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Row {
    Download(DownloadId),
    Export(ExportId),
}

/// The lists as the core last sent them. Whether the list is open, and a
/// row's menu in it, is the window's popup ([`app::Popup::Downloads`]).
#[derive(Default)]
pub struct Downloads {
    /// Newest first.
    list: Vec<Download>,
    /// Newest first.
    exports: Vec<Export>,
    /// Why opening or showing a row's file failed.
    errors: HashMap<Row, String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Updated(Vec<Download>),
    Exports(Vec<Export>),
    /// For the app: open the list.
    OpenList,
    /// For the app: open a row's menu in the list, or close it (`None`),
    /// the list staying open.
    Menu(Option<Row>),
    /// Open a finished download's file in its app.
    Open(DownloadId),
    /// Show a finished download's file in its folder.
    Show(DownloadId),
    Cancel(DownloadId),
    Retry(DownloadId),
    Remove(DownloadId),
    /// Open an export's index in the browser.
    OpenExport(ExportId),
    /// Show an export's folder.
    ShowExport(ExportId),
    CancelExport(ExportId),
    RemoveExport(ExportId),
    /// Remove every download and export that doesn't run.
    Clear,
    /// Opening or showing a row's file failed.
    Failed(Row, String),
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
    /// For the app: something finished, such as an export.
    Finished(String),
}

/// The downloads, now and after every change.
pub fn subscription(session: &Arc<Session>) -> Subscription<Message> {
    Subscription::run_with((Shared(session.clone()), "downloads"), |(s, _)| {
        s.0.downloads().updates()
    })
    .map(Message::Updated)
}

/// The exports, now and after every change.
pub fn exports_subscription(session: &Arc<Session>) -> Subscription<Message> {
    Subscription::run_with((Shared(session.clone()), "exports"), |(s, _)| {
        s.0.exports().updates()
    })
    .map(Message::Exports)
}

impl Downloads {
    /// Handles all but [`Message::OpenList`], [`Message::Menu`],
    /// [`Message::GoTo`], [`Message::Report`] and [`Message::Finished`],
    /// which are the app's.
    pub fn update(&mut self, session: &Arc<Session>, message: Message) -> Task<Message> {
        match message {
            Message::Updated(list) => {
                self.list = list;
                self.forget_errors();
            }
            Message::Exports(exports) => {
                let ended = ended(&self.exports, &exports);
                self.exports = exports;
                self.forget_errors();
                return Task::batch(ended.into_iter().map(Task::done));
            }
            Message::Open(id) => {
                if let Some(path) = self.done(id) {
                    return open_path(Row::Download(id), path);
                }
            }
            Message::Show(id) => {
                if let Some(path) = self.done(id) {
                    return show_path(Row::Download(id), path);
                }
            }
            Message::OpenExport(id) => {
                if let Some(export) = self.export(id) {
                    return open_path(Row::Export(id), export.index.clone());
                }
            }
            Message::ShowExport(id) => {
                if let Some(export) = self.export(id) {
                    return show_path(Row::Export(id), export.folder.clone());
                }
            }
            Message::Cancel(id) => session.cancel_download(id),
            Message::Retry(id) => {
                self.errors.remove(&Row::Download(id));
                session.retry_download(id);
            }
            Message::Remove(id) => session.remove_download(id),
            Message::CancelExport(id) => session.cancel_export(id),
            Message::RemoveExport(id) => session.remove_export(id),
            Message::Clear => {
                session.clear_downloads();
                for export in self.exports.iter().filter(|e| !e.is_running()) {
                    session.remove_export(export.id);
                }
            }
            Message::Failed(row, error) => {
                self.errors.insert(row, error);
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
                    return open_path(Row::Download(id), path);
                }
                let id = match start(session, &file, SaveTo::Downloads) {
                    Ok(id) => id,
                    Err(e) => return Task::done(Message::Report(e)),
                };
                let session = session.clone();
                // A failed download says so in the list.
                return Task::future(async move { session.finished(id).await.ok() })
                    .and_then(move |path| open_path(Row::Download(id), path));
            }
            Message::OpenList
            | Message::Menu(_)
            | Message::GoTo(..)
            | Message::Report(_)
            | Message::Finished(_) => {}
        }
        Task::none()
    }

    /// Errors of rows gone go too.
    fn forget_errors(&mut self) {
        let (list, exports) = (&self.list, &self.exports);
        self.errors.retain(|row, _| match row {
            Row::Download(id) => list.iter().any(|d| d.id == *id),
            Row::Export(id) => exports.iter().any(|e| e.id == *id),
        });
    }

    fn is_empty(&self) -> bool {
        self.list.is_empty() && self.exports.is_empty()
    }

    /// What stays open of the list, `menu` open in it before, as the lists
    /// changed: nothing once both are empty, and no menu of a row gone.
    pub fn kept(&self, menu: Option<Row>) -> Option<Option<Row>> {
        if self.is_empty() {
            return None;
        }
        Some(menu.filter(|row| match row {
            Row::Download(id) => self.list.iter().any(|d| d.id == *id),
            Row::Export(id) => self.exports.iter().any(|e| e.id == *id),
        }))
    }

    /// Whether `message` is a choice in a row, which closes its menu.
    pub fn chosen(message: &Message) -> bool {
        matches!(
            message,
            Message::Open(_)
                | Message::Show(_)
                | Message::Remove(_)
                | Message::OpenExport(_)
                | Message::ShowExport(_)
                | Message::RemoveExport(_)
        )
    }

    /// The path of download `id`, if done.
    fn done(&self, id: DownloadId) -> Option<PathBuf> {
        self.list.iter().find_map(|d| match &d.state {
            DownloadState::Done { path } if d.id == id => Some(path.clone()),
            _ => None,
        })
    }

    fn export(&self, id: ExportId) -> Option<&Export> {
        self.exports.iter().find(|e| e.id == id)
    }

    /// The icon, with the list above it while `open` and `menu`'s menu in
    /// it; nothing while there are no downloads or exports.
    pub fn view(&self, open: bool, menu: Option<Row>) -> Option<Element<'_, app::Message>> {
        if self.is_empty() {
            return None;
        }
        let downloading = self.list.iter().filter(|d| d.is_running()).count();
        let exporting = self.exports.iter().filter(|e| e.is_running()).count();
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
        if downloading + exporting > 0 {
            icon = icon.push(icons::ring(RING, overall(&self.list, &self.exports)));
        }
        // While open, a click on it closes the list as any outside click.
        let anchor = button(icon)
            .padding(3)
            .style(style::list_item(open))
            .on_press_maybe((!open).then_some(Message::OpenList));
        let anchor: Element<'_, Message> =
            hint(anchor, tip(downloading, exporting), Position::Right);
        let panel = open.then(|| {
            self.panel(menu)
                .map(|m| app::Message::Media(crate::media::Message::Downloads(m)))
        });
        Some(
            popover(
                anchor.map(|m| app::Message::Media(crate::media::Message::Downloads(m))),
                panel,
                Side::Above,
                Edge::Left,
            )
            .width(PANEL_WIDTH)
            .on_dismiss(app::Message::ClosePopup)
            .into(),
        )
    }

    fn panel(&self, menu: Option<Row>) -> Element<'_, Message> {
        let mut rows = column![].spacing(2);
        if !self.exports.is_empty() {
            rows = rows.push(heading("Exports"));
            for export in &self.exports {
                rows = rows.push(self.export_row(export, menu == Some(Row::Export(export.id))));
            }
        }
        if !self.list.is_empty() {
            rows = rows.push(heading("Downloads"));
            for download in &self.list {
                rows = rows.push(self.row(download, menu == Some(Row::Download(download.id))));
            }
        }
        let finished = (self.list.iter()).any(|d| !d.is_running())
            || (self.exports.iter()).any(|e| !e.is_running());
        let clear = button(text("Clear").size(13))
            .padding([4, 10])
            .style(style::secondary_button)
            .on_press_maybe(finished.then_some(Message::Clear));
        container(
            column![
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
        let failed = matches!(download.state, DownloadState::Failed(_));
        let progress = download.is_running().then(|| download.fraction());
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
        self.entry(
            Row::Download(id),
            (&download.file_name, download.status(), failed, progress),
            action,
            done.then_some(Message::Open(id)),
            menu.then(|| entries(download)),
        )
    }

    /// An export, with its `menu` open or not.
    fn export_row<'a>(&'a self, export: &'a Export, menu: bool) -> Element<'a, Message> {
        let id = export.id;
        let failed = matches!(export.state, ExportState::Failed { .. });
        let progress = export.is_running().then(|| export.fraction());
        let action = if export.is_running() {
            small_button(
                icons::cross(12.0, secondary),
                Message::CancelExport(id),
                "Stop; what's written stays",
            )
        } else {
            small_button(
                icons::folder(14.0, secondary),
                Message::ShowExport(id),
                "Show in folder",
            )
        };
        let title = format!("{} · {}", export.title, export.format.name());
        self.entry(
            Row::Export(id),
            (&title, export.status(), failed, progress),
            action,
            (!export.is_running()).then_some(Message::OpenExport(id)),
            menu.then(|| export_entries(export)),
        )
    }

    /// A row: its icon, name, status (a problem if `failed`) and progress
    /// (`Some` while running, empty while unknown), its action; a click
    /// sends `open`; `menu` is open on it.
    fn entry<'a>(
        &'a self,
        row_of: Row,
        (name, status, failed, progress): (&str, String, bool, Option<Option<f32>>),
        action: Element<'a, Message>,
        open: Option<Message>,
        menu: Option<Vec<Entry<'static, Message>>>,
    ) -> Element<'a, Message> {
        let status = text(status).size(12);
        let status = if failed {
            status.style(style::problem_text)
        } else {
            status.style(text::secondary)
        };
        let mut about = column![ellipsis(name.to_owned()).size(13), status]
            .spacing(1)
            .width(Length::Fill);
        if let Some(error) = self.errors.get(&row_of) {
            about = about.push(ellipsis(error).size(12).style(style::problem_text));
        }
        if let Some(fraction) = progress {
            // Empty while how far is unknown.
            about = about.push(progress_bar(0.0..=1.0, fraction.unwrap_or(0.0)).girth(3));
        }
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
            .on_press_maybe(open);
        let menu = menu.map(|entries| menu::view(entries, Width::Menu));
        context_menu(entry, menu, Message::Menu(Some(row_of)))
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

/// An export's menu: its folder shows while it runs too.
fn export_entries(export: &Export) -> Vec<Entry<'static, Message>> {
    let mut items = Vec::new();
    if !export.is_running() {
        items.push(Item::new("Open", Message::OpenExport(export.id)));
    }
    items.push(Item::new("Show in folder", Message::ShowExport(export.id)));
    items.push(Item::new(
        "Remove from list",
        Message::RemoveExport(export.id),
    ));
    items.into_iter().map(Entry::Item).collect()
}

/// What to say of the exports that ended between `before` and `now`: one
/// done, or failed; not one cancelled.
fn ended(before: &[Export], now: &[Export]) -> Vec<Message> {
    let was_running = |id| before.iter().any(|e| e.id == id && e.is_running());
    (now.iter())
        .filter(|e| !e.is_running() && was_running(e.id))
        .filter_map(|export| match &export.state {
            ExportState::Done(summary) => Some(Message::Finished(format!(
                "Exported {}: {}",
                export.title,
                summary.describe(export.rooms > 1)
            ))),
            ExportState::Failed { message, .. } => Some(Message::Report(format!(
                "Could not export {}: {message}",
                export.title
            ))),
            ExportState::Cancelled(_) | ExportState::Running { .. } => None,
        })
        .collect()
}

/// The icon's hint.
fn tip(downloading: usize, exporting: usize) -> String {
    let files = match downloading {
        0 => None,
        1 => Some("Downloading 1 file…".to_owned()),
        n => Some(format!("Downloading {n} files…")),
    };
    let exports = match exporting {
        0 => None,
        1 => Some("Exporting chats…".to_owned()),
        n => Some(format!("{n} exports running…")),
    };
    match (files, exports) {
        (None, None) => "Downloads and exports".to_owned(),
        (Some(one), None) | (None, Some(one)) => one,
        (Some(files), Some(exports)) => format!("{files} {exports}"),
    }
}

fn heading(label: &str) -> Element<'_, Message> {
    container(text(label).size(14).font(fonts::SEMIBOLD))
        .padding([4, 0])
        .into()
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

/// Open `row`'s file at `path` in its app; a failure shows in its row.
fn open_path(row: Row, path: PathBuf) -> Task<Message> {
    if !path.exists() {
        return Task::done(Message::Failed(row, "Moved or deleted".into()));
    }
    match open::that_detached(&path) {
        Ok(()) => Task::none(),
        Err(e) => Task::done(Message::Failed(row, format!("Could not open it: {e}"))),
    }
}

/// Show `row`'s file at `path` in its folder; a failure shows in its row.
fn show_path(row: Row, path: PathBuf) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || files::show_in_folder(&path))
                .await
                .map_err(|e| e.to_string())?
        },
        move |shown| shown.err().map(|e| Message::Failed(row, e)),
    )
    .and_then(Task::done)
}

/// How far the running downloads with a known length are, together, and
/// the running exports whose progress is known: the downloads count as one.
fn overall(list: &[Download], exports: &[Export]) -> Option<f32> {
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
    let downloads = (total > 0).then(|| (received as f64 / total as f64).min(1.0) as f32);
    let parts: Vec<f32> = (downloads.into_iter())
        .chain(exports.iter().filter_map(Export::fraction))
        .collect();
    (!parts.is_empty()).then(|| parts.iter().sum::<f32>() / parts.len() as f32)
}

#[cfg(test)]
mod tests {
    use mtrx_core::ExportSummary;

    use super::*;

    fn running(id: DownloadId, received: u64, total: Option<u64>) -> Download {
        let mut download = test_download(id);
        download.state = DownloadState::Downloading { received, total };
        download
    }

    fn test_download(id: DownloadId) -> Download {
        Download::example(id, "report.pdf")
    }

    fn exporting(id: ExportId, rooms_done: usize, rooms: usize) -> Export {
        Export::example(
            id,
            "Ice sheets",
            ExportState::Running {
                room: "Lab".into(),
                rooms_done,
                rooms,
                messages: 0,
                files: 0,
                room_fraction: None,
            },
        )
    }

    #[test]
    fn overall_progress_counts_known_lengths() {
        assert_eq!(overall(&[], &[]), None);
        assert_eq!(overall(&[running(1, 5, None)], &[]), None);
        let list = [
            running(1, 25, Some(100)),
            running(2, 75, Some(100)),
            running(3, 5, None),
        ];
        assert_eq!(overall(&list, &[]), Some(0.5));
        // An export half done, with the downloads half done.
        assert_eq!(overall(&list, &[exporting(1, 2, 4)]), Some(0.5));
        assert_eq!(overall(&[], &[exporting(1, 1, 4)]), Some(0.25));
    }

    fn labels(entries: Vec<Entry<'static, Message>>) -> Vec<String> {
        entries
            .iter()
            .filter_map(Entry::label)
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn menus_offer_what_fits_the_state() {
        let mut done = test_download(1);
        done.state = DownloadState::Done {
            path: "/tmp/report.pdf".into(),
        };
        done.event_id = Some("$event".try_into().unwrap());
        assert_eq!(
            labels(entries(&done)),
            [
                "Open",
                "Show in folder",
                "Go to message",
                "Remove from list"
            ]
        );
        assert_eq!(labels(entries(&running(2, 0, None))), ["Remove from list"]);

        assert_eq!(
            labels(export_entries(&exporting(1, 0, 2))),
            ["Show in folder", "Remove from list"]
        );
        let mut finished = exporting(1, 2, 2);
        finished.state = ExportState::Done(ExportSummary::default());
        assert_eq!(
            labels(export_entries(&finished)),
            ["Open", "Show in folder", "Remove from list"]
        );
    }

    #[test]
    fn the_icon_shows_while_there_are_downloads_or_exports() {
        let mut downloads = Downloads::default();
        assert!(downloads.view(false, None).is_none());
        downloads.list = vec![running(1, 0, None)];
        assert!(downloads.view(false, None).is_some());
        downloads.list.clear();
        downloads.exports = vec![exporting(1, 0, 1)];
        assert!(downloads.view(true, None).is_some());
    }

    /// The list stays open while there are rows, a menu while its row is
    /// listed.
    #[test]
    fn what_stays_open_as_the_lists_change() {
        let mut downloads = Downloads {
            list: vec![running(1, 0, None), running(2, 0, None)],
            exports: vec![exporting(7, 0, 1)],
            ..Downloads::default()
        };
        assert_eq!(
            downloads.kept(Some(Row::Download(2))),
            Some(Some(Row::Download(2)))
        );
        assert_eq!(downloads.kept(Some(Row::Download(3))), Some(None));
        assert_eq!(
            downloads.kept(Some(Row::Export(7))),
            Some(Some(Row::Export(7)))
        );
        assert_eq!(downloads.kept(Some(Row::Export(1))), Some(None));
        downloads.list.clear();
        assert_eq!(downloads.kept(None), Some(None));
        downloads.exports.clear();
        assert_eq!(downloads.kept(None), None);
    }

    #[test]
    fn an_export_that_ends_says_so() {
        let before = [exporting(1, 0, 7), exporting(2, 0, 1), exporting(3, 0, 1)];
        let mut done = before[0].clone();
        done.state = ExportState::Done(ExportSummary {
            rooms: 7,
            messages: 12_340,
            ..Default::default()
        });
        let mut failed = before[1].clone();
        failed.state = ExportState::Failed {
            message: "Disk full".into(),
            summary: ExportSummary::default(),
        };
        let mut cancelled = before[2].clone();
        cancelled.state = ExportState::Cancelled(ExportSummary::default());
        let said: Vec<String> = ended(&before, &[done.clone(), failed, cancelled])
            .into_iter()
            .map(|message| match message {
                Message::Finished(text) => format!("done: {text}"),
                Message::Report(text) => format!("failed: {text}"),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(
            said,
            [
                "done: Exported Ice sheets: 7 rooms, 12\u{a0}340 messages",
                "failed: Could not export Ice sheets: Disk full"
            ]
        );
        // Said once.
        assert!(ended(&[done.clone()], &[done]).is_empty());
    }
}
