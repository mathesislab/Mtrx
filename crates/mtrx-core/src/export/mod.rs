//! Exporting chats: a room, or a space's rooms, as HTML, plain text or
//! JSON, with their files, into a folder.
//!
//! An export is a job of the session, like a download: it's in
//! [`Session::exports`] with its progress at once, and can be cancelled.
//! What it wrote stays when it is cancelled or fails: the rooms finished,
//! the one it was in ending "Export stopped here", and the index.
//!
//! It streams: each room's events are fetched newest first into a spool
//! file on disk, then read back a page at a time, oldest first, and
//! written out. What it keeps in memory grows with a room's edits and
//! reactions, not its messages. See [`job`].

mod event;
mod html;
mod job;
mod json;
mod spool;
mod text;
mod write;

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

use matrix_sdk::{
    Client, RoomState,
    ruma::{OwnedRoomId, RoomId},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, watch};

use crate::{
    AbortOnDrop, Error, Live, Result, SaveTo, Session, SpaceGraph,
    fetch::{downloads_folder, safe_name, unique_path},
};

pub type ExportId = u64;

/// The finished, failed and cancelled exports the list keeps, newest
/// first; it keeps every running one too.
const KEEP: usize = 10;

/// What to export, how, and where to.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportRequest {
    pub of: ExportOf,
    pub format: ExportFormat,
    pub range: ExportRange,
    pub files: ExportFiles,
    /// The folder the export's folder goes in: Downloads, a folder chosen
    /// ([`SaveTo::File`]), or the profile's cache (phones, to share).
    pub to: SaveTo,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExportOf {
    Room(OwnedRoomId),
    /// The rooms chosen, below `space`; subspaces themselves have no
    /// messages. The index shows them in the space's order.
    Space {
        space: OwnedRoomId,
        rooms: Vec<OwnedRoomId>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat {
    Html,
    PlainText,
    Json,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 3] = [Self::Html, Self::PlainText, Self::Json];

    /// "HTML", "Plain text", "JSON".
    pub fn name(self) -> &'static str {
        match self {
            Self::Html => "HTML",
            Self::PlainText => "Plain text",
            Self::Json => "JSON",
        }
    }

    /// Of its files: "html", "txt", "json".
    pub fn extension(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::PlainText => "txt",
            Self::Json => "json",
        }
    }
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Which of each room's messages.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExportRange {
    All,
    Since(SystemTime),
    /// The newest this many messages of each room (with the room's
    /// changes among them).
    Last(u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportFiles {
    pub include: bool,
    /// Files larger than this are left out, with a note.
    pub max_file: Option<u64>,
}

impl Default for ExportFiles {
    /// Files up to 100 MB.
    fn default() -> Self {
        Self {
            include: true,
            max_file: Some(100_000_000),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExportState {
    Running {
        /// The room being exported.
        room: String,
        rooms_done: usize,
        rooms: usize,
        messages: u64,
        files: u64,
        /// How far the room is, when that can be known.
        room_fraction: Option<f32>,
    },
    Done(ExportSummary),
    /// What was written stays.
    Failed {
        message: String,
        summary: ExportSummary,
    },
    Cancelled(ExportSummary),
}

/// What an export wrote.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportSummary {
    /// Rooms written to their end.
    pub rooms: usize,
    pub messages: u64,
    pub files: u64,
    /// Too large, or failed to download.
    pub files_left_out: u64,
    /// Messages that could not be decrypted.
    pub undecryptable: u64,
    /// Rooms that could not be read to the end: the room's name, and why in
    /// English.
    pub rooms_failed: Vec<(String, String)>,
}

/// An export ([`Session::exports`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Export {
    pub id: ExportId,
    /// The space's or room's name.
    pub title: String,
    pub format: ExportFormat,
    /// Where it's written, known at the start; renamed when it ends (from
    /// "… .part").
    pub folder: PathBuf,
    /// Its index, in the folder.
    pub index: PathBuf,
    pub started: SystemTime,
    /// The rooms to export.
    pub rooms: usize,
    pub state: ExportState,
}

impl Export {
    pub fn is_running(&self) -> bool {
        matches!(self.state, ExportState::Running { .. })
    }

    /// From 0 to 1, while it runs, when that can be known: the rooms done
    /// and how far the current one is.
    pub fn fraction(&self) -> Option<f32> {
        let ExportState::Running {
            rooms_done,
            rooms,
            room_fraction,
            ..
        } = self.state
        else {
            return None;
        };
        if rooms == 0 || (rooms == 1 && room_fraction.is_none()) {
            return None;
        }
        let done = rooms_done as f32 + room_fraction.unwrap_or(0.0);
        Some((done / rooms as f32).clamp(0.0, 1.0))
    }

    /// A line about it: "Lab chat · 3 of 7 rooms · 4 210 messages", "7
    /// rooms, 12 340 messages, 85 files", "Cancelled after 3 of 7 rooms",
    /// "Failed: …".
    pub fn status(&self) -> String {
        match &self.state {
            ExportState::Running {
                room,
                rooms_done,
                rooms,
                messages,
                files,
                ..
            } => {
                let mut parts = Vec::new();
                if *rooms > 1 {
                    parts.push(room.clone());
                    parts.push(format!("{} of {rooms} rooms", (rooms_done + 1).min(*rooms)));
                }
                match (messages, files) {
                    (0, 0) if *rooms <= 1 => parts.push("Starting…".into()),
                    (0, 0) => {}
                    (messages, 0) => parts.push(count(*messages, "message")),
                    (messages, files) => {
                        parts.push(count(*messages, "message"));
                        parts.push(count(*files, "file"));
                    }
                }
                parts.join(" · ")
            }
            ExportState::Done(summary) => summary.describe(self.rooms > 1),
            ExportState::Failed { message, .. } => format!("Failed: {message}"),
            ExportState::Cancelled(summary) if self.rooms > 1 => {
                format!("Cancelled after {} of {} rooms", summary.rooms, self.rooms)
            }
            ExportState::Cancelled(_) => "Cancelled".into(),
        }
    }

    /// What it wrote, once it ended.
    pub fn summary(&self) -> Option<&ExportSummary> {
        match &self.state {
            ExportState::Running { .. } => None,
            ExportState::Done(summary)
            | ExportState::Failed { summary, .. }
            | ExportState::Cancelled(summary) => Some(summary),
        }
    }

    /// An export of `title`, running, for frontends' tests.
    #[cfg(feature = "test-support")]
    pub fn example(id: ExportId, title: &str, state: ExportState) -> Self {
        Self {
            id,
            title: title.into(),
            format: ExportFormat::Html,
            folder: format!("/tmp/{title}").into(),
            index: format!("/tmp/{title}/index.html").into(),
            started: SystemTime::UNIX_EPOCH,
            rooms: match &state {
                ExportState::Running { rooms, .. } => *rooms,
                _ => 1,
            },
            state,
        }
    }
}

impl ExportSummary {
    /// "7 rooms, 12 340 messages, 85 files", with what went wrong; the
    /// rooms only if `rooms`.
    pub fn describe(&self, rooms: bool) -> String {
        let mut parts = Vec::new();
        if rooms {
            parts.push(count(self.rooms as u64, "room"));
        }
        parts.push(count(self.messages, "message"));
        if self.files > 0 {
            parts.push(count(self.files, "file"));
        }
        if self.files_left_out > 0 {
            parts.push(format!("{} left out", count(self.files_left_out, "file")));
        }
        if self.undecryptable > 0 {
            parts.push(format!("{} not decrypted", grouped(self.undecryptable)));
        }
        if !self.rooms_failed.is_empty() {
            parts.push(format!(
                "{} failed",
                count(self.rooms_failed.len() as u64, "room")
            ));
        }
        parts.join(", ")
    }
}

/// "1 message", "4 210 messages".
pub(crate) fn count(n: u64, what: &str) -> String {
    let s = if n == 1 { "" } else { "s" };
    format!("{} {what}{s}", grouped(n))
}

/// `n` with its thousands apart: "12 340" (with a no-break space).
pub(crate) fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{a0}');
        }
        out.push(digit);
    }
    out
}

