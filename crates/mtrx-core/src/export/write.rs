//! What the writers of the three formats share: the trait, what they are
//! given, and helpers.

use std::{
    fs::File,
    io::{self, BufWriter},
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{DateTime, Local};
use matrix_sdk::ruma::{OwnedEventId, OwnedRoomId, OwnedUserId};
use mtrx_format::Document;
use serde_json::value::RawValue;

use super::{ExportFormat, ExportRange, ExportSummary, Line, event::Style};
use crate::{MediaKind, human_size};

/// Writes an export in one format: the rooms' events as they come, oldest
/// first, and the index. Synchronous and buffered: it runs in a blocking
/// thread.
pub(super) trait Writer: Send {
    fn begin_room(&mut self, room: &RoomHead) -> io::Result<()>;
    /// Called oldest first, with its edits, reactions, quote and file.
    fn event(&mut self, event: &Rendered) -> io::Result<()>;
    /// The room's files are complete.
    fn end_room(&mut self, end: &RoomEnd) -> io::Result<()>;
    /// The index, replacing the one there: written at the start, after
    /// each room and at the end.
    fn index(&mut self, index: &Index) -> io::Result<()>;
}

/// A room as its files begin.
pub(super) struct RoomHead {
    pub id: OwnedRoomId,
    pub name: String,
    pub topic: Option<String>,
    /// Its folder in the export's.
    pub folder: String,
    pub about: About,
}

/// About the whole export, for the index and each room's file.
#[derive(Clone)]
pub(super) struct About {
    /// The space's or room's name.
    pub title: String,
    /// "Alice (@alice:example.org)".
    pub exported_by: String,
    pub exported_at: SystemTime,
    pub format: ExportFormat,
    pub range: ExportRange,
    /// Files up to this size were included, if any were.
    pub files: Option<Option<u64>>,
}

impl About {
    /// "HTML · messages since 1 November 2026 · files up to 100 MB".
    pub fn options(&self) -> String {
        let range = match self.range {
            ExportRange::All => "all messages".to_owned(),
            ExportRange::Since(since) => format!("messages since {}", long_day(since)),
            ExportRange::Last(n) => format!(
                "the last {} of each room",
                super::count(n.into(), "message")
            ),
        };
        let files = match self.files {
            None => "without files".to_owned(),
            Some(None) => "with files".to_owned(),
            Some(Some(max)) => format!("files up to {}", human_size(max)),
        };
        format!("{} · {range} · {files}", self.format.name())
    }

    /// "Exported by Alice (@alice:example.org) on 7 November 2026 at 14:03".
    pub fn exported(&self) -> String {
        let at = DateTime::<Local>::from(self.exported_at);
        format!(
            "Exported by {} on {} at {}",
            self.exported_by,
            long_day(self.exported_at),
            at.format("%H:%M")
        )
    }
}

/// How a room's files end.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum RoomEnd {
    Complete,
    /// The export was cancelled, or failed, while in it.
    Stopped,
    /// Its events could not be read to the end: why, in English.
    Failed(String),
}

/// An event as written: with its edit, reactions, the message it answers
/// and its file applied.
pub(super) struct Rendered {
    pub id: Option<OwnedEventId>,
    pub sender: OwnedUserId,
    pub sender_name: String,
    pub time: SystemTime,
    pub shown: Shown,
    pub reply: Option<Quote>,
    /// The thread's first message, if it is in one.
    pub thread: Option<OwnedEventId>,
    /// Each key and how many reacted with it, in the order first used.
    pub reactions: Vec<(String, u32)>,
    /// The event as sent, decrypted (JSON).
    pub raw: Option<Box<RawValue>>,
}

pub(super) enum Shown {
    Message {
        style: Style,
        document: Document,
        edited: bool,
        file: Option<Attached>,
    },
    /// Why, if known.
    Undecryptable(Option<String>),
    Deleted,
    /// After the sender's name: "joined".
    Change(String),
    /// After the sender's name: "sent a sticker".
    Other(String),
    /// In JSON only.
    Hidden,
}

