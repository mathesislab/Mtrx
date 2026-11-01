# Step 2: the core sends finished state

Goal: the three apps stop rebuilding what is shown from raw core data. Core (Rust) computes it once; desktop uses it directly; FFI passes it to iOS and Android. Where the apps disagree today, iOS wins (user decision 2026-11-01).

Wording (state lines, level confirmations, presence lines) moves into core as English text: the apps are English only, and the three copies are already word-for-word the same. A typed alternative is open question Q6.

---

## 1. Room lists, sections, badges, search

**Today**
- Core: `RoomList` → `RoomListState{rooms, loaded}`; `RoomsBySpace`, `Layout::group`, `space_rooms`, `home_rooms`.
- FFI: `room_list.rs::follow` adds spaces, sections, notes and drafts into `RoomListState`.
- iOS `Models/RoomLists.swift`: chats, notes, `SpaceItem`, Home sections, Rooms tab groups, badges, `unread(besides:)`, `firstSpace`, `chatSpaces`, search, `RoomSort`, sections by phone layout.
- Android `rooms/RoomLists.kt` and `rooms/Sections.kt`: the same again, with no search.
- Desktop `sidebar.rs` (`unread`, `search`, `first_match`, `sections`, `listed`) and `arrange.rs` (`listed`, its own `SpaceMembers` loading).

**Where they disagree** (iOS wins)

