//! Plain text: a file per room, a line per event (`2026-11-07 14:03  Alice:
//! text`, local time), and an index.

use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use super::{
    Line,
    event::Style,
    write::{self, Attached, Index, Rendered, RoomEnd, RoomHead, Shown, Writer},
};
use crate::human_size;

/// Continued lines of an event start with this.
const INDENT: &str = "    ";

pub(super) struct TextWriter {
    root: PathBuf,
    room: Option<Room>,
}

struct Room {
    out: BufWriter<File>,
    events: u64,
}

impl TextWriter {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
            room: None,
        }
    }
}

impl Writer for TextWriter {
    fn begin_room(&mut self, room: &RoomHead) -> io::Result<()> {
        let folder = self.root.join(&room.folder);
        std::fs::create_dir_all(&folder)?;
        let mut out = write::create(&folder.join("messages.txt"))?;
        let mut head = format!("{}\n", room.name);
        if let Some(topic) = room.topic.as_deref().filter(|t| !t.trim().is_empty()) {
            head.push_str(&format!("{}\n", topic.trim()));
        }
        head.push_str(&format!(
            "{}\n{}\n\n",
            room.about.exported(),
            room.about.options()
        ));
        out.write_all(head.as_bytes())?;
        self.room = Some(Room { out, events: 0 });
        Ok(())
    }

    fn event(&mut self, event: &Rendered) -> io::Result<()> {
        let Some(room) = &mut self.room else {
            return Ok(());
        };
        let Some(text) = event_text(event) else {
            return Ok(());
        };
        room.events += 1;
        room.out.write_all(text.as_bytes())
    }

    fn end_room(&mut self, end: &RoomEnd) -> io::Result<()> {
        let Some(mut room) = self.room.take() else {
            return Ok(());
        };
        let note = match end {
            RoomEnd::Complete if room.events == 0 => "No messages in this range.\n".to_owned(),
            RoomEnd::Complete => String::new(),
            RoomEnd::Stopped => "\nExport stopped here.\n".to_owned(),
            RoomEnd::Failed(why) => format!("\nNot all of this room could be exported: {why}\n"),
        };
        room.out.write_all(note.as_bytes())?;
        room.out.flush()
    }

    fn index(&mut self, index: &Index) -> io::Result<()> {
        let about = index.about;
        let mut text = format!("{}\n", about.title);
        if let Some(topic) = index.topic.filter(|t| !t.trim().is_empty()) {
            text.push_str(&format!("{}\n", topic.trim()));
        }
        text.push_str(&format!(
            "{}\n{}\n{}\n\n",
            about.exported(),
            about.options(),
            index.state.describe(index.summary, index.space)
        ));
        for line in index.tree {
            match line {
                Line::Space { depth, name } => {
                    text.push_str(&format!("{}{name}\n", "  ".repeat(*depth)))
                }
                Line::Room { depth, room } => {
                    let Some(room) = index.rooms.get(*room) else {
                        continue;
                    };
                    let file = (room.folder.as_ref())
                        .map(|folder| format!(" ({folder}/messages.txt)"))
                        .unwrap_or_default();
                    text.push_str(&format!(
                        "{}- {}{file}: {}\n",
                        "  ".repeat(*depth),
                        room.name,
                        room.status()
                    ));
                }
            }
        }
        write::replace(&self.root.join("index.txt"), text.as_bytes())
    }
}

/// An event's lines, ending with a line break; `None` if not shown.
pub(super) fn event_text(event: &Rendered) -> Option<String> {
    let name = &event.sender_name;
    let quote = event.reply.as_ref().map(|quote| match &quote.original {
        Some((sender, preview)) => format!("(in reply to {sender}: {preview})"),
        None => "(in reply to a message from earlier)".into(),
    });
    let text = match &event.shown {
        Shown::Hidden => return None,
        Shown::Message {
            style,
            document,
            edited,
            file,
        } => {
            let mut text = document.to_plain_text();
            if *edited {
                text.push_str(" (edited)");
            }
            if let Some(file) = file {
                let file = attached(file);
                text = if text.trim().is_empty() {
                    file
                } else {
                    format!("{text}\n{file}")
                };
            }
            match style {
                Style::Emote => format!("* {name} {text}"),
                Style::Text | Style::Notice => format!("{name}: {text}"),
            }
        }
        Shown::Undecryptable(why) => format!(
            "{name}: [Unable to decrypt this message{}]",
            why.as_ref().map(|w| format!(": {w}")).unwrap_or_default()
        ),
        Shown::Deleted => format!("{name}: [Message deleted]"),
        Shown::Change(what) | Shown::Other(what) => format!("{name} {what}"),
    };
    let mut lines = text.lines();
    let stamp = format!("{} {}  ", write::day(event.time), write::clock(event.time));
    let mut out = format!("{stamp}{}\n", lines.next().unwrap_or_default());
    for line in lines.chain(quote.as_deref()) {
        out.push_str(INDENT);
        out.push_str(line);
        out.push('\n');
    }
    if !event.reactions.is_empty() {
        let reactions: Vec<String> = (event.reactions.iter())
            .map(|(key, n)| format!("{key} {n}"))
            .collect();
        out.push_str(&format!("{INDENT}[{}]\n", reactions.join(" · ")));
    }
    Some(out)
}

/// "[file: files/report.pdf]", or why it was left out.
fn attached(file: &Attached) -> String {
    match &file.saved {
        Ok(path) => format!("[file: {path}]"),
        Err(why) => {
            let size = file
                .size
                .map(|size| format!(", {}", human_size(size)))
                .unwrap_or_default();
            format!("[file left out: {}{size}: {why}]", file.name)
        }
    }
}
