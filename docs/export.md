# Export chats

Goal: export a room, or a whole space (its rooms and subspaces' rooms) in one go, as HTML, plain text or JSON, with their files. The export is a core job with progress, cancellable, owned by the session like downloads. The desktop offers the dialog and shows progress; the phones can use it later through FFI.

Rules that apply (from `docs/downloads.md`, `docs/app-structure.md`):
- Shared state lives in core and reaches the apps as finished state (`Live<T>`). Core returns English text.
- Stream to disk; never hold a room's history in memory.
- The desktop part is one component module with its own `Message`; menus are `menu::Entry` lists.

Paths: `core/` = `crates/mtrx-core/src/`, `format/` = `crates/mtrx-format/src/`, `app/` = `crates/mtrx-app/src/`, `sdk/` = `~/.cargo/registry/src/index.crates.io-*/matrix-sdk-0.19.1/src/`, `element/` = `element-hq/element-web: apps/web/src/utils/exportUtils/`.

---

## What Element does

Room menu → "Export chat" (`element/Exporter.ts`, `exportUtils.ts`, `HtmlExport.tsx`, `PlainTextExport.ts`, `JSONExport.ts`):
- **Options:** Format (HTML, Plain text, JSON); Messages (Current timeline, From the beginning, Specify a number of messages, at most 10⁸); Size limit (1 MB to 8 GB, a total for all attachments; once reached, later files are left out with a note); Include attachments.
- **Fetching:** `/messages` backwards, 1000 events a page, every event kept in memory, then reversed and all decrypted at once. "Current timeline" takes what is loaded.
- **Output:** a zip built in memory (JSZip), saved as `<brand> - <room> - Chat Export - <date>.zip`, holding one folder:
  - HTML: `messages.html`, `messages2.html`, … with page links, `css/style.css`, `js/script.js`, `room.png`, the senders' avatars.
  - JSON: `export.json` = `{room_name, room_creator, topic, export_date, exported_by, messages: [raw events]}`.
  - Plain text: `export.txt`, lines `<date> - <sender>: <text>`. Without attachments it saves the `.txt` alone.
  - Files in `images/`, `videos/`, `audio/`, `files/`, `stickers/`, named by event ID.
- **Progress:** text only ("Fetched 2000 events so far", "Processing event 12 out of 3000", "Generating a ZIP"). Cancel stops at the next check and saves nothing.
- One room at a time. No space export. A large room can exhaust the tab's memory.

## Today

**Core**
- History is read only through the SDK UI `Timeline` (`core/timeline.rs:349-363`): `paginate_back(events)` (`:436-444`) and `seek` (`:449-466`, at most `SEEK_PAGES` × `SEEK_PAGE` = 20 × 50). The timeline keeps every item it loaded, so it can't stream a whole room.
- The converter works on SDK timeline items (`timeline.rs:1244-1330`). Its helpers take ruma's `MessageType` and can be shared: `message_kind` (`:1457`), `media` (`:1471`), `html_body` (`:1538`). `notifications::body` (`core/notifications.rs:276`) reads a raw event.
- `sdk/room/mod.rs:612` `Room::messages(MessagesOptions)` sends `/messages` and decrypts each event (`try_decrypt_event`). It returns `Messages{start, end, chunk: Vec<TimelineEvent>, state}`. `MessagesOptions::backward()` without `from` starts at the newest event (`sdk/room/messages.rs:88-99`).
- The SDK's client retries transient errors and 429 with the server's `retry_after` (`sdk/http_client/native.rs:118-127`).
- `RoomKeys::download` (`core/encryption.rs:652`) fetches a room's keys from the backup; `Session::timeline` calls it for encrypted rooms (`timeline.rs:933-937`).
- Downloads (`core/downloads.rs`, PR 84):
  - `Shared::fetch` (`:668-695`): media-store shortcut, else stream to `.part`; encrypted files stream their ciphertext to the cache, then `decrypt` (`:806-841`, `spawn_blocking`, checks the hash). `place` renames atomically.
  - `stream` (`:708-731`): `chunk()` reading, `Throttle` steps, a 60 s stall timeout. `response` (`:735-765`) refreshes an expired token once.
  - These are tied to `Job` and `DownloadId` (progress goes into the downloads list), so an export can't call them as they are.
  - `safe_name` (`:964`), `unique_path` (`:1034`), `downloads_folder` (`:409-430`), `human_size` (`:221`) can be shared.
- `SpaceGraph` (`core/space_graph.rs`): `tree(space)` (`:119`) gives everything below a space depth first in each space's order, with depths; `descendants` (`:89`). Joined spaces only; cycles handled.

**Format**
- `Document::to_html` (`format/copy.rs:29`): our document model back to HTML, escaped, math as `$…$` text, mentions as `matrix.to` links. `Document::to_plain_text` (`format/document.rs:207`).
- `escape(text, attribute)` (`format/markdown.rs:187`) is `pub(crate)`.
- `Document::from_html` keeps link URLs as sent: no scheme check (`format/document.rs:76`).
- Typesetting (`render` feature, Typst) is for images in the desktop; core doesn't enable it.

**Desktop**
- Menus: the space bar's space menu `sidebar::space_entries` (`app/sidebar/mod.rs:1102`): Overview, Invite people…, Space settings…, Leave the space…. The room list's `room_entries` (`:964`).
- The space overview's Rooms tab has "New room…" and "Add yours…" buttons (`app/overview.rs:268-278`).
- Dialogs: `dialog::Kind` (`app/dialog.rs:29`) opened by `modal::Message::OpenDialog`. The room checklists (`dialog.rs:~900-1000`, add existing rooms, category rooms) have a search field, ticks and "Choose all shown".
- `send_later.rs` has a calendar (month grid) for picking a day.
- Downloads popover (`app/downloads.rs`): `Popup::Downloads { menu }` (`app/app.rs:241`), the icon with a progress ring at the bottom of the space bar (`sidebar/mod.rs:216, 322`), `files.rs` "Show in folder".

**Dependencies (`Cargo.lock`)**
- In core already: `serde_json`, `chrono`, `reqwest`, `tokio` (`fs`, `io-util`), `mime_guess`.
- No `zip` crate (nor `async_zip`, `tar`). `crc32fast` and `flate2` are in the lock only as others' dependencies.
- No HTML-escaping crate needed: `format/markdown.rs:187` does it.

---

## Core: new `core/export/`

```rust
pub type ExportId = u64;

#[derive(Clone, Debug, PartialEq)]
pub struct ExportRequest {
    pub of: ExportOf,
    pub format: ExportFormat,
    pub range: ExportRange,
    pub files: ExportFiles,
    pub to: SaveTo,                  // Downloads, or File(folder) (Q2); Cache for phones later
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExportOf {
    Room(OwnedRoomId),
    /// The rooms ticked, in the space's tree order; subspaces themselves have no messages.
    Space { space: OwnedRoomId, rooms: Vec<OwnedRoomId> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat { Html, PlainText, Json }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExportRange {
    All,
    Since(SystemTime),
    /// The newest this many events of each room.
    Last(u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportFiles {
    pub include: bool,
    /// Files larger than this are left out, with a note (Q5).
    pub max_file: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExportState {
    Running { room: String, rooms_done: usize, rooms: usize, messages: u64, files: u64, room_fraction: Option<f32> },
    Done(ExportSummary),
    /// What was written stays (Q6).
    Failed { message: String, summary: ExportSummary },
    Cancelled(ExportSummary),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportSummary {
    pub rooms: usize,
    pub messages: u64,
    pub files: u64,
    pub files_left_out: u64,         // too large, or failed to download
    pub undecryptable: u64,
    pub rooms_failed: Vec<(String, String)>,   // room name, English reason
}

#[derive(Clone, Debug, PartialEq)]
pub struct Export {
    pub id: ExportId,
    pub title: String,               // the space's or room's name
    pub format: ExportFormat,
    pub folder: PathBuf,             // known at start; "Show in folder" works while running
    pub started: SystemTime,
    pub state: ExportState,
}
impl Export {
    pub fn is_running(&self) -> bool;
    pub fn fraction(&self) -> Option<f32>;   // (rooms_done + room_fraction) / rooms
    pub fn status(&self) -> String;          // "Lab chat · 3 of 7 rooms · 4 210 messages",
                                             // "7 rooms, 12 340 messages, 85 files", "Cancelled after 3 of 7 rooms"
}

impl Session {
    /// Start an export. It's in `exports()` at once.
    pub fn export(&self, request: ExportRequest) -> Result<ExportId>;
    pub fn exports(&self) -> Live<Vec<Export>>;     // newest first
    pub fn cancel_export(&self, id: ExportId);
    pub fn remove_export(&self, id: ExportId);      // from the list; the folder stays
}
```

**Session**
- Gains `exports: Exports`, like `downloads` (`core/session.rs:57`): a `watch::Sender<Vec<Export>>` read through `Live::from_watch`, and one `AbortOnDrop` task per running export.
- In memory only: an export isn't resumed after quitting. The folder stays, its index says it stopped. The list keeps running exports and the 10 newest others.

**Shared media fetching** (refactor first, no behaviour change)
- New `core/fetch.rs`: `fetch_file(client, &MediaRef, size, target, cache, on_progress: impl FnMut(u64, Option<u64>)) -> Result<PathBuf, String>`, taken out of `Shared::{fetch, stored, stream, response}`. Downloads call it with a progress closure that updates their list; exports with one that counts bytes.
- `safe_name`, `unique_path`, `downloads_folder` become `pub(crate)` there.
- A 429 on media waits for `Retry-After` and tries again, up to 3 times (the SDK does this for API requests, not for our `reqwest` calls).

**How an export runs**
1. The folder is created at once (`unique_path` in the destination), with `<name>.part` while running; renamed to the final name when done or stopped (Q6).
2. Rooms one after another, in the space's tree order. Not joined rooms are listed in the index as "Not joined", not read.
3. Per room:
   1. Encrypted: `RoomKeys::download` first, so keys from the backup are there.
   2. **Fetch pass, newest first:** `Room::messages(MessagesOptions::backward())`, 500 events a page, `from` = the previous `end`, until no `end`, the range's limit (`Last(n)`), or an event older than `Since(t)`. Each event is converted (below) and appended to a spool file `<folder>/.spool/<n>.jsonl`; the byte offset of each page's start is kept (a `Vec<u64>`). Edits (`m.replace`) and reactions come before their targets in this order, so two maps keep them: target → newest edit content, target → reaction counts. Their size grows with the edits and reactions, not the messages.
   3. **Write pass, oldest first:** pages read from the spool last to first, each reversed, so the writer gets events in time order with one page in memory. Edits and reactions are applied from the maps; edit, reaction and redaction events are skipped. Replies show their original's sender and preview from a bounded map of the last 2000 messages written; older ones say "a message from earlier".
   4. Files, if included and not over `max_file`: `fetch_file` into `<room>/files/` as the message is written, one at a time. A failure or a file left out is noted on the message and counted.
   5. The spool is deleted.
4. A room that fails (forbidden, the server unreachable after the SDK's retries) is noted in its file and in the index; the export goes on with the next room. Only a write error (disk full) fails the whole export.
5. Cancelling aborts the task. Open files are closed with a line "Export stopped here", the index is written for what's there.
6. Progress steps at most 10 a second (`Throttle`). `room_fraction`: `Last(n)`: events / n; `Since(t)`: how far the oldest event read is between now and t; `All`: none.

**Conversion** (`core/export/event.rs`)
- `ExportEvent { event_id, sender, sender_name, timestamp, content: ExportContent, reply_to: Option<OwnedEventId>, thread: Option<OwnedEventId>, raw: Option<Box<RawValue>> }`; `raw` only for JSON.
- From `TimelineEvent`: deserialize as `AnySyncTimelineEvent`. Messages reuse `message_kind`, `media` and `html_body` (made `pub(crate)` in `timeline.rs`), so the document is `Document::from_html` as in the timeline.
- Undecryptable (`m.room.encrypted` still): "Unable to decrypt this message" with the cause's text where the SDK gives one. Counted in the summary.
- State events: joins and leaves, name, topic and pin changes as short lines, as the timeline shows them. Others are left out (JSON keeps all).
- Sender names: the current member names, looked up once per sender per room.

**Writers** (`core/export/{html,text,json}.rs`), one trait:
```rust
trait Writer {
    fn begin_room(&mut self, room: &RoomInfo) -> io::Result<()>;
    fn event(&mut self, event: &Rendered) -> io::Result<()>;   // with edits, reactions, reply preview, file path applied
    fn end_room(&mut self, end: RoomEnd) -> io::Result<()>;    // Complete, Stopped, Failed(reason)
    fn finish(self, rooms: &[RoomEntry], summary: &ExportSummary) -> io::Result<()>;  // the index
}
```
- Buffered (`BufWriter`) and synchronous, inside `spawn_blocking` per page, so the async runtime isn't held up by formatting.
- HTML:
  - The body is `Document::to_html()`, rebuilt from our model, not the sender's HTML.
  - Links keep only `https`, `http`, `mailto` and `matrix.to`; others become text (`Document::from_html` doesn't check schemes).
  - A `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src 'self'; media-src 'self'; style-src 'self'">` in every page: no script can run.
  - Math stays `$…$` LaTeX in `<span class="math">` (Q8). Images inline (`<img loading="lazy">`), video and audio with `<video>`/`<audio controls>`, other files as links.
  - Avatars: initials in a coloured circle, CSS only.
  - `format::escape` becomes `pub` for names, topics and timestamps.
- Plain text: `2026-11-07 14:03  Alice: text` (local time, `chrono`), `Document::to_plain_text()`, files as `[file: files/report.pdf]`.
- JSON: per room `{room_id, room_name, topic, exported_by, export_date, messages: [` then each decrypted raw event, written as it comes, then `]}`. Same shape as Element's `export.json`, plus `room_id`.

## Output layout (folder, Q1)

```
Mtrx – Ice sheets – 2026-11-07/       (unique in the destination: "… (2)")
  index.html | index.txt | index.json   the space, its topic, the tree of rooms with
                                        depth, counts, links, "Not joined" / failures,
                                        exported by and when, the options used
  style.css                             (HTML)
  lab-chat/                             one folder per room: safe_name of its name,
    messages.html, messages-2.html …    made unique; 2000 events per page, page links
    messages.txt | messages.json
    files/2026-11-07 report.pdf         date + safe sender name, made unique
  field-trip/
    …
```
- Flat: one folder per room, whatever the depth. The index shows subspaces as headings in the tree. A room in two subspaces is exported once and linked from both.
- A single room uses the same layout with one room, so the code has one path.

## Desktop

**Where**
- The space bar's space menu: "Export chats…" (after "Space settings…").
- The space overview's Rooms tab: an "Export…" button beside "Add yours…".
- The room list's room menu: "Export chat…".
- All open `dialog::Kind::Export { of: ExportOf, title: String }`.

**The dialog** (`app/export.rs`, own `Message`; shown by the modal like other dialogs)
- Title: "Export Ice sheets" / "Export Lab chat".
- **Rooms** (space only): the checklist from `dialog.rs` with depth indents from `SpaceGraph::tree`, all joined rooms ticked, not joined ones shown disabled, "Choose all shown".
- **Format:** HTML · Plain text · JSON (segmented, HTML first).
- **Messages:** All · Since [day] (the `send_later` calendar in a popover) · Last [n] per room (text field, default 1000).
- **Files:** "Include files" (on), "Leave out files over [100] MB".
- **Save to:** "Downloads" with "Change…" (`rfd` folder picker).
- Buttons: Cancel, Export. Export starts the job and closes the dialog.

**Progress** (Q7)
- The downloads popover gets an "Exports" section above the downloads, from `session.exports()` (one more subscription beside the downloads one). The space-bar icon shows while either list isn't empty; its ring is the fraction over both.
- A row: the format's icon, the title, `status()`, a progress bar (empty while the fraction is unknown).
- Running: ✕ cancels (no confirmation; what's written stays). Finished: a click opens the index in the browser (`open::that_detached`), a folder button shows it in the folder. Right-click: "Open", "Show in folder", "Remove from list".
- When it ends, a notice: "Exported Ice sheets: 7 rooms, 12 340 messages" or the failure.

## FFI (later, when a phone wants it)

`Session.export(request) -> u64`, `exports()`, `subscribe_exports(listener) -> TaskHandle`, `cancel_export`, `remove_export`; records mirroring `Export` with `fraction` and `status` filled. Phones would use `SaveTo::Cache` and share the result; a folder can't go through a share sheet to mail, so this is where Q1 (b) may come back.

## Tests

- Core unit:
  - conversion of message kinds, an edit and a reaction applied to their target, a redaction, an undecryptable event;
  - the spool: pages written newest first come out oldest first;
  - writers: HTML escaping, a `javascript:` link becomes text, the CSP line, page splitting at 2000, JSON stays valid when stopped mid-room;
  - `status()` and `fraction()`.
- Core integration (CI Synapse, `test-support`):
  - a space with a subspace, two rooms (one encrypted), messages, an edit, a reaction, a reply and a file: export as HTML and JSON, check the files, the edited text, the decrypted message, the file's bytes, the index's tree;
  - `Last(n)` and `Since(t)` limits;
  - a room with more than one page (1200 events);
  - cancel during a room: the folder has the finished rooms and "Export stopped here";
  - a room we left: listed as not joined.
- Desktop (`app/app/tests/`): the space menu, overview and room menu items open the dialog; the dialog builds the `ExportRequest` from its options (pure function); the popover's export rows and their menu entries.

## PR split

One PR, four commits, each building (CI only):
1. Core: `fetch.rs` taken out of `downloads.rs` (no behaviour change; the download tests pass as they are).
2. Core: `export/` (job, spool, conversion, writers), `Session.exports`, `format::escape` public, unit and integration tests.
3. Desktop: the dialog, the three entry points, the exports section in the popover.
4. Docs: "As built" here.

Size: M–L. About 900 lines of core (a third writers), 400 desktop.

---

## Open questions

1. **Folder or zip.**
   - (a) A folder. No new dependency; the HTML opens straight from it; files are written once. Finder or Explorer can compress it.
   - (b) A zip like Element's, with the `zip` crate: a **new dependency** (needs your OK). Written at the end from the folder (each file read twice), or as it goes (store-only entries).
   - (c) A zip without a crate: a small store-only zip writer in core (~150 lines, our own CRC-32). No compression, but no dependency.
   - Recommend (a). Revisit (b) when a phone shares exports.
2. **Where it's saved.**
   - (a) The Downloads folder, with "Change…" in the dialog to pick another.
   - (b) A folder picker every time.
   - Recommend (a), as downloads do (downloads Q3a).
3. **Formats.**
   - (a) HTML, plain text and JSON, as Element.
   - (b) HTML and JSON.
   - (c) HTML only.
   - Recommend (a). The writers share everything but their output, and each serves someone (reading, grepping, scripts).
4. **Which messages.**
   - (a) All · Since a day · Last n per room.
   - (b) Element's: Current timeline · From the beginning · Last n. "Current timeline" means nothing for a space.
   - Recommend (a).
5. **File size limit.**
   - (a) Per file: "Leave out files over N MB" (default 100).
   - (b) Element's total cap (default 8 MB): files after it are left out, so the first rooms get theirs and the last don't.
   - (c) Both.
   - Recommend (a). A total cap depends on room order in a space export.
6. **Cancel and failure.**
   - (a) Keep what's written: finished rooms whole, the current one ending "Export stopped here", the index written.
   - (b) Delete the folder, as a cancelled download deletes its `.part`.
   - Recommend (a). A space export can run for long; what it fetched is worth keeping, and the user can delete the folder.
7. **Where progress shows.**
   - (a) An "Exports" section in the downloads popover; the icon's ring covers both.
   - (b) The dialog stays open with a progress bar until done.
   - (c) A row of its own in the space bar.
   - Recommend (a). One place for long-running jobs, and the app stays usable.
8. **Math in HTML.**
   - (a) LaTeX as `$…$` text, styled monospace: offline, readable, copyable.
   - (b) Typeset to SVG with Typst: core would need mtrx-format's `render` feature (large; fonts).
   - (c) KaTeX from a CDN: needs the internet and a script, which the CSP forbids.
   - Recommend (a).
9. **Reactions.**
   - (a) Included, as "👍 2 · 🎉 1" under the message (from the fetch pass's map).
   - (b) Left out, as Element's plain text does.
   - Recommend (a). The map is small and they carry meaning ("agreed").

## Decisions (2026-11-07)

The user took all recommendations: 1a, 2a, 3a, 4a, 5a, 6a, 7a, 8a, 9a. No new dependencies.

## As built

Differences from the design above:

**Core**
- `fetch.rs`: `fetch_file(client, Fetch { source, size, target, ciphertext, unique, limit }, on_progress)`. The caller gives the ciphertext's place in the cache (downloads by ID, exports in `<cache>/exporting/`). `limit` stops a file that turns out larger than the export's limit when the sender gave no size.
- `Export` also has `rooms` (the rooms to export, for "Cancelled after 3 of 7 rooms") and `index` (its index file). `ExportSummary::describe` gives the summary line. `Error::Export` says why one can't start (no Downloads folder).
- `remove_export` on a running export stops it first, writing what it has. `SaveTo::Cache` puts exports in `<cache>/exports/`.
- Rooms we were never in are named from the space's listing (`Listings::name`), else their ID.
- **Folders:** a room's folder is its name made safe (slashes as `_`), unique whatever the letters' case and apart from the index's files: `Lab chat/`, not `lab-chat/`. The export's folder drops dots from the title, so `unique_path` adds " (1)" at its end.
- **Index:** written at the start, after each room and at the end, each time whole (to a part file, then renamed). One left by a quit says "Not finished". Each room's line says how far it got: "120 messages, 3 files", "Not joined", "Stopped after …", "Failed after …: why", "Not exported".
- **A room that can't be read to its start** keeps what was read (the newest part), ending "Not all of this room could be exported: why", instead of nothing.
- **Ranges:** `Last(n)` counts messages (deleted and undecryptable ones too), not the room's changes, which come along. JSON also keeps the edits and reactions among them.
- **Edits** apply only from the original's sender; the newest wins. Reactions count each person's key once, most used first.
- **HTML:** the policy is `default-src 'none'; img-src 'self' file:; media-src 'self' file:; style-src 'self' file:; base-uri 'none'; form-action 'none'`, with `<meta name="referrer" content="no-referrer">`: `'self'` doesn't match a `file://` page in every browser. ruma-html already drops some schemes as it parses; `Document::to_page_html` keeps `http`, `https` and `mailto` links (`matrix.to` is `https`). Pages also have day headings, a sender's messages within 5 minutes grouped under one name, "In a thread" marks, and spoilers in `spoiler` spans.
- **Plain text:** a reply's quote is a line after the text, "(in reply to Bob: …)"; reactions "[👍 2 · 🎉 1]"; continued lines indented by 4.
- **JSON:** each room's file ends with `files` (event ID → path) and `status` (`complete`, `stopped`, `failed` with `error`). `index.json` has the options, state, summary and the tree with each room's file and status.
- No FFI yet (as planned).

**Desktop**
- The dialog is a modal of its own (`modal::Message::OpenExport(export::Target, title)`, `modal::Shown::Export`), not a `dialog::Kind`: it shares nothing with `Dialog`'s state.
- The calendar shows below "Since a day" in the dialog, not in a popover: the dialog's body scrolls. Its month grid is `calendar.rs`, now also the send-later picker's.
- Format and Messages are rows of buttons. "Leave out files over [100] MB" left empty means no limit.
- The rooms checklist lists the rooms the room list knows; ones not joined are shown disabled. Rooms never seen aren't listed (the index lists them as not joined).
- "Export…" in the overview's Rooms tab shows for everyone: exporting needs no rights. "Export chat…" is the room menu's last item.
- The popover: an "Exports" heading over rows "Ice sheets · HTML", then "Downloads". Running: ✕ stops it. Finished: a click opens the index, the folder button shows the folder. Right-click: "Open" (finished), "Show in folder", "Remove from list". "Clear" also removes finished exports. `Popup::Downloads { menu: Option<downloads::Row> }`.
- The icon's ring averages the downloads (as one) and each export whose progress is known. Its hint names both.
- An export that ends says "Exported Ice sheets: 7 rooms, 12 340 messages" (a failure as a problem); a cancelled one says nothing.

**Tests**
- No 1200-event integration test: sending that many is slow on CI. Paging is covered by unit tests (the spool's pages come back oldest first; HTML splits at 2000).
- The undecryptable message is an `m.room.encrypted` event of a session no device has, posted through the API: matrix-sdk shares a room's history keys with those invited, and keys reach a new device too, so neither gives a dependable one.
- The cancel test cancels at once, so it always lands while running: the folder renamed, valid JSON files, the index "cancelled", the list entry removed while the folder stays.
- Desktop: the dialog's request (pure), the space and room menus' items, the popover's export entries, notices and progress, and opening the dialog, ⌘↩ starting an export and Esc (UI test).