/// A message's file, saved or left out.
pub(super) struct Attached {
    pub kind: MediaKind,
    pub name: String,
    pub size: Option<u64>,
    /// Its path from the room's folder ("files/2026-11-07 report.pdf"), or
    /// why it was left out.
    pub saved: Result<String, String>,
}

/// The message a reply answers.
pub(super) struct Quote {
    pub id: OwnedEventId,
    /// Its sender's name and text, if it was written shortly before.
    pub original: Option<(String, String)>,
}

/// The index: the export's rooms, how far each got, and what it wrote.
pub(super) struct Index<'a> {
    pub about: &'a About,
    pub topic: Option<&'a str>,
    pub space: bool,
    pub tree: &'a [Line],
    pub rooms: &'a [IndexRoom],
    pub state: IndexState<'a>,
    pub summary: &'a ExportSummary,
}

/// A room in the index.
#[derive(Clone)]
pub(super) struct IndexRoom {
    pub id: OwnedRoomId,
    pub name: String,
    /// Its folder; `None` if not joined.
    pub folder: Option<String>,
    pub progress: RoomProgress,
    pub messages: u64,
    pub files: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum RoomProgress {
    NotJoined,
    /// Not reached (yet).
    Waiting,
    /// Being written now (or when Mtrx quit).
    Writing,
    Complete,
    Stopped,
    Failed(String),
}

impl IndexRoom {
    /// "120 messages, 3 files", "Not joined", "Stopped after 40 messages".
    pub fn status(&self) -> String {
        let counts = || {
            let mut text = super::count(self.messages, "message");
            if self.files > 0 {
                text = format!("{text}, {}", super::count(self.files, "file"));
            }
            text
        };
        match &self.progress {
            RoomProgress::NotJoined => "Not joined".into(),
            RoomProgress::Waiting => "Not exported".into(),
            RoomProgress::Writing => "Not finished".into(),
            RoomProgress::Complete => counts(),
            RoomProgress::Stopped => format!("Stopped after {}", counts()),
            RoomProgress::Failed(reason) => format!("Failed after {}: {reason}", counts()),
        }
    }
}

/// How far the export is, as the index says.
#[derive(Clone, Copy)]
pub(super) enum IndexState<'a> {
    /// Running, or stopped when Mtrx quit.
    Unfinished,
    Done,
    Cancelled,
    Failed(&'a str),
}

impl IndexState<'_> {
    pub fn describe(&self, summary: &ExportSummary, space: bool) -> String {
        match self {
            IndexState::Unfinished => {
                "Not finished: still running, or Mtrx quit while exporting.".into()
            }
            IndexState::Done => format!("{}.", summary.describe(space)),
            IndexState::Cancelled => format!("Cancelled: {}.", summary.describe(space)),
            IndexState::Failed(why) => format!("Failed: {why}. {}.", summary.describe(space)),
        }
    }
}

/// A file being written, buffered.
pub(super) fn create(path: &Path) -> io::Result<BufWriter<File>> {
    Ok(BufWriter::with_capacity(
        crate::fetch::BUFFER,
        File::create(path)?,
    ))
}

/// Write `contents` to `path` whole: to a part file first, then renamed,
/// so `path` is never half written.
pub(super) fn replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    let part: PathBuf = crate::fetch::part_path(path);
    std::fs::write(&part, contents)?;
    std::fs::rename(&part, path)
}

/// "Saturday 7 November 2026", local.
pub(super) fn long_day(at: SystemTime) -> String {
    DateTime::<Local>::from(at)
        .format("%A %-d %B %Y")
        .to_string()
}

/// "2026-11-07", local.
pub(super) fn day(at: SystemTime) -> String {
    DateTime::<Local>::from(at).format("%Y-%m-%d").to_string()
}

/// "14:03", local.
pub(super) fn clock(at: SystemTime) -> String {
    DateTime::<Local>::from(at).format("%H:%M").to_string()
}

/// "2026-11-07T14:03:00+01:00", local.
pub(super) fn iso(at: SystemTime) -> String {
    DateTime::<Local>::from(at).to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}