impl Session {
    /// Start an export. It's in [`Session::exports`] at once, with its
    /// folder made.
    pub fn export(&self, request: ExportRequest) -> Result<ExportId> {
        self.exports.start(
            &self.client,
            self.room_keys.clone(),
            &self.spaces.get(),
            request,
        )
    }

    /// The exports, newest first.
    pub fn exports(&self) -> Live<Vec<Export>> {
        Live::from_watch(self.exports.list.subscribe())
    }

    /// Stop an export; what it wrote stays, the index saying it stopped.
    pub fn cancel_export(&self, id: ExportId) {
        self.exports.cancel(id);
    }

    /// Take an export off the list (stopping it if it runs); its folder
    /// stays.
    pub fn remove_export(&self, id: ExportId) {
        self.exports.remove(id);
    }
}

/// A session's exports: the list, and a task per running export.
pub(crate) struct Exports {
    list: Arc<watch::Sender<Vec<Export>>>,
    running: Mutex<HashMap<ExportId, Running>>,
    next_id: AtomicU64,
    /// The user's Downloads folder, if the platform has one.
    downloads: Option<PathBuf>,
    /// The profile's cache directory.
    cache: PathBuf,
}

/// A running export's task, and how to stop it.
struct Running {
    stop: Option<oneshot::Sender<()>>,
    _task: AbortOnDrop,
}

