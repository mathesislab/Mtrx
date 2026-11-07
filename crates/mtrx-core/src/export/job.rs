//! How an export runs, one room after another:
//!
//! 1. **Fetch pass, newest first:** `/messages` backwards, a page at a time,
//!    until the room's start or the range's limit. Each event is read
//!    ([`event::convert`]) and appended to a spool file on disk. Edits and
//!    reactions come before their targets in this order, so they are kept
//!    in maps until their targets are written.
//! 2. **Write pass, oldest first:** the spool's pages read back last to
//!    first, each reversed, so the writer gets events in time order with
//!    one page in memory. Files are fetched as their messages are written.
//!
//! A room that can't be read to its start (forbidden, the server
//! unreachable) has what was read written, with a note, and the export
//! goes on; only a write error (a full disk) fails the export.
//! Cancelling drops the work at its next step, then closes the open room's
//! file with "Export stopped here" and writes the index for what's there.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
    time::{Instant, SystemTime},
};

use matrix_sdk::{
    Client, Room,
    room::MessagesOptions,
    ruma::{OwnedEventId, OwnedRoomId, OwnedUserId, UInt},
};
use mtrx_format::Document;
use tokio::sync::{oneshot, watch};
use tracing::warn;

use super::{
    Export, ExportFiles, ExportFormat, ExportId, ExportRange, ExportRequest, ExportState,
    ExportSummary, Plan,
    event::{self, Body, Content, FileRef, Item, Related},
    html::HtmlWriter,
    json::JsonWriter,
    spool::Spool,
    text::TextWriter,
    trim,
    write::{
        About, Attached, Index, IndexRoom, IndexState, Quote, Rendered, RoomEnd, RoomHead,
        RoomProgress, Shown, Writer,
    },
};
use crate::{
    encryption::RoomKeys,
    fetch::{Fetch, STEP, Throttle, fetch_file, safe_name, taken, unique_path},
    human_size,
};

/// Events asked for at once.
const PAGE: u64 = 500;
/// Messages remembered for the replies that answer them.
const REMEMBERED: usize = 2000;
/// A reply's quote of its original is cut to this many characters.
const PREVIEW: usize = 140;

/// What an export starts with.
pub(super) struct Setup {
    pub id: ExportId,
    pub client: Client,
    pub room_keys: Arc<RoomKeys>,
    pub list: Arc<watch::Sender<Vec<Export>>>,
    pub request: ExportRequest,
    pub plan: Plan,
    /// Where it's written while it runs.
    pub writing: PathBuf,
    /// Where it goes when it ends.
    pub folder: PathBuf,
    pub started: SystemTime,
    /// Where an encrypted file's ciphertext goes while it downloads.
    pub ciphertext: PathBuf,
}

pub(super) struct Job {
    id: ExportId,
    client: Client,
    room_keys: Arc<RoomKeys>,
    list: Arc<watch::Sender<Vec<Export>>>,
    format: ExportFormat,
    range: ExportRange,
    files: ExportFiles,
    plan: Plan,
    writing: PathBuf,
    folder: PathBuf,
    ciphertext: PathBuf,
    about: About,
    /// For the index, by their place in the plan.
    rooms: Vec<IndexRoom>,
    summary: ExportSummary,
    /// Shared with the blocking threads it writes in; it outlives a write
    /// cancelled midway, which the end waits for.
    writer: Arc<Mutex<Box<dyn Writer>>>,
    progress: Progress,
}

/// What [`ExportState::Running`] says.
struct Progress {
    room: String,
    rooms_done: usize,
    rooms: usize,
    messages: u64,
    files: u64,
    room_fraction: Option<f32>,
    steps: Throttle,
}

/// How the export ended.
enum Outcome {
    Done,
    Cancelled,
    Failed(String),
}

/// What the fetch pass read of a room.
struct Fetched {
    spool: Spool,
    relations: Relations,
    names: HashMap<OwnedUserId, String>,
    /// Why it couldn't read further back, in English.
    failure: Option<String>,
}