| What | iOS | Android | Desktop |
|---|---|---|---|
| Rooms tab badge | invited rooms not counted | invited counted | — |
| Back-button count | excludes invites | includes invites | — |
| Rooms tab groups: invited spaces | left out | shown | — |
| Search | prefix matches first, then by name | none | same as iOS, but leaves out spaces |
| Space unread dot | counts direct chats | counts direct chats | ignores direct chats (#45) → **Q1** |

Android also keeps its own Home sections, the same as iOS's.

**Core: new module `crates/mtrx-core/src/lists.rs`**
```rust
pub struct RoomLists {             // built from one RoomListState + categories + space members + drafts + notes
    pub rooms: Vec<RoomSummary>,  // as now, most recent first
    pub loaded: bool,
    pub notes: Option<OwnedRoomId>,        // Some while pinned at the top
    pub chats: Vec<OwnedRoomId>,           // direct chats, invites to them; not pinned notes
    pub spaces: Vec<SpaceEntry>,           // top-level spaces, then Home if any
    pub drafts: BTreeSet<OwnedRoomId>,
    pub badges: Badges,
}
pub enum SpaceKey { Space(OwnedRoomId), Home }      // saved form "other-rooms" for Home stays an app concern
pub struct SpaceEntry { pub key: SpaceKey, pub rooms: Vec<OwnedRoomId>, pub sections: Vec<(Section, Vec<OwnedRoomId>)>, pub unread: bool }
pub struct Badges { pub chats: u64, pub rooms: u64, pub spaces: u64 }
pub enum RoomSort { ListOrder, AToZ, ZToA, MostRecent }
pub struct SectionLayout { pub order: Vec<String>, pub hidden: BTreeSet<String>, pub folded: BTreeSet<String> } // per device, kept by the app (Q2)
pub struct ListSection { pub key: String, pub title: String, pub rooms: Vec<OwnedRoomId>, pub folded: bool,
                         pub unread: u64, pub mentions: bool, pub space: Option<OwnedRoomId> }

impl RoomLists {
    pub fn space_list(&self, key: &SpaceKey, layout: &SectionLayout, sort: RoomSort, filter: &str) -> Vec<ListSection>;
    pub fn section_choices(&self, key: &SpaceKey, layout: &SectionLayout) -> Vec<(String, String, bool)>;
    pub fn room_groups(&self, among: Option<&BTreeSet<OwnedRoomId>>, folded: &BTreeSet<String>) -> Vec<ListSection>; // Rooms tab
    pub fn search(&self, key: Option<&SpaceKey>, filter: &str) -> Vec<OwnedRoomId>;   // iOS rule; desktop Enter = first
    pub fn unread_besides(&self, room: &RoomId) -> u64;
    pub fn first_space_of(&self, room: &RoomId) -> Option<OwnedRoomId>;
    pub fn chat_spaces(&self, chat: &RoomId) -> Vec<String>;                            // space names
    pub fn top_level_spaces(&self) -> Vec<OwnedRoomId>;
}
impl Session { pub fn room_lists(&self, list: &RoomList) -> impl Stream<Item = Arc<RoomLists>>; } // moves room_list.rs::follow (space members, categories, drafts) into core
```
- `SectionKind` titles ("Invites", "Direct messages" and so on) come from core.
- The desktop's space bar and its nested spaces stay desktop-only. The desktop takes `RoomLists` for sections, search, unread dots and badges, and drops `Arrange::space_members` / `load_space_members`.

**FFI**
- `RoomListListener::on_update(lists: Arc<RoomLists>)`, where `RoomLists` is a `uniffi::Object` with the methods above, returning records:
  - `ListSection{key, title, rooms: Vec<RoomSummary>, folded, unread, mentions, space}`
  - `SpaceEntry{key, space: Option<RoomSummary>, unread}`
  - `Badges`
- Getters: `rooms()`, `room(id)`, `chats()`, `notes()`, `spaces()`, `drafts()`, `badges()`, `loaded()`.
- It's an object, so the apps don't copy the whole list across FFI on every call.
- The `RoomListState` record goes away.

**Deleted**
- iOS: `RoomLists.swift` except `SpaceEntry.saved` and `RoomSort.title` (~250 lines).
- Android: most of `RoomLists.kt`, and `Sections.kt` except `SectionStore`/`SectionLayouts` (~300 lines).
- Desktop: `sidebar::unread`, `search`, `first_match`, the bodies of `sections`/`listed`, and `Arrange::space_members`.

---

## 2. Timeline rows

**Today**
- iOS `TimelineRows.swift` (`build`, `stateSummary`, `MessageRow.actions`) plus Pin/Unpin in `TimelineModel.swift:485`.
- Android `TimelineRows.kt` (`timelineRows`, `stateSummary`, `messageActions`, `menuActions`).
- Desktop `rows.rs::rows`, `room.rs:2288` state summary, `actions.rs::toolbar` rules, and pin in `room.rs:1644`.

**Where they disagree** (iOS wins)
- **Grouping.** A message continues its group when it has the same sender and comes less than 5 minutes after the one before.
  - iOS: day divider, read marker, state line and start all break a group.
  - Android: no time gap, and the read marker doesn't break a group.
  - Desktop: 5 minutes, but only after messages; the read marker is not shown at all → **Q3**.
- **Delete.** iOS offers it for our own sent messages that are not already deleted. The desktop also offers it on deleted ones.
- **Pin.** Each app computes it on its own.
- **"Decrypting…"** while keys arrive: iOS has it, the others don't.

**Core: `crates/mtrx-core/src/rows.rs`**
```rust
pub enum Row {
    Start,
    Day(SystemTime),
    ReadMarker,
    Summary { id: String, text: String },          // id = first event's item id
    Message(MessageRow),
}
pub struct MessageRow {
    pub event: Event,
    pub first_in_group: bool,
    pub show_sender: bool,     // first_in_group && !own && !direct
    pub avatar_column: bool,   // !own && !direct
    pub decrypting: bool,      // UTD while keys arrive
    pub actions: Vec<MessageAction>, // in menu order, Pin/Unpin included
}
pub enum MessageAction { React, Reply, Copy, SelectText, Edit, Pin, Unpin, Delete, Retry, Discard }
pub const GROUP_GAP: Duration = Duration::from_secs(5 * 60);
pub fn rows(entries: &[TimelineEntry], ctx: &RowContext) -> Vec<Row>;   // pure, tested once
pub struct RowContext<'a> { pub direct: bool, pub keys_arriving: bool, pub can_pin: bool, pub pinned: &'a [OwnedEventId] }
impl RoomTimeline { pub fn rows(&self) -> impl Stream<Item = Arc<Vec<Row>>>; } // timeline + pinned + can_pin + encryption
```
- Item ids stay as they are now (`TimelineEntry` id).
- Desktop: `rows.rs` keeps `Part`/selection on top of `core::Row` and uses `show_sender`/`first_in_group` for its header. Copy image, Save image and the math items stay app-side, appended to `actions`.

**FFI**
- `TimelineListener::on_update(rows: Vec<TimelineRow>)`.
- `TimelineRow{id, content: RowContent}`, where `RowContent` = `Start | Day{date} | ReadMarker | Summary{text} | Message{event, first_in_group, show_sender, avatar_column, decrypting, actions}`.
- `TimelineItem` / `TimelineItemContent` go away.

**Deleted**
- iOS: `TimelineRows.build`, `stateSummary`, `MessageRow.actions`, the pin part of `TimelineModel`, and `canPin` loading.
- Android: `timelineRows`, `stateSummary`, `messageActions`, `menuActions`, and its `canPin` load.
- Desktop: `rows::rows`, the `room.rs` state summary, and the action rules in `toolbar`.

---

## 3. seek(event_id)

**Today**
- iOS `TimelineModel.goTo`: 20 pages × 50 events; a second call is ignored while one is under way.
- Android `goToOriginal`: a new call replaces the one under way.
- Desktop `app.rs::go_to`: `GO_TO_PAGES` 20, page size from `paginate`.
- All three use the same "not found" text.

**Core**
```rust
impl RoomTimeline {
    /// Loads older events until `event` is in the timeline, the room's start, or SEEK_PAGES x SEEK_PAGE.
    pub async fn seek(&self, event: &EventId) -> Result<String /* item id */>;   // Error::EventNotFound
    /// Waits (no loading) until `event` is in the timeline: draft replies.
    pub async fn item_of(&self, event: &EventId) -> String;
}
pub const SEEK_PAGES: usize = 20; pub const SEEK_PAGE: u16 = 50;
```
- Pagination is serialised inside core.
- Dropping the future cancels the seek. On the desktop that's the `Task` abort; on the apps it's async cancellation.
- Each app keeps its scroll and its highlight timer, and decides whether to ignore a second tap or replace the first.

**FFI**
- `Timeline::seek(event_id) -> Result<String, MtrxError>`.
- New `MtrxError::EventNotFound`.

**Deleted**
- iOS: `goTo`, `checkSeek`, `endSeek`, the paging in `paginate`, and the `Seek` state.
- Android: `findOriginal`, `loadForOriginal`.
- Desktop: `go_to` paging and `Room.seeking`.

---

## 4. Draft rules

**Today**
Each app re-implements the same rules:
- Load the draft once.
- Don't overwrite text written meanwhile.
- Save after a pause:
  - iOS and Android: 1.5 s.
  - Desktop: `DRAFT_PAUSE` 1 s.
- Don't save it unchanged since the last save.
- Saves go one after another.
- A failed save is retried at the next change.
- A pending reply waits until its row is there.
- During an edit, keep the draft from before it.
- Leaving a room clears its draft on Android only; iOS and the desktop keep it (Q7: keep).

Where:
- iOS `TimelineModel.swift:22-104, 360-430`.
- Android `TimelineViewModel.kt:295-365`.
- Desktop `room.rs:395-420`, `app.rs:869-1000`.

**Core**
```rust
pub struct DraftKeeper { /* room, last kept, save queue, debounce */ }
impl Session { pub fn draft_keeper(&self, room: &RoomId) -> DraftKeeper; }
impl DraftKeeper {
    pub async fn restore(&self) -> Option<Draft>;          // once; None if nothing kept
    pub fn changed(&self, draft: Draft);                    // saved after DRAFT_PAUSE (1.5 s), if not the kept one
    pub async fn flush(&self);                              // now: leaving the room, app to background
}
pub const DRAFT_PAUSE: Duration = Duration::from_millis(1500);
// Drafts are never cleared by leaving (Q7).
```
- "Don't replace text written meanwhile" stays in the app: only the app knows its composer. The app calls `restore()` and applies the result only if the composer is empty.
- A reply is resolved with `timeline.item_of(reply_to)` (item 3).
- During an edit, the app passes the draft from before the edit.

**FFI**
- `Session::draft_keeper(room_id) -> Arc<DraftKeeper>`, with `restore`, `changed`, `flush`.
- `draft` and `save_draft` go away.

**Deleted**
- iOS: `restoreDraft`, `draftChanged`, `saveDraft`, `storedDraft`, `draftSave`.
- Android: `saveNow`, `changed`, `stored`, the `saving` mutex, and `Account.clearDraft` (leaving keeps the draft).
- Desktop: `draft_to_keep`, `kept_draft`, `save_draft`, the `DRAFT_PAUSE` timer, and the `Later::Scheduled` → `save_draft(default)` step (it becomes `keeper.changed(Draft::default())`).

---

## 5. Member level choices and confirmations

**Today**
- iOS `MemberRoles.swift`.
- Android `rooms/Levels.kt`.
- Desktop `members.rs:291-390` and `dialog.rs:620-700`.

**Where they disagree** (iOS wins)
- Android shows levels we may not give, disabled; iOS and the desktop hide them.
- Android doesn't drop "Member" when the default level is 100 or 50.
- Android's "alone" check counts invited members.

**Core: `members.rs`**
```rust
pub struct LevelChoice { pub title: String, pub level: i64, pub current: bool }
pub struct Confirmation { pub title: String, pub message: String, pub button: String }
impl Member {
    pub fn role_label(&self) -> Option<String>;          // "Owner" | "Admin" | "Mod" | "Level 20" | None
    pub fn level_choices(&self) -> Vec<LevelChoice>;     // iOS rules
    pub fn confirmation(&self, level: i64, own: bool, alone: bool) -> Option<Confirmation>;
    pub fn manageable(&self) -> bool;
}
pub fn alone_at_our_level(members: &[Member], own: &UserId) -> bool;   // joined only
pub fn typed_level(text: &str, up_to: i64) -> Option<i64>;
```

**FFI**
- `Member` gains `role_label`, `level_choices` and `manageable`, filled in when it's built. They are records, not methods.
- `MemberList` gains `alone: bool`.
- New `Session::level_confirmation(room_id, user_id, level) -> Option<Confirmation>`.
- `Member.role` goes away. It's unused; this was step 4 anyway.

**Deleted**
- iOS: `MemberRoles` (all of it).
- Android: `Levels.kt` except `memberName`.
- Desktop: `role_label`, the level list in `role_menu`, `LevelChange::role`, and the wording in `dialog.rs`.

---

## 6. Presence activity and a typed `PresenceUnavailable`

**Today**
- Core has `Error::PresenceUnavailable`, but FFI flattens it to `Other{"This server does not share presence"}`, and iOS matches that string (`PresenceModel.unavailableReason`).
- The idle timer, foreground and background handling, the sharing preference and the serialised `setPresence` are all written per app:
  - iOS `PresenceModel.swift:145-260`.
  - Android `model/Presence.kt:130-330`.
  - Desktop `presence.rs:185-240` (`Activity`).
- The presence line exists three times:
  - iOS `PresenceModel.line`.
  - Android `presenceLine`.
  - Core `describe_last_active`, used only by the desktop.

**Where they disagree**
The desktop never goes idle while its window is focused. iOS goes idle after 5 minutes without input in the foreground → **Q4**.

**Core: `presence.rs`**
```rust
pub enum Activity { Foreground, Background, Input, SignOut }
pub struct OwnPresence { /* sharing flag, idle timer, last set, serial queue */ }
impl Session {
    pub fn own_presence(&self) -> &OwnPresence;
}
impl OwnPresence {
    pub async fn activity(&self, a: Activity);       // Foreground: online (sharing) / offline (not); Input: online if idle; Background, SignOut: offline; awaits the set
    pub fn set_sharing(&self, on: bool);             // the preference itself stays in the app (device setting), passed at start
    pub fn state(&self) -> impl Stream<Item = PresenceState>;
}
pub const IDLE_AFTER: Duration = Duration::from_secs(5 * 60);
impl Presence { pub fn dot(&self) -> Option<Dot>; pub fn line(&self, now: SystemTime) -> Option<String>; }
```
- Reading others' presence (`watch_presence`) stays as it is.

**FFI**
- `MtrxError::PresenceUnavailable`.
- `Session::presence_activity(activity) async`.
- `Session::set_presence_sharing(on)`.
- `Presence` gains `dot: Option<PresenceDot>` and `line: Option<String>`. The line is computed when sent, so it can go stale; it can be refreshed with `presence_line(presence, now)`.

**Deleted**
- iOS: `PresenceModel` idle timer, `setOwn` queue, `beActive`, `unavailableReason`, `line`, `dot`.
- Android: the same in `Presence.kt`.
- Desktop: `presence::Activity`, `IDLE_CHECK`, `set_own`.

---

## 7. Local echoes in core streams

**Today**
Several hand-written "show it now, wait for the server" layers:
- iOS:
  - Reactions: `TimelineModel.swift:107`, `withOurReactions`.
  - Pins: `TimelineModel.swift:498`.
  - Topic: `RoomInfoModel.swift:20, 61-80`.
  - Notes at the top: `SessionModel.swift:72, 143-178`.
  - Joins: `SessionModel.expecting`, `DirectoryModels.swift:97-150`.
- Desktop:
  - Tags: `arrange.rs` `pending_tags`, `TAG_SYNC` 10 s.
  - Topic: `topic.rs` `TOPIC_SYNC` 10 s.
- Android has none, so it shows the old value until sync catches up.

**Core: `crates/mtrx-core/src/echo.rs`**
```rust
pub(crate) struct Echoes {                       // in Session, watch-backed
    tags: HashMap<OwnedRoomId, Pending<RoomTag>>,
    topics: HashMap<OwnedRoomId, Pending<Option<String>>>,
    pinned: HashMap<OwnedRoomId, Pending<Vec<OwnedEventId>>>,
    joining: BTreeSet<OwnedRoomId>,              // joined/made, not listed yet
}
struct Pending<T> { value: T, since: Instant }   // dropped when the synced value equals it, on failure, or after ECHO_TIMEOUT
pub const ECHO_TIMEOUT: Duration = Duration::from_secs(10);
```
- These mutations record an echo before their request, and drop it if the request fails:
  - `set_room_tag`, `set_room_topic`, `set_pinned`, `join`, `create_room`, `direct_chat`.
- The `RoomList` stream lays echoes over the summaries (`tag`, `topic`, `pinned`). `RoomLists.joining` lists rooms that are coming.
- `change_room_categories` (notes at the top) publishes its new value at once, before the server confirms.
- The rows' Pin/Unpin come from the echoed `pinned`.
- Reactions: matrix-sdk-ui already echoes them. Measure whether the iOS overlay is needed before deleting it (a sub-step, no decision).

**FFI**
- No new calls. `RoomLists` gains `joining() -> Vec<String>`.
- Mutations keep their signatures and now return after recording the echo and making the request.

**Deleted**
- iOS: the overlays listed above (`expecting`, `notesAtTopSaving`, `pinned` overlay, topic `saved`, `JoiningModel.joined`), and probably the reaction overlay.
- Desktop: `PendingTag`, `rooms_updated`, `TAG_SYNC`, `topic::Panel::saved`, `TOPIC_SYNC`.

Overlap with step 3: there, the live RoomSettings stream takes over the room-settings echoes (access, history and so on). Step 2 covers only what shows in lists, timeline and topic.

---

## PR split

One PR per concern. Each has the same four commits, so every commit builds:
1. Core and FFI: new types, with tests, alongside the old ones.
2. Desktop switches.
3. iOS switches.
4. Android switches, and the old FFI is removed.

| PR | Items | Size | Depends on |
|---|---|---|---|
| A | 2 timeline rows | L | — |
| B | 3 seek, 4 drafts | M | A (`item_of`) |
| C | 1 room lists | L | — |
| D | 7 echoes | M | A, C |
| E | 5 member levels | S | — |
| F | 6 presence | M | — |

Order:
- E and F are independent; they can go first or run in parallel.
- Then A, C, B, D.
- Release after C and after D, so the phone gets the changes in between.

Per-room calls (`seek`, `DraftKeeper`, rows) go on the existing per-room FFI `Timeline` object for now. They move into step 3's `Room` object later → **Q5**.

---

## Decisions (user, 2026-11-01)

- Q1: a space's unread dot ignores direct chats everywhere (#45).
- Q2: section layout and room sort stay per device in the apps, passed in as arguments.
- Q3: the desktop gets the iOS read-marker line, which breaks groups.
- Q4: the desktop goes idle after 5 minutes without input, even focused.
- Q5: rows and seek go on the FFI `Timeline` object (their home); `DraftKeeper` is its own object. Nothing moves in step 3.
- Q6: core returns the English text (state lines, level confirmations, presence line).
- Q7: drafts stay on the device in each room until sent; leaving a room does not clear them. Android stops clearing on leave.
