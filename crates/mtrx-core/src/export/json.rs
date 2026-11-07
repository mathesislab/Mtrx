//! JSON: a file per room as Element writes its `export.json`, `{room_id,
//! room_name, topic, exported_by, export_date, messages: [events as sent,
//! decrypted]}`, then where the files went and how it ended; and an index.
//! Events are written as they come, so a file stays valid JSON however
//! the room ends.

use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use serde_json::{Map, Value, json};

use super::{
    Line,
    write::{self, Index, IndexState, Rendered, RoomEnd, RoomHead, RoomProgress, Writer},
};

pub(super) struct JsonWriter {
    root: PathBuf,
    room: Option<Room>,
}

struct Room {
    out: BufWriter<File>,
    events: u64,
    /// Each saved file's event and path.
    files: Map<String, Value>,
}

impl JsonWriter {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
            room: None,
        }
    }
}

impl Writer for JsonWriter {
    fn begin_room(&mut self, room: &RoomHead) -> io::Result<()> {
        let folder = self.root.join(&room.folder);
        std::fs::create_dir_all(&folder)?;
        let mut out = write::create(&folder.join("messages.json"))?;
        let head = json!({
            "room_id": room.id,
            "room_name": room.name,
            "topic": room.topic,
            "exported_by": room.about.exported_by,
            "export_date": write::iso(room.about.exported_at),
        });
        // The head's fields, then the messages as they come.
        let head = serde_json::to_string(&head)?;
        let head = head.strip_suffix('}').unwrap_or(&head);
        out.write_all(format!("{head},\"messages\":[").as_bytes())?;
        self.room = Some(Room {
            out,
            events: 0,
            files: Map::new(),
        });
        Ok(())
    }

    fn event(&mut self, event: &Rendered) -> io::Result<()> {
        let Some(room) = &mut self.room else {
            return Ok(());
        };
        let Some(raw) = &event.raw else {
            return Ok(());
        };
        if room.events > 0 {
            room.out.write_all(b",")?;
        }
        room.out.write_all(b"\n")?;
        room.out.write_all(raw.get().as_bytes())?;
        room.events += 1;
        if let (Some(id), Some(Ok(path))) = (&event.id, file_path(event)) {
            room.files
                .insert(id.to_string(), Value::String(path.clone()));
        }
        Ok(())
    }

    fn end_room(&mut self, end: &RoomEnd) -> io::Result<()> {
        let Some(mut room) = self.room.take() else {
            return Ok(());
        };
        let (status, reason) = match end {
            RoomEnd::Complete => ("complete", None),
            RoomEnd::Stopped => ("stopped", None),
            RoomEnd::Failed(why) => ("failed", Some(why)),
        };
        let mut tail = Map::new();
        tail.insert(
            "files".into(),
            Value::Object(std::mem::take(&mut room.files)),
        );
        tail.insert("status".into(), status.into());
        if let Some(reason) = reason {
            tail.insert("error".into(), reason.as_str().into());
        }
        let tail = serde_json::to_string(&Value::Object(tail))?;
        let tail = tail.strip_prefix('{').unwrap_or(&tail);
        room.out.write_all(format!("\n],{tail}\n").as_bytes())?;
        room.out.flush()
    }

    fn index(&mut self, index: &Index) -> io::Result<()> {
        let about = index.about;
        let (state, error) = match index.state {
            IndexState::Unfinished => ("unfinished", None),
            IndexState::Done => ("done", None),
            IndexState::Cancelled => ("cancelled", None),
            IndexState::Failed(why) => ("failed", Some(why)),
        };
        let tree: Vec<Value> = (index.tree.iter())
            .filter_map(|line| {
                Some(match line {
                    Line::Space { depth, name } => json!({"depth": depth, "space": name}),
                    Line::Room { depth, room } => {
                        let room = index.rooms.get(*room)?;
                        json!({
                            "depth": depth,
                            "room_id": room.id,
                            "name": room.name,
                            "file": room.folder.as_ref().map(|f| format!("{f}/messages.json")),
                            "status": progress(&room.progress),
                            "error": match &room.progress {
                                RoomProgress::Failed(why) => Some(why),
                                _ => None,
                            },
                            "messages": room.messages,
                            "files": room.files,
                        })
                    }
                })
            })
            .collect();
        let summary = index.summary;
        let value = json!({
            "title": about.title,
            "topic": index.topic,
            "space": index.space,
            "exported_by": about.exported_by,
            "export_date": write::iso(about.exported_at),
            "options": about.options(),
            "state": state,
            "error": error,
            "summary": {
                "rooms": summary.rooms,
                "messages": summary.messages,
                "files": summary.files,
                "files_left_out": summary.files_left_out,
                "undecryptable": summary.undecryptable,
            },
            "rooms": tree,
        });
        let mut bytes = serde_json::to_vec_pretty(&value)?;
        bytes.push(b'\n');
        write::replace(&self.root.join("index.json"), &bytes)
    }
}

/// Where the event's file went, if it has one.
fn file_path(event: &Rendered) -> Option<&Result<String, String>> {
    match &event.shown {
        write::Shown::Message {
            file: Some(file), ..
        } => Some(&file.saved),
        _ => None,
    }
}

fn progress(progress: &RoomProgress) -> &'static str {
    match progress {
        RoomProgress::NotJoined => "not_joined",
        RoomProgress::Waiting => "not_exported",
        RoomProgress::Writing => "unfinished",
        RoomProgress::Complete => "complete",
        RoomProgress::Stopped => "stopped",
        RoomProgress::Failed(_) => "failed",
    }
}