/// Edits and reactions read in the fetch pass, until their targets are
/// written.
#[derive(Default)]
pub(super) struct Relations {
    /// The newest text of each message, by each who edited it (only its
    /// sender's counts).
    pub(super) edits: HashMap<(OwnedEventId, OwnedUserId), Body>,
    reactions: HashMap<OwnedEventId, Vec<(String, u32)>>,
    /// Who reacted with what, counted once.
    reacted: HashSet<(OwnedEventId, OwnedUserId, String)>,
}

impl Relations {
    pub(super) fn add(&mut self, related: Related) {
        match related {
            // The first seen is the newest.
            Related::Edit {
                target,
                sender,
                body,
            } => {
                self.edits.entry((target, sender)).or_insert(body);
            }
            Related::Reaction {
                target,
                sender,
                key,
            } => {
                if !self.reacted.insert((target.clone(), sender, key.clone())) {
                    return;
                }
                let keys = self.reactions.entry(target).or_default();
                match keys.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, n)) => *n += 1,
                    None => keys.push((key, 1)),
                }
            }
        }
    }

    /// The reactions to `id`, most used first.
    pub(super) fn take_reactions(&mut self, id: &OwnedEventId) -> Vec<(String, u32)> {
        let mut reactions = self.reactions.remove(id).unwrap_or_default();
        // Seen newest first: the first used last.
        reactions.reverse();
        reactions.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        reactions
    }
}

/// The latest messages written, for the replies that answer them: their
/// sender's name and text.
pub(super) struct Recent {
    pub(super) quotes: HashMap<OwnedEventId, (String, String)>,
    order: VecDeque<OwnedEventId>,
}

