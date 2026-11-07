# Room links on the phones

Goal: the phones show and edit a room's calls and links as the desktop does, from one parsed topic that core hands every app. The topic format (`[call] <url>`, `[call Title passcode 123] <url>`, `[link] <url>`, `[link Title] <url>` lines after the text) stays as it is.

The phones don't show the raw topic any more: the catch-up (`831b4d6` iOS, `ffe31ce` Android) gave both a topic line, a topic panel with link buttons and a structured editor. What's left is smaller:
- a Join button for the room's call in the header;
- reordering links (the first call is the room's);
- one URL rule, and the passcode shown on Android;
- the space overview's links on iOS;
- the parse moving from the apps' render paths into core's room list.

Rules from steps 2–4 that apply:
- Core sends finished state through the FFI. Apps keep no parallel copies and don't parse.
- Core returns English text. Derived values are record fields, not functions.
- iOS behaviour is canonical, unless it's a bug.
- No new dependencies.

Paths: `core/` = `crates/mtrx-core/src/`, `ffi/` = `crates/mtrx-ffi/src/`, `app/` = `crates/mtrx-app/src/`, `ios/` = `apps/ios/Mtrx/`, `android/` = `apps/android/app/src/main/kotlin/org/mtrx/app/`.

---

## Today

**Core**
- `core/room_links.rs` has the parser, already in core since PR #46:
  - `Topic{text, links}` (`:25`), `RoomLink{kind, title, url}` (`:32`), `LinkKind::{Call{passcode}, Link}` (`:39`);
  - `Topic::parse` (`:46`, links in written order), `to_topic` (`:63`, calls first), `calls`, `default_call`;
  - `RoomLink::name(services)` (`:227`: title, else the call's service, else the host);
  - `describe_change` (`:84`), which the timeline's state rows use (`core/rows.rs:241`).
- But `RoomSummary.topic` is `Option<String>` (`core/rooms.rs:206`, set at `:367`), and so is `DirectoryRoom.topic` (`core/directory.rs:44`).
- Echoes keep the raw string: `Pendings<Option<String>>` (`core/echo.rs:270, 332, 547-561`).

**FFI**
- `RoomSummary.topic: Option<String>` (`ffi/types.rs:131`) and `DirectoryRoom.topic` (`ffi/rooms.rs:16`).
- Free functions `parse_topic(String) -> Topic` and `compose_topic(Topic) -> String` (`ffi/rooms.rs:147-211`).
  - The `Topic{text, links}` and `RoomLink{call, title, url, passcode, name}` records serve both as output and as editor input. As input, `name` means nothing (iOS passes `""`, Android a made-up value).
  - `parse_topic` names links with no user services (`name(&[])`).
  - `compose_topic` only checks that `Url::parse` succeeds. So `mailto:x` is written as `[link] mailto:x`, which `parse` then reads back as text. That's a bug.
- `Room.set_topic(String)` (`ffi/room.rs:77`).

**Desktop** (it uses core's parser, but parses at view time)
- `Topic::parse(summary.topic)` is called in:
  - the header, on every view (`app/room/view.rs:332`);
  - the space topic (`app/sidebar/mod.rs:392`, `sidebar/state.rs:192`);
  - the room card (`app/info.rs:252, 285, 1196-1211`);
  - the menu reorder (`app/calls.rs:432-434`).
- In `app/topic.rs`:
  - `preview` (`:179`), `moved` (`:242`);
  - `Editor::new(&str)` (`:290`) and `changed` (`:310`), which compare tidied strings;
  - `Draft::link` (`:570`), the desktop's own URL rule: `https://` may be left out, a host is required;
  - the raw-text toggle, `Edit::AsText` (`:347-355`).
- The header's split button (`app/calls.rs:103-205`):
  - the left half is Join (the first call; the hint says "Join <name>, passcode …"), or Start call with the space's or own call service;
  - the right half shows camera N · link N with a caret, and opens a menu: calls, then links, each dragged to reorder, then the services to start a call with.
- `app/directory.rs:150` shows the raw topic's first line. A topic that only has links shows "[call] https://…".
- Unrelated: `app/overview.rs:218` has a broken doc comment ("The space's topic and links    /// The tabs…").

**iOS**
- `ios/Views/RoomView.swift:142-144`: `topic` is a computed property that calls `parseTopic`, so every body evaluation crosses the FFI.
- `RoomHeader` (`RoomView.swift:226-262`):
  - the name opens Room info;
  - a caret and `Topics.preview` open the `TopicPanel` sheet;
  - the trailing toolbar slot has the `SpaceButton`.
- `ios/Views/TopicViews.swift`:
  - `TopicPanel`: the text, a `LinkButton` per link (the passcode beside it), the pins, and Edit.
  - `TopicEditor`: the text, then Calls and Links sections, "Add call", "Add link" and remove. You can't reorder links or change a link's kind, and there's no raw text.
- `ios/Models/Topics.swift`:
  - `preview` (`:9`);
  - `firstLine` (`:22`, which parses);
  - `TopicDraft` (`:32-92`), with its own URL rule (`valid`, `:43-47`: a scheme and a host are required) and its `dropped` count.
- `TopicSection` (`ios/Views/RoomInfoView.swift:171-196`): the text, the links as buttons, and Edit. It's in Room info and in the space overview's Settings tab (`SpaceOverviewView.swift:86`). The space overview's header shows the text only (`:125-132`).
- `DirectoryModels.swift:17, 120` and `InviteView.swift:45` parse for the first line.
- Tests: `MtrxTests/RoomInfoTests.swift:95-122`.

**Android**
- The topic is parsed in several places:
  - `roomHeader` (`android/timeline/RoomHeader.kt:38-46`, remembered at `ui/Navigation.kt:307`);
  - again for the info header (`Navigation.kt:425`).
- `RoomTitle` (`timeline/TimelineScreen.kt:601-665`):
  - the name, a caret, the pin count, and `topicPreview`;
  - it opens `TopicPanel` (`timeline/TopicPanel.kt`), dropped over the timeline: the text, `TopicLinks`, the pins, Edit, and the editor inline;
  - the top bar's actions slot has the `SpaceButton`.
- `rooms/TopicViews.kt`:
  - `TopicLinks` (`:53`): a `FlowRow` of tonal buttons. **No passcode.**
  - `TopicEditor` (`:76-125`): one list of links, each with a Link/Call segmented button and remove. No reorder, no raw text.
- `rooms/Topics.kt`: `topicPreview`, and `TopicDraft.toTopic`, which drops blank URLs and nothing else.
- `TopicBox` (`rooms/RoomInfo.kt:267-305`) is in Room info (`:150`) and above the space overview's tabs (`SpaceOverview.kt:159`).
- The raw topic's first line or full text shows in `ui/HomeLists.kt:384`, `rooms/Directory.kt:25, 41` and `rooms/InviteScreen.kt:69`.

## Where they disagree

| | Desktop | iOS | Android |
|---|---|---|---|
| Parse | core fn, at view | FFI `parseTopic` per render | FFI `parseTopic`, remembered |
| Header | preview; split [Join \| camera N · link N ▾] | preview → sheet | pins + preview → panel |
| Join the room's call | yes | no | no |
| Passcode | Join hint, panel | beside the link | not shown |
| Reorder | drag (menu, editor) | no | no |
| Kind of a link | fixed (Add call / Add link) | fixed | segmented toggle |
| Raw text | toggle in the editor | no | no |
| URL rule | `https://` optional, host | scheme and host | none (core: parses) |
| Space links | chips in the overview | Settings tab only | box above the tabs |
| List / directory line | raw first line | parsed text | raw first line |

---

## Core

`core/room_links.rs` gains these, mostly moved from the desktop:

```rust
impl Topic {
    pub fn is_empty(&self) -> bool;
    /// The links that aren't calls, in their order.
    pub fn others(&self) -> impl Iterator<Item = &RoomLink>;
    /// The text's first non-blank line: list rows and directory entries.
    pub fn first_line(&self) -> Option<&str>;
    /// The text's first line, else the links' names: under the room's name.
    /// Moved from app/topic.rs:179, without the "Add a topic" fallback (see Q1's UI).
    pub fn preview(&self, services: &[CallService]) -> Option<String>;
    /// With a call (`calls`) or other link moved from one place to another
    /// among its kind; the calls come first. Moved from app/topic.rs:242.
    pub fn moved(self, calls: bool, from: usize, to: usize) -> Topic;
    /// Text and links as typed: calls first in their order, blank links left
    /// out, and those without a web address too, counted in `dropped`.
    pub fn from_drafts(text: &str, links: &[LinkDraft]) -> (Topic, usize);
}

/// A link as typed in an editor. Replaces the desktop's `topic::Draft`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinkDraft { pub call: bool, pub title: String, pub url: String, pub passcode: String }
impl LinkDraft {
    pub fn link(&self) -> Option<RoomLink>;   // app/topic.rs:570, the one URL rule
    pub fn is_blank(&self) -> bool;
    pub fn line(&self) -> String;             // as typed when not a link yet (the desktop's raw toggle)
}
impl From<&RoomLink> for LinkDraft;

/// A typed address as the topic keeps it: "https://" may be left out; http(s)
/// with a host only. None if it isn't a web address.
pub fn typed_link_url(text: &str) -> Option<Url>;
```

- **One URL rule.** `LinkDraft::link` and `typed_link_url` share it, with `https://` added when there's no scheme. `RoomLink::parse` stays strict (an explicit http(s) URL), but it now requires a host too. This fixes the `mailto:` round trip.
- **The summary carries the parsed topic.** `RoomSummary.topic: Topic` replaces `Option<String>`, parsed once where the summary is built (`core/rooms.rs:367`). An empty `Topic` means none.
- `DirectoryRoom.topic: Topic` likewise (`core/directory.rs:197, 318, 386, 415`).
- **Echoes.** `Pendings<Topic>`: `Echoes::set_topic(room, Topic)` is parsed once in `RoomHandle::set_topic` (`core/rooms.rs:659`), and `shows_over` compares `Topic`s.
- `RoomHandle::set_topic(&str)` doesn't change.
- **Cost.** One parse per summary rebuild: a handful of lines per room, microseconds. Today it's one per render on iOS and per view on the desktop.

## FFI (`ffi/rooms.rs`, `ffi/types.rs`)

```rust
/// A call or link in a topic. Output only.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RoomLink {
    pub title: Option<String>,
    pub url: String,
    pub passcode: Option<String>,   // calls only
    /// Its title, else the call's service, else its host.
    pub name: String,
}

/// A room's topic, parsed by core.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct Topic {
    pub text: String,
    /// In order; the first is the room's call, which the header joins.
    pub calls: Vec<RoomLink>,
    /// The other links, in order.
    pub links: Vec<RoomLink>,
    /// The text's first non-blank line: list rows and directory entries.
    pub first_line: Option<String>,
    /// The first line, else the links' names: under the room's name.
    pub preview: Option<String>,
}

/// A call or link as typed in the topic editor.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct LinkDraft { pub call: bool, pub title: String, pub url: String, pub passcode: String }

pub struct RoomSummary   { /* … */ pub topic: Topic, /* was Option<String> */ }
pub struct DirectoryRoom { /* … */ pub topic: Topic, /* was Option<String> */ }

/// The address a typed URL is saved as ("example.org/x" → "https://example.org/x"),
/// or none if it isn't a web address: the editor marks the field.
#[uniffi::export] pub fn link_url(text: String) -> Option<String>;
/// The topic to save from typed text and links: calls first, each kind in its
/// order; blank links and those without a web address left out.
#[uniffi::export] pub fn compose_topic(text: String, links: Vec<LinkDraft>) -> String;
```

- Splitting `calls` and `links` replaces `RoomLink.call`. Every view groups them anyway: the desktop menu, both phones' editors, the panel order.
- `From<&core::Topic>` fills `first_line` and `preview` with `name(&[])`: the well-known services. The phones have no call services of their own (Q6).
- `NewRoom.topic` stays a `String`: it's free text typed in.
- `Room.set_topic(String)` stays, fed by `compose_topic`.
- Removed: `parse_topic`, and the old `compose_topic(Topic)` (see Q4).
- `TryFrom<&RoomSummary> for core::RoomSummary` (the apps' tests) builds the core `Topic` back from the record.

## Desktop (switches to the parsed summary)

- The views read `summary.topic` (a `&Topic`) and no longer call `Topic::parse`: `room/view.rs:332`, `sidebar/mod.rs:392`, `sidebar/state.rs:192`, `info.rs:1196-1211`, `calls.rs:432`.
- `calls.rs:433-434` becomes `summary.topic.clone().moved(calls, from, to).to_topic()`.
- In `topic.rs`:
  - `Editor::new(&Topic)`;
  - `changed(&Topic)` compares against `topic.to_topic()`;
  - `Draft` becomes `core::LinkDraft`;
  - `preview` and `moved` go, since core has them. The header adds "Add a topic" itself when `can_edit`.
- The raw toggle still parses what's typed. `Editor::new` from text becomes `Editor::from_text(&str)`, which uses `Topic::parse`.
- `directory.rs:150` uses `room.topic.first_line()`.
- Fix the doc comment at `overview.rs:218` in passing.
- Nothing changes visibly, except that the directory no longer shows "[call] …" lines.

## iOS (canonical)

**Header** (Q1)
- The trailing toolbar gets one item before the `SpaceButton`, only when the topic has a call or a link.
- With a call:
  - `Menu { … } label: { Label("Join", systemImage: "video.fill") } primaryAction: { join(topic.calls[0]) }`, a small prominent capsule;
  - a tap joins the room's call, a long-press shows the menu;
  - the menu has a "Calls" section, then a "Links" section, each entry `Label(name, systemImage:)` with its passcode as the subtitle; a tap opens it;
  - for those who may edit, the last entry is "Edit links…", which opens the editor.
- With links only: the same menu behind a `link` icon, with no primary action.
- Accessibility label: "Join <name>", plus ", passcode …" when there is one.
- `RoomHeader` keeps the name and the caret line. The line is `topic.preview ?? (canEdit ? "Add a topic" : nil)`.
- `RoomView.topic` goes. Views read `room.topic`.

**Joining** (Q2): `openURL(call.url)`. If the call has a passcode, it's copied to the clipboard first, and a short banner says "Passcode 123 copied".

**Panel** (`TopicPanel`): it shows calls, then links. Otherwise it's unchanged. `LinkButton` takes the new `RoomLink`, and its `call` flag comes from the section.

**Room info** (`TopicSection`): unchanged, with calls before links.

**Space overview header** (Q5): link chips (capsule `LinkButton`s in a wrapping layout) under the text, like the desktop's chips. Edit stays in the Settings tab's `TopicSection`.

**Editor** (Q3, Q4)
- It stays structured: text, Calls, Links.
- Each section gets `.onMove`, with drag handles always shown (`editMode` active within the editor). The first call is the room's: the Calls footer says "The first call is the room's: Join opens it".
- `TopicDraft.Link.valid` uses `linkUrl`, and `dropped` counts the same way.
- `TopicDraft.topic` calls `composeTopic(text:links:)`.
- `TopicDraft.init(_ topic: Topic)` maps `calls` and `links` to drafts.

**Lists**: `Topics.firstLine` and `Topics.preview` go. `DirectoryModels` and `InviteView` read `topic.firstLine`, and search uses `topic.text`.

## Android (follows iOS)

**Header**
- In the top bar's `actions`, before the `SpaceButton`:
  - with a call, a tonal "Join" button (camera icon), where `combinedClickable` taps to join and long-presses to open a `DropdownMenu` with the same sections and entries as iOS;
  - with links only, a link `IconButton` that opens the menu.
- Material3 1.3 (BOM 2025.05.01) has no split button. That's fine: iOS doesn't show one either.
- `RoomTitle` keeps the pins and the preview line, which reads `topic.preview` or "Add a topic".

**Joining:** `uriHandler.openUri`, copying the passcode first through `ClipboardManager`. A snackbar says "Passcode 123 copied" (Android 13+ also shows its own clipboard chip).

**Panel and room info:**
- `TopicLinks` shows calls, then links. A call's passcode shows as a second line in its button, which fixes the gap.
- `TopicBox` is otherwise unchanged.

**Space overview** (Q5):
- the header shows the text and link chips, as on iOS;
- `TopicBox` moves into the Settings tab, as iOS's `TopicSection`.

**Editor:**
- Calls and Links sections with "Add call" and "Add link", as on iOS.
- The segmented Link/Call toggle goes: iOS and the desktop fix a link's kind.
- Reorder (Q3): a drag handle on each link, written by hand with `detectDragGesturesAfterLongPress` on the editor's column, with no library. About 80 lines, like the desktop's `reorder.rs`.
- The URL field turns red when `linkUrl(url)` is null.
- `TopicDraft.toTopic` becomes `compose`, through `composeTopic(text, links)`.

**Parsing gone:**
- `roomHeader` loses its `parseTopic` parameter;
- `infoHeader` (`Navigation.kt:425`) reads `room.topic`;
- `NO_TOPIC` stays, as the empty record;
- `HomeLists.kt:384`, `Directory.kt:25, 41` and `InviteScreen.kt:69` read `topic.firstLine` and `topic.text`.

## Tests

- Core (`room_links.rs`):
  - `typed_link_url`: a missing scheme is added; `mailto:`, `ftp:` and no host are rejected;
  - `parse` rejects a URL without a host;
  - `from_drafts`: calls first, blank links left out, invalid ones counted;
  - `first_line`, `preview` and `moved`;
  - `echo.rs`'s topic test with `Topic`.
- FFI (`rooms.rs`):
  - `From<&core::Topic>` splits calls and links and fills the names, `first_line` and `preview`;
  - `compose_topic` with an invalid link and a reordered call;
  - the integration test (`tests/integration.rs:896-936`) composes from `LinkDraft`s and reads `summary.topic.calls[0]`.
- Desktop: the `topic.rs` tests move to `Editor::new(&Topic)`, and the `preview` and `moved` tests move to core.
- iOS:
  - `TopicsTests` become `TopicDraftTests` (round trip, invalid URL marked, calls first after a move);
  - a header test: Join only with a call, the menu's sections in order;
  - a header screenshot with Join.
- Android:
  - `TopicDraft` compose and reorder;
  - `roomHeader` without a parser;
  - screenshot tests: the header with Join, a passcode in `TopicLinks`, and the space overview's chips.

## PR split

One PR, three commits (CI only):
1. **Core, FFI and the desktop switch:** `room_links.rs` additions, `RoomSummary`/`DirectoryRoom.topic: Topic`, echoes, the FFI records and functions, and the desktop reading the parsed topic. These go together because the summary's type changes under the desktop. The phones build again from commits 2–3, and CI builds the PR's tip.
2. **iOS:** the header's Join menu, the passcode copy, reorder in the editor, the space overview chips, the parse calls removed, tests.
3. **Android:** the same, plus the passcode in `TopicLinks`, the segmented toggle removed, and `TopicBox` moved into the Settings tab.

Size: M. About +150 / −80 lines in core and the desktop, 80 in the FFI, 150 for iOS and 250 for Android.

---

## Open questions

1. **The phone header.**
   - (a) One toolbar item: "Join" when there's a call (a tap joins the first call, a long-press shows all calls and links and Edit links…), else a link icon that opens the same menu. The caret line stays for the text and pins.
   - (b) Join only when there's a call. Links stay in the panel.
   - (c) Nothing new in the header. Show "camera 2 · link 3" in the caret line instead.
   - Recommend (a). It's the desktop's split button folded into the platform's menu-with-primary-action, and it costs one toolbar slot.

2. **A call's passcode when joining from the phone.**
   - (a) Copy it to the clipboard, say "Passcode 123 copied", then open the call.
   - (b) An alert with the passcode and a Join button.
   - (c) Nothing: it's in the panel and the menu.
   - Recommend (a). The call app opens on top, and pasting is the easiest way in.

3. **Reordering in the phone editor.**
   - (a) Drag within Calls and within Links: `.onMove` on iOS, a hand-written drag handle on Android.
   - (b) Only "Make the room's call" on a call, which moves it first.
   - (c) No reordering.
   - Recommend (a). Order is what the topic stores and the desktop shows, and with (b) the links can't be reordered.

4. **The raw topic text on the phones.**
   - (a) No: the editor is structured only, and `parse_topic` leaves the FFI.
   - (b) An "Edit as text" toggle, as on the desktop. The FFI keeps `parse_topic` for toggling back.
   - Recommend (a). Typing link lines on a phone keyboard is error-prone, and the structured editor covers every line format.

5. **Where a space's links show on the phones.**
   - (a) As chips under the text in the overview's header, with the editor in the Settings tab, on both phones (Android's `TopicBox` moves there).
   - (b) Leave them as they are: iOS in Settings, Android in a box above the tabs.
   - Recommend (a). It's the desktop's chips, and one layout on both phones.

6. **Start call on the phones** (the desktop's left half when there's no call).
   - (a) Not now. Call services are desktop settings, and the phones have none.
   - (b) Start a call with the space's call service only (`Rights.setCallService` already reaches the FFI).
   - Recommend (a). It's a separate feature, and Google Meet needs the Google sign-in, which only the desktop has.

7. **`RoomSummary.topic` in the FFI.**
   - (a) `Topic` replaces the string in `RoomSummary` and `DirectoryRoom`, and the apps never parse.
   - (b) Keep the raw string beside the parsed `Topic`.
   - (c) Keep the string and the `parse_topic` function (today).
   - Recommend (a). One model, parsed once. Nothing reads the raw string if (4a) holds.

## Decisions (2026-11-07)

The user took all recommendations: 1a, 2a, 3a, 4a, 5a, 6a, 7a.