impl Exports {
    pub(crate) fn new(cache: &Path) -> Self {
        Self {
            list: Arc::new(watch::Sender::new(Vec::new())),
            running: Default::default(),
            next_id: AtomicU64::new(1),
            downloads: downloads_folder(),
            cache: cache.to_owned(),
        }
    }

    fn start(
        &self,
        client: &Client,
        room_keys: Arc<crate::encryption::RoomKeys>,
        graph: &SpaceGraph,
        request: ExportRequest,
    ) -> Result<ExportId> {
        let plan = Plan::new(client, graph, &request.of)?;
        let destination = match &request.to {
            SaveTo::Downloads => (self.downloads.clone())
                .ok_or_else(|| Error::Export("There is no Downloads folder".into()))?,
            SaveTo::File(folder) => folder.clone(),
            SaveTo::Cache => self.cache.join("exports"),
        };
        std::fs::create_dir_all(&destination)?;
        let started = SystemTime::now();
        let name = folder_name(&plan.title, started);
        let folder = unique_path(&destination, &name, |path| {
            path.exists() || part_folder(path).exists()
        });
        let writing = part_folder(&folder);
        std::fs::create_dir(&writing)?;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let rooms = plan.exported();
        let export = Export {
            id,
            title: plan.title.clone(),
            format: request.format,
            index: writing.join(format!("index.{}", request.format.extension())),
            folder: writing.clone(),
            started,
            rooms,
            state: ExportState::Running {
                room: plan.first_room().unwrap_or_default(),
                rooms_done: 0,
                rooms,
                messages: 0,
                files: 0,
                room_fraction: None,
            },
        };
        let job = job::Job::new(job::Setup {
            id,
            client: client.clone(),
            room_keys,
            list: self.list.clone(),
            request,
            plan,
            writing: writing.clone(),
            folder,
            started,
            ciphertext: self.cache.join("exporting").join(format!("{id}.part")),
        });
        let job = job.inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&writing);
        })?;
        self.list.send_modify(|list| {
            list.insert(0, export);
            trim(list);
        });
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(job.run(stopped));
        let mut running = self.running.lock().expect("not poisoned");
        running.retain(|_, r| !r._task.0.is_finished());
        running.insert(
            id,
            Running {
                stop: Some(stop),
                _task: AbortOnDrop(task),
            },
        );
        Ok(id)
    }

    fn cancel(&self, id: ExportId) {
        let mut running = self.running.lock().expect("not poisoned");
        // It writes what it has, then ends as cancelled.
        if let Some(stop) = running.get_mut(&id).and_then(|r| r.stop.take()) {
            let _ = stop.send(());
        }
    }

    fn remove(&self, id: ExportId) {
        // A running one stops, writing what it has, and isn't listed again.
        self.cancel(id);
        self.list.send_if_modified(|list| {
            let before = list.len();
            list.retain(|e| e.id != id);
            list.len() != before
        });
    }
}

/// The running exports and the [`KEEP`] newest others, of a list newest
/// first.
fn trim(list: &mut Vec<Export>) {
    let mut others = 0;
    list.retain(|e| {
        if e.is_running() {
            return true;
        }
        others += 1;
        others <= KEEP
    });
}

/// `path` with `.part` added: where an export is written until it ends.
fn part_folder(path: &Path) -> PathBuf {
    crate::fetch::part_path(path)
}

/// "Mtrx – Ice sheets – 2026-11-07", safe as a folder name.
fn folder_name(title: &str, at: SystemTime) -> String {
    let day = chrono::DateTime::<chrono::Local>::from(at).format("%Y-%m-%d");
    // Without a dot, so no part of the title is taken as an extension.
    let title = title.replace('.', " ");
    safe_folder(&format!("Mtrx – {title} – {day}"))
}

/// A name as a folder's: its slashes kept as `_`, made safe.
fn safe_folder(name: &str) -> String {
    safe_name(&name.replace(['/', '\\'], "_"), None)
}