impl Recent {
    pub(super) fn new() -> Self {
        Self {
            quotes: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub(super) fn insert(&mut self, id: OwnedEventId, quote: (String, String)) {
        if self.order.len() == REMEMBERED
            && let Some(oldest) = self.order.pop_front()
        {
            self.quotes.remove(&oldest);
        }
        self.quotes.insert(id.clone(), quote);
        self.order.push_back(id);
    }
}

impl Job {
    pub fn new(setup: Setup) -> io::Result<Self> {
        let Setup {
            id,
            client,
            room_keys,
            list,
            request,
            plan,
            writing,
            folder,
            started,
            ciphertext,
        } = setup;
        let writer: Box<dyn Writer> = match request.format {
            ExportFormat::Html => Box::new(HtmlWriter::new(&writing)?),
            ExportFormat::PlainText => Box::new(TextWriter::new(&writing)),
            ExportFormat::Json => Box::new(JsonWriter::new(&writing)),
        };
        let own = client
            .user_id()
            .map_or_else(|| "?".to_owned(), ToString::to_string);
        let about = About {
            title: plan.title.clone(),
            exported_by: own,
            exported_at: started,
            format: request.format,
            range: request.range,
            files: request.files.include.then_some(request.files.max_file),
        };
        let rooms = (plan.rooms.iter())
            .map(|room| IndexRoom {
                id: room.id.clone(),
                name: room.name.clone(),
                folder: room.folder.clone(),
                progress: match room.folder {
                    Some(_) => RoomProgress::Waiting,
                    None => RoomProgress::NotJoined,
                },
                messages: 0,
                files: 0,
            })
            .collect();
        let progress = Progress {
            room: plan.first_room().unwrap_or_default(),
            rooms_done: 0,
            rooms: plan.exported(),
            messages: 0,
            files: 0,
            room_fraction: None,
            steps: Throttle::new(STEP, Instant::now()),
        };
        Ok(Self {
            id,
            client,
            room_keys,
            list,
            format: request.format,
            range: request.range,
            files: request.files,
            plan,
            writing,
            folder,
            ciphertext,
            about,
            rooms,
            summary: ExportSummary::default(),
            writer: Arc::new(Mutex::new(writer)),
            progress,
        })
    }

    /// Export until done, failed or `stopped`; then write what's there.
    pub async fn run(mut self, stopped: oneshot::Receiver<()>) {
        let outcome = tokio::select! {
            biased;
            _ = stopped => Outcome::Cancelled,
            result = self.export() => match result {
                Ok(()) => Outcome::Done,
                Err(e) => Outcome::Failed(format!("Couldn't write the export: {e}")),
            },
        };
        self.end(outcome).await;
    }

    async fn export(&mut self) -> io::Result<()> {
        self.about.exported_by = self.exported_by().await;
        self.write_index(IndexState::Unfinished).await?;
        for index in 0..self.plan.rooms.len() {
            if self.plan.rooms[index].folder.is_none() {
                continue;
            }
            self.room(index).await?;
            self.progress.rooms_done += 1;
            self.write_index(IndexState::Unfinished).await?;
        }
        Ok(())
    }

    /// "Alice (@alice:example.org)", as the first room knows us.
    async fn exported_by(&self) -> String {
        let Some(own) = self.client.user_id() else {
            return self.about.exported_by.clone();
        };
        let room = (self.plan.rooms.iter())
            .filter(|r| r.folder.is_some())
            .find_map(|r| self.client.get_room(&r.id));
        let name = match room {
            Some(room) => (room.get_member_no_sync(own).await.ok().flatten())
                .and_then(|member| member.display_name().map(ToOwned::to_owned)),
            None => None,
        };
        match name {
            Some(name) => format!("{name} ({own})"),
            None => own.to_string(),
        }
    }

    /// Export the room at `index` in the plan. Only a write error is an
    /// error; a room that can't be read is noted.
    async fn room(&mut self, index: usize) -> io::Result<()> {
        let planned = self.plan.rooms[index].clone();
        let folder = planned.folder.clone().unwrap_or_default();
        self.rooms[index].progress = RoomProgress::Writing;
        self.progress.room = planned.name.clone();
        self.progress.room_fraction = None;
        self.report(true);
        let room = self.client.get_room(&planned.id);
        let head = RoomHead {
            id: planned.id.clone(),
            name: planned.name.clone(),
            topic: room.as_ref().and_then(|room| room.topic()),
            folder: folder.clone(),
            about: self.about.clone(),
        };
        self.with_writer(move |writer| writer.begin_room(&head))
            .await?;
        let failure = match &room {
            Some(room) => self.room_events(room, index, &folder).await?,
            None => Some("Not in the room any more".into()),
        };
        let end = match failure {
            None => RoomEnd::Complete,
            Some(why) => RoomEnd::Failed(why),
        };
        let ended = end.clone();
        self.with_writer(move |writer| writer.end_room(&ended))
            .await?;
        match end {
            RoomEnd::Complete | RoomEnd::Stopped => {
                self.rooms[index].progress = RoomProgress::Complete;
                self.summary.rooms += 1;
            }
            RoomEnd::Failed(why) => {
                self.summary.rooms_failed.push((planned.name, why.clone()));
                self.rooms[index].progress = RoomProgress::Failed(why);
            }
        }
        Ok(())
    }

    /// Fetch and write the room's events; why not all could be read.
    async fn room_events(
        &mut self,
        room: &Room,
        index: usize,
        folder: &str,
    ) -> io::Result<Option<String>> {
        let id: OwnedRoomId = room.room_id().to_owned();
        if room.encryption_state().is_encrypted() {
            self.room_keys.download(&self.client, &id).await;
        }
        let Fetched {
            spool,
            mut relations,
            names,
            failure,
        } = self.fetch(room, index).await?;
        let mut pages = blocking(move || spool.read()).await?;
        let mut recent = Recent::new();
        loop {
            let (back, page) = blocking(move || {
                let page = pages.next_page();
                Ok::<_, io::Error>((pages, page))
            })
            .await?;
            pages = back;
            let Some(page) = page? else {
                break;
            };
            let mut rendered = Vec::with_capacity(page.len());
            for item in page {
                let item = self
                    .render(item, index, folder, &mut relations, &names, &mut recent)
                    .await;
                rendered.push(item);
            }
            self.with_writer(move |writer| {
                rendered.iter().try_for_each(|event| writer.event(event))
            })
            .await?;
            self.report(false);
        }
        Ok(failure)
    }

    /// The fetch pass: the room's events in the range, newest first, into a
    /// spool; the edits and reactions; the senders' names. A server's
    /// refusal ends it, keeping what was read.
    async fn fetch(&mut self, room: &Room, index: usize) -> io::Result<Fetched> {
        let keep_raw = self.format == ExportFormat::Json;
        let path = self.writing.join(".spool").join(format!("{index}.jsonl"));
        let mut spool = blocking(move || Spool::create(&path)).await?;
        let mut relations = Relations::default();
        let mut names: HashMap<OwnedUserId, String> = HashMap::new();
        let (mut from, mut kept, now) = (None::<String>, 0u64, SystemTime::now());
        let mut failure = None;
        loop {
            let want = match self.range {
                ExportRange::Last(n) => PAGE.min(u64::from(n).saturating_sub(kept)),
                _ => PAGE,
            };
            let mut options = MessagesOptions::backward().from(from.as_deref());
            options.limit = UInt::try_from(want.max(1)).unwrap_or(UInt::MIN);
            let messages = match room.messages(options).await {
                Ok(messages) => messages,
                Err(e) => {
                    failure = Some(server_error(&e));
                    break;
                }
            };
            let mut reached = messages.chunk.is_empty();
            let mut page = Vec::with_capacity(messages.chunk.len());
            let mut oldest = None;
            for timeline_event in &messages.chunk {
                let converted = event::convert(timeline_event, keep_raw);
                let item = converted.item;
                if let ExportRange::Since(since) = self.range
                    && item.time() < since
                {
                    reached = true;
                    break;
                }
                oldest = Some(item.time());
                if let Some(related) = converted.related {
                    relations.add(related);
                }
                if item.is_message() {
                    kept += 1;
                }
                if keep_raw || !matches!(item.content, Content::Hidden) {
                    page.push(item);
                }
                if let ExportRange::Last(n) = self.range
                    && kept >= u64::from(n)
                {
                    reached = true;
                    break;
                }
            }
            for item in &page {
                if !names.contains_key(&item.sender) {
                    let name = member_name(room, &item.sender).await;
                    names.insert(item.sender.clone(), name);
                }
            }
            let messages_read = page.iter().filter(|item| item.is_message()).count() as u64;
            spool = blocking(move || spool.write_page(&page).map(|()| spool)).await?;
            self.progress.messages += messages_read;
            self.progress.room_fraction = match self.range {
                ExportRange::All => None,
                ExportRange::Last(n) => Some((kept as f32 / n.max(1) as f32).min(1.0)),
                ExportRange::Since(since) => oldest.map(|oldest| {
                    let span = now.duration_since(since).unwrap_or_default().as_secs_f32();
                    let done = now.duration_since(oldest).unwrap_or_default().as_secs_f32();
                    if span > 0.0 {
                        (done / span).min(1.0)
                    } else {
                        1.0
                    }
                }),
            };
            self.report(false);
            match messages.end {
                Some(end) if !reached && from.as_deref() != Some(end.as_str()) => from = Some(end),
                _ => break,
            }
        }
        Ok(Fetched {
            spool,
            relations,
            names,
            failure,
        })
    }

    /// `item` as written: its edit, reactions, quote and file applied.
    async fn render(
        &mut self,
        item: Item,
        index: usize,
        folder: &str,
        relations: &mut Relations,
        names: &HashMap<OwnedUserId, String>,
        recent: &mut Recent,
    ) -> Rendered {
        let sender_name = (names.get(&item.sender).cloned())
            .unwrap_or_else(|| item.sender.localpart().to_owned());
        let reactions = (item.id.as_ref())
            .map(|id| relations.take_reactions(id))
            .unwrap_or_default();
        let reply = (item.reply_to.clone()).map(|id| Quote {
            original: recent.quotes.get(&id).cloned(),
            id,
        });
        let time = item.time();
        if item.is_message() {
            self.summary.messages += 1;
            self.rooms[index].messages += 1;
        }
        let shown = match item.content {
            Content::Message(mut body) => {
                let edit = (item.id.clone())
                    .and_then(|id| relations.edits.remove(&(id, item.sender.clone())));
                let edited = edit.is_some();
                if let Some(edit) = edit {
                    // The text changes; the file stays.
                    body.style = edit.style;
                    body.text = edit.text;
                    body.html = edit.html;
                }
                let document = match &body.html {
                    Some(html) => Document::from_html(html),
                    None => Document::from_plain(&body.text),
                };
                if let Some(id) = &item.id {
                    let text = document.to_plain_text();
                    let text = match &body.file {
                        Some(file) if text.trim().is_empty() => file.name.clone(),
                        _ => text,
                    };
                    recent.insert(id.clone(), (sender_name.clone(), preview(&text)));
                }
                let file = match body.file {
                    Some(file) => Some(self.attach(file, time, index, folder).await),
                    None => None,
                };
                Shown::Message {
                    style: body.style,
                    document,
                    edited,
                    file,
                }
            }
            Content::Undecryptable(why) => {
                self.summary.undecryptable += 1;
                Shown::Undecryptable(why)
            }
            Content::Deleted => Shown::Deleted,
            Content::Change(change) => Shown::Change(change),
            Content::Other(what) => Shown::Other(what),
            Content::Hidden => Shown::Hidden,
        };
        Rendered {
            id: item.id,
            sender: item.sender,
            sender_name,
            time,
            shown,
            reply,
            thread: item.thread,
            reactions,
            raw: item.raw,
        }
    }

    /// A message's file, saved into the room's `files` folder, or why not.
    async fn attach(
        &mut self,
        file: FileRef,
        at: SystemTime,
        index: usize,
        folder: &str,
    ) -> Attached {
        let saved = self.save(&file, at, folder).await;
        match &saved {
            Ok(_) => {
                self.summary.files += 1;
                self.rooms[index].files += 1;
                self.progress.files += 1;
                self.report(false);
            }
            Err(_) if self.files.include => self.summary.files_left_out += 1,
            Err(_) => {}
        }
        Attached {
            kind: file.kind,
            name: file.name,
            size: file.size,
            saved,
        }
    }

    async fn save(&self, file: &FileRef, at: SystemTime, folder: &str) -> Result<String, String> {
        if !self.files.include {
            return Err("not included".into());
        }
        let max = self.files.max_file;
        if let (Some(max), Some(size)) = (max, file.size)
            && size > max
        {
            return Err(format!("left out, larger than {}", human_size(max)));
        }
        let name = format!(
            "{} {}",
            super::write::day(at),
            safe_name(&file.name, file.mimetype.as_deref())
        );
        let files = self.writing.join(folder).join("files");
        let target = unique_path(&files, &name, taken);
        let fetch = Fetch {
            source: &file.source,
            size: file.size,
            target: &target,
            ciphertext: self.ciphertext.clone(),
            unique: true,
            limit: max,
        };
        let path = fetch_file(&self.client, fetch, |_, _| {})
            .await
            .map_err(|why| format!("not downloaded: {why}"))?;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or(name);
        Ok(format!("files/{name}"))
    }

    /// Write the index as things are now.
    async fn write_index(&self, state: IndexState<'static>) -> io::Result<()> {
        let about = self.about.clone();
        let topic = self.plan.topic.clone();
        let space = self.plan.space;
        let tree = self.plan.tree.clone();
        let rooms = self.rooms.clone();
        let summary = self.summary.clone();
        self.with_writer(move |writer| {
            writer.index(&Index {
                about: &about,
                topic: topic.as_deref(),
                space,
                tree: &tree,
                rooms: &rooms,
                state,
                summary: &summary,
            })
        })
        .await
    }

    /// Run `work` with the writer, in a blocking thread.
    async fn with_writer<R: Send + 'static>(
        &self,
        work: impl FnOnce(&mut dyn Writer) -> io::Result<R> + Send + 'static,
    ) -> io::Result<R> {
        let writer = self.writer.clone();
        blocking(move || {
            let mut writer = writer.lock().unwrap_or_else(PoisonError::into_inner);
            work(writer.as_mut())
        })
        .await
    }

    /// Tell the list how far it is; at most every [`STEP`] unless `now`.
    fn report(&mut self, now: bool) {
        if !self.progress.steps.due(Instant::now()) && !now {
            return;
        }
        let p = &self.progress;
        let state = ExportState::Running {
            room: p.room.clone(),
            rooms_done: p.rooms_done,
            rooms: p.rooms,
            messages: p.messages,
            files: p.files,
            room_fraction: p.room_fraction,
        };
        self.list.send_if_modified(|list| {
            match list.iter_mut().find(|e| e.id == self.id && e.is_running()) {
                Some(export) if export.state != state => {
                    export.state = state;
                    true
                }
                _ => false,
            }
        });
    }

    /// Close what's open, write the index, and move the folder to its name.
    async fn end(mut self, outcome: Outcome) {
        // A room the export stopped in ends here.
        let ended = self
            .with_writer(|writer| writer.end_room(&RoomEnd::Stopped))
            .await
            .err();
        for room in &mut self.rooms {
            if room.progress == RoomProgress::Writing {
                room.progress = RoomProgress::Stopped;
            }
        }
        let message = match &outcome {
            Outcome::Failed(message) => Some(message.clone()),
            _ => None,
        };
        let index_state = match &outcome {
            Outcome::Done => IndexState::Done,
            Outcome::Cancelled => IndexState::Cancelled,
            Outcome::Failed(_) => IndexState::Failed("couldn't write the export"),
        };
        let written = self.write_index(index_state).await;
        let failure = message.or_else(|| {
            (ended.or(written.err())).map(|e| format!("Couldn't write the export: {e}"))
        });
        let _ = tokio::fs::remove_dir_all(self.writing.join(".spool")).await;
        let _ = tokio::fs::remove_file(&self.ciphertext).await;
        // Kept as it is if it can't be renamed (open elsewhere, on Windows).
        let folder = match tokio::fs::rename(&self.writing, &self.folder).await {
            Ok(()) => self.folder.clone(),
            Err(e) => {
                warn!("could not rename the export's folder: {e}");
                self.writing.clone()
            }
        };
        let summary = self.summary.clone();
        let state = match (failure, outcome) {
            (Some(message), _) => ExportState::Failed { message, summary },
            (None, Outcome::Cancelled) => ExportState::Cancelled(summary),
            (None, _) => ExportState::Done(summary),
        };
        let index = folder.join(format!("index.{}", self.format.extension()));
        self.list.send_if_modified(|list| {
            let Some(export) = list.iter_mut().find(|e| e.id == self.id) else {
                return false;
            };
            export.state = state;
            export.folder = folder;
            export.index = index;
            trim(list);
            true
        });
    }
}

/// Run blocking `work` in a thread of its own.
async fn blocking<R: Send + 'static>(
    work: impl FnOnce() -> io::Result<R> + Send + 'static,
) -> io::Result<R> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(io::Error::other)?
}

/// A sender's name as the room knows them now.
async fn member_name(room: &Room, user: &OwnedUserId) -> String {
    (room.get_member_no_sync(user).await.ok().flatten())
        .map(|member| member.name().to_owned())
        .unwrap_or_else(|| user.localpart().to_owned())
}

/// Why `/messages` failed, in English.
fn server_error(error: &matrix_sdk::Error) -> String {
    use matrix_sdk::ruma::api::error::ErrorKind;
    match error.client_api_error_kind() {
        Some(ErrorKind::Forbidden) => "The server didn't allow reading its messages".into(),
        Some(_) => format!("The server refused: {error}"),
        None => format!("Couldn't reach the server: {error}"),
    }
}

/// The text of a message on one line, cut short.
pub(super) fn preview(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > PREVIEW {
        format!("{}…", text.chars().take(PREVIEW).collect::<String>())
    } else {
        text
    }
}