/// What an export goes through: the rooms in the order they're written,
/// and the index's tree.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Plan {
    /// The space's or room's name.
    title: String,
    /// The space's topic, or the room's.
    topic: Option<String>,
    /// For the index: subspaces as headings, and the rooms, by their place
    /// in `rooms`. A room in two subspaces is listed under both.
    tree: Vec<Line>,
    /// Each room once, in tree order.
    rooms: Vec<PlannedRoom>,
    /// It's a space's.
    space: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Line {
    Space { depth: usize, name: String },
    Room { depth: usize, room: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlannedRoom {
    id: OwnedRoomId,
    name: String,
    /// Its folder in the export's; `None` if not joined (not exported).
    folder: Option<String>,
}

impl Plan {
    fn new(client: &Client, graph: &SpaceGraph, of: &ExportOf) -> Result<Self> {
        let name = |id: &RoomId| {
            client
                .get_room(id)
                .and_then(|room| room.cached_display_name())
                .map(|name| name.to_string())
                .unwrap_or_else(|| id.to_string())
        };
        let joined = |id: &RoomId| {
            client
                .get_room(id)
                .is_some_and(|room| room.state() == RoomState::Joined)
        };
        let mut plan = match of {
            ExportOf::Room(id) => {
                let room = client
                    .get_room(id)
                    .ok_or_else(|| Error::RoomNotFound(id.to_string()))?;
                Self {
                    title: name(id),
                    topic: room.topic(),
                    tree: vec![Line::Room { depth: 0, room: 0 }],
                    rooms: vec![PlannedRoom {
                        id: id.clone(),
                        name: name(id),
                        folder: joined(id).then(String::new),
                    }],
                    space: false,
                }
            }
            ExportOf::Space { space, rooms } => {
                let chosen: HashSet<&OwnedRoomId> = rooms.iter().collect();
                let mut plan = Self {
                    title: name(space),
                    topic: client.get_room(space).and_then(|room| room.topic()),
                    tree: Vec::new(),
                    rooms: Vec::new(),
                    space: true,
                };
                let is_space = |id: &RoomId| {
                    !graph.children(id).is_empty()
                        || client.get_room(id).is_some_and(|room| room.is_space())
                };
                let mut listed = HashSet::new();
                for (depth, id) in graph.tree(space) {
                    let line = if is_space(&id) {
                        Line::Space {
                            depth,
                            name: name(&id),
                        }
                    } else if chosen.contains(&id) || !joined(&id) {
                        listed.insert(id.clone());
                        Line::Room {
                            depth,
                            room: plan.add(&id, name(&id), joined(&id)),
                        }
                    } else {
                        continue;
                    };
                    plan.tree.push(line);
                }
                // Chosen, though the space doesn't list it (any more).
                for id in rooms.iter().filter(|id| !listed.contains(*id)) {
                    let room = plan.add(id, name(id), joined(id));
                    plan.tree.push(Line::Room { depth: 0, room });
                }
                plan.prune();
                plan
            }
        };
        plan.name_folders();
        Ok(plan)
    }

    /// `id` in `rooms`, added once; its place.
    fn add(&mut self, id: &OwnedRoomId, name: String, joined: bool) -> usize {
        if let Some(at) = self.rooms.iter().position(|r| r.id == *id) {
            return at;
        }
        self.rooms.push(PlannedRoom {
            id: id.clone(),
            name,
            folder: joined.then(String::new),
        });
        self.rooms.len() - 1
    }

    /// Subspaces with no rooms listed below them go.
    fn prune(&mut self) {
        let mut keep = vec![true; self.tree.len()];
        for (i, line) in self.tree.iter().enumerate() {
            if let Line::Space { depth, .. } = line {
                let below = self.tree[i + 1..]
                    .iter()
                    .take_while(|l| l.depth() > *depth)
                    .any(|l| matches!(l, Line::Room { .. }));
                keep[i] = below;
            }
        }
        let mut keep = keep.into_iter();
        self.tree.retain(|_| keep.next().unwrap_or(true));
    }

    /// Each exported room's folder: its name made safe, unique among the
    /// others and the index's files, whatever the letters' case.
    fn name_folders(&mut self) {
        let mut taken: HashSet<String> = ["index.html", "index.txt", "index.json", "style.css"]
            .map(str::to_owned)
            .into();
        for room in &mut self.rooms {
            let Some(folder) = &mut room.folder else {
                continue;
            };
            let base = safe_folder(&room.name);
            let mut name = base.clone();
            let mut n = 1;
            while !taken.insert(name.to_lowercase()) {
                n += 1;
                name = format!("{base} ({n})");
            }
            *folder = name;
        }
    }

    /// The rooms to export: the joined ones.
    fn exported(&self) -> usize {
        self.rooms.iter().filter(|r| r.folder.is_some()).count()
    }

    fn first_room(&self) -> Option<String> {
        (self.rooms.iter())
            .find(|r| r.folder.is_some())
            .map(|r| r.name.clone())
    }
}

impl Line {
    fn depth(&self) -> usize {
        match self {
            Line::Space { depth, .. } | Line::Room { depth, .. } => *depth,
        }
    }
}

#[cfg(test)]
mod tests;
