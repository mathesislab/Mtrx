# Step 4: app structure

Goal: each app is built from parts that own their state, their work and their messages. Steps 2 and 3 moved behaviour into core. This step splits what is left in the apps: the desktop's god object, its four menu renderers, two message fields and two selection widgets; the phones' session holders; the tasks nobody owns; and the FFI nobody calls.

Rules from steps 2 and 3 that still apply:
- iOS behaviour is canonical, unless it's clearly a bug.
- Core returns English text.
- FFI labels are record fields, not functions (Android JVM tests can't call FFI functions).
- No new dependencies without asking (flagged in the open questions).

Paths: `core/` = `crates/mtrx-core/src/`, `ffi/` = `crates/mtrx-ffi/src/`, `app/` = `crates/mtrx-app/src/`, `cli/` = `crates/mtrx-cli/src/`, `ios/` = `apps/ios/Mtrx/`, `android/` = `apps/android/app/src/main/kotlin/org/mtrx/app/`, `android-test/` = `apps/android/app/src/test/kotlin/org/mtrx/app/`.

Checked against `main` at 0.6.0 (#69–#80). What step 3 changed in the agreed items:
- All items still apply. None is dropped.
- `app.rs` lost the seven rights loads and the settings reloads (#77, #78), but is still 5373 lines.
- `Shared(...)` stream wrappers became `live.updates()` (#70). `Shared` stays only as a subscription key.
- iOS `SessionModel` lost `SpaceChildrenCache` and `joined` (#73, #76). It is 453 lines, still four concerns.
- The FFI room-picture calls moved onto `Room` as `set_picture`/`remove_picture` (#71). Still unused by the phones.
- Re-checking the FFI found four more unused calls, added in #73 (item 13, Q10).

Downloads (`docs/downloads.md`, in parallel) are not designed here. Items 1, 2 and 6 say where they fit.

---

## 1. Desktop: app.rs into components

**Today**
- `app.rs` is 5373 lines.
  - `Message` has 91 variants (`app.rs:98-253`).
  - `Main` has 34 fields (`app.rs:294-356`).
  - `App::handle` is one 1240-line match (`app.rs:979-2215`).
  - `App::room` is another 640 lines for `room::Message` (`app.rs:2219-2854`).
  - Room features have their handlers in `app.rs`: `calls` (`:3200`), `room_topic` (`:3276`), `act` (`:3387-3647`), `find` (`:3650`), `later` (`:3720-3835`).
- Every handler re-finds its state:
  - `Screen::Main` is unwrapped 120 times.
  - `main.pane.open_mut(&id)` is called 33 times.
- `App::update` (`app.rs:865-944`) runs cross-cutting rules after every message:
  - closing the popup on six message kinds;
  - closing a member menu when the card changes, and a field's popups when the edit changes;
  - resetting the space topic when the space changes;
  - keeping the draft;
  - the badge, rooms out of view, followed presence, and the "Notes at the top" migration.
- Some components already have their own `update`, returning an `Action` that `app.rs` runs:
  - `login::Action` (`app.rs:1073`), `dialog::Action` (`:2891`), `join::Action` (`:3994`), `settings::Action` (`:3072`).
  - `info::Card::update` returns `overview::Action` (`:1506`).
  - `arrange` takes a `Context` with `&mut popup` (`:1154`).
- Step 1 (#61) added `Pane` (`:361`), `Modal` (`:412`) and `Popup` (`:426`).

**Components**

| Component | State (from `Main` unless said) | Messages | Moves from |
|---|---|---|---|
| `window` (new `app/window.rs`) | `App.placement`, `window`, `scale`, `system_mode`, `resizing` | `Resized`, `Opened`, `Moved`, `Placed`, `ScaleFactor`, `Theme`, `Resize`, `Zoom`, `Close` | `app.rs:989-1007, 1902-1927, 2043-2115, 3837-3852` |
| `start` (new `app/start.rs`) | `App.screen` changes | `Restored`, `Login`, `LoggedIn`, `Ready`, `SignInAgain`, `Forgotten` | `:1008-1121, 1564-1585` |
| `Sidebar` (new `sidebar/state.rs`; `sidebar.rs` → `sidebar/mod.rs`) | `space`, `filter`, `arrange`, `out_of_view`, `space_topic`, `space_rooms`, `colours` | `SelectSpace`, `Filter`, `OpenFirstMatch`, `GoTo`, `FocusSearch`, `SpaceRoomsLoaded`, `JoinFound`, `FoundJoined`, `Scrolled`, `OutOfView`, `ScrollTo`, `AddMenu`, `SpaceMenu`, `SpaceTopic`, `Arrange`, `ArrangeFailed`, `Colours`, `ColoursMoved` | `:1037-1057, 1140-1169, 1643-1714, 3301-3319, 5085-5114` |
| `Pane` (new `app/pane.rs`) | `pane`, `show_members` | `Open`, `Opened`, `Answer`, `Answered`, `ToggleMembers`, `ShowMembers`, `MemberMenu`, `SetLevel`, `MembersChanged`, `Reload` | `:1170-1250, 1528-1550, 1594-1642, 1774-1832, 4707` |
| `Room` (`room.rs` → `room/mod.rs`, `room/view.rs`, `room/update.rs`) | `Room` | `room::Message`, unchanged | `:2219-2854, 5269-5370`; `actions::update` from `:3387-3647`; `find::update` from `:3650`; `send_later::update` from `:3720`; `calls::update` from `:3200`; `topic::update` from `:3276, 5046-5084` |
| `info::Card` | `card` | `info::Message` + `Show(Target, Tab)` | `:1353, 1397-1513, 4106-4149, 5115-5176` |
| `Modal` (new `app/modal.rs`) | `modal`, `background`, `last_request` | `ShowSettings`, `Settings`, `OpenDialog`, `Dialog`, `OpenJoin`, `Join`, `OpenImage`, `CloseImage`, `ImageLoaded`, `OpenPdf`, `Pdf`, `Progress` | `:1251-1269, 1302-1337, 1354-1396, 2128-2167, 2206-2213, 2856-3185, 3994-4103, 4743` |
| `Account` (new `app/account.rs`) | `verified`, `recovery_*`, `connection`, `scheduled`, `now` | `RecoveryKey`, `Recover`, `RecoveryProgress`, `Recovered`, `Connection`, `Reconnect`, `Scheduled`, `Tick`, `Notified`, `NotificationClicked` | `:1551-1563, 1586-1592, 1858-1901, 1943-2019, 5193-5252` |
| `People` (new `app/people.rs`) | `presences`, `contacts`, `frequent_emoji` | `Presence`, `ContactsLoaded`, `FrequentEmoji`, `ChatWith`, `ChatStarted`, `StartNotes`, `FavouritePerson` | `:1338-1352, 1514-1527, 1833-1857, 3187-3198, 5282-5311` |
| `MediaState` (`media.rs`) | `avatars`, `previews`, `App.math` (and downloads later) | `AvatarLoaded`, `PreviewLoaded`, `MathRendered`, `Save`, `CopyImage`, `Done` (+ item 5's `*Wanted`) | `:2020-2023, 2116-2127, 2168-2205` |

`Main` itself keeps `session`, `own_user`, the room lists (`room_lists`, `lists`, `restore`, `joined_space`, `notes_moved`), `popup` and `Notices` (`notice`).

Window-wide messages stay on `App`: `Escape`, `Submit`, `LinkClicked`, `ClosePopup` (item 2), `CloseNotice`, `Update`, `CheckForUpdates`. `Escape` asks in today's order (`app.rs:1715-1760`): `Modal::escape`, the popup, the card, `Room::escape` (completion, find bar, selection, reply, edit), `Sidebar::escape` (drag or category name, else the search).

**Dispatch**
```rust
// app.rs: one variant per component (Q2)
pub enum Message {
    Window(window::Message), Start(start::Message),
    Sidebar(sidebar::Message), Pane(pane::Message),
    Room(OwnedRoomId, room::Message), Shown(room::Message),   // as now
    Card(info::Message), Modal(modal::Message),
    Account(account::Message), People(people::Message), Media(media::Message),
    Update(update::Message), #[cfg(target_os = "macos")] CheckForUpdates,
    Escape, Submit, LinkClicked(String), ClosePopup, CloseNotice,
}

/// What a component's update may read and change besides its own state.
pub struct Cx<'a> {
    pub session: &'a Arc<Session>,
    pub own_user: &'a OwnedUserId,
    pub lists: &'a Arc<RoomLists>,
    pub connection: Connection,
    pub settings: &'a mut Settings,     // saved by App after the update if changed
    pub google: &'a Arc<Google>,
    pub window: &'a window::Window,     // viewport, pixel scale, math params
    pub popup: &'a mut Option<Popup>,
    pub notices: &'a mut Notices,       // `report`, `done`
}
impl Cx<'_> {
    /// Something opens over the window: the popup closes.
    pub fn cover(&mut self);
}

impl Sidebar   { pub fn update(&mut self, m: sidebar::Message, cx: &mut Cx) -> Task<Message>; }
impl Pane      { pub fn update(&mut self, m: pane::Message, cx: &mut Cx) -> Task<Message>;
                 /// Show `pane`: the old room's popups close and its draft is kept now.
                 pub fn set(&mut self, pane: Shown, cx: &mut Cx) -> Task<Message>; }
impl Room      { pub fn update(&mut self, m: room::Message, cx: &mut Cx) -> Task<Message>; }
impl info::Card{ pub fn update(&mut self, m: info::Message, cx: &mut Cx) -> Task<Message>; }
impl Modal     { pub fn update(&mut self, m: modal::Message, cx: &mut Cx, updater: &mut Updater) -> Task<Message>; }
impl Account   { pub fn update(&mut self, m: account::Message, cx: &mut Cx) -> Task<Message>; }
impl People    { pub fn update(&mut self, m: people::Message, cx: &mut Cx) -> Task<Message>; }
impl MediaState{ pub fn update(&mut self, m: media::Message, cx: &mut Cx) -> Task<Message>; }

impl Main {
    /// Destructures itself so a component and `Cx` borrow disjoint fields.
    fn update(&mut self, message: Message, app: AppParts<'_>) -> Task<Message>;
    /// After every message: badge, rooms out of view, followed presence, notes migration.
    fn after(&mut self, cx: &mut Cx) -> Task<Message>;
}
```
- Components reach each other by sending app messages (`Task::done(Message::Card(..))`), as `LinkClicked` and `ShowSpace` already do (Q1).
- Views still produce `app::Message`, through the `room::Message::to(id)` style helpers. No `.map` churn.
- Existing `Action` enums stay where they are (login, dialog, join, settings). Their runners move into `start`, `Modal` and `Sidebar`.
- A component that closes itself sends its close message. `Main` drops it.

The cross-cutting rules in `App::update` become explicit:
- The popup closes in the handlers that cover it (`OpenDialog`, `OpenJoin`, `Card::Show`, `SelectSpace`, `Pane::Open`), through `Cx::cover()`.
- `Main::set_card(Option<Card>)` closes a `Popup::MemberMenu`.
- `Room::set_editing(Option<InlineEdit>)` closes the edit field's popups.
- `Sidebar::select(space)` resets the space topic and remembers the view.
- The draft: `Room::update` tells the keeper after a field edit; `Pane::set` keeps the old room's draft now.
- `Main::after` keeps the badge, out-of-view, presence and notes rules, with today's message list for out-of-view.

**Room for downloads.** `MediaState` holds the downloads list. `Message::Media` carries its messages. Item 2's "Save…" items send `Message::Media(media::Message::Save(..))`.

**Deleted**
- `App::handle`'s single match, `App::room`, `room_left`, `act`, `find`, `later`, `calls`, `room_topic`, `space_topic` (moved, not rewritten).
- The 120 `Screen::Main` unwraps (one in `App::update`) and the 33 `open_mut(&id)` lookups (one in `Main::update`).
- The six-message popup list, `card_shown`, `edit_shown` and `selected_space` comparisons in `App::update`.
- `app.rs` keeps `App`, `Screen`, `Message`, routing, `subscription` and `view`: about 900 lines.

---

## 2. Desktop: one menu model

**Today**
Four renderers, five looks:
- `context_menu::menu` (`context_menu.rs:26`): width 170, `style::toolbar`, 13 pt, ellipsis. Used by message, link, media, edit, space, member "more", picture and add-to-space menus.
- `sidebar::menu` (`sidebar.rs:952`): full width, `style::code_block`, no ellipsis. Room and category menus.
- `add_menu_view` (`sidebar.rs:1137`): own buttons, 14 pt, `[5, 10]`, a separator.
- `members::role_menu` (`members.rs:280`): own buttons with a tick and the level.
- `actions::unsent_menu` (`actions.rs:806`): a row of text buttons.
- `calls::menu_view` (`calls.rs:203`): headings, separators, hints and reorderable lists.

Open state in three places:
- `Popup` (`app.rs:426-464`), with 17 close messages: `CloseMenu`, `ClosePicker`, `CloseReaders`, `CloseReactors`, `CloseUnsent`, `CancelDelete`, `AddMenu(false)`, `SpaceMenu(None)`, `MemberMenu(None)`, `calls::Close`, `send_later::Close`, `EditMenu(_, Close)`, `ComposerEmoji(Close)`, `EditEmoji(Close)`, `RoomMenu(None)`, `CategoryMenu(None)`, `ShowMenu(false)`.
- Card-local bools: `info.rs:183, 187` (`picture_menu`, `spaces_menu`).
- The viewer's `menu: bool` (`media.rs:451`), special-cased in `Escape` (`app.rs:1729`).

The native menu bar (`menu_bar.rs`) has its own static `Entry` (`Item`, `Separator`, `Submenu`) with selectors and shortcuts. It shares nothing with the in-window menus. Its Edit items reach fields as standard actions (`app.rs:4911`).

**Where they disagree**
- Five looks for the same thing (above).
- Esc with the card's picture or spaces menu open closes the whole card, not the menu: `Escape` sees no `Popup` (`app.rs:1731-1732`). This is a bug.
- A click outside closes `Popup` menus through their `on_dismiss`. The card's menus close only through their own toggles.

**Desktop: new `app/menu.rs`**
```rust
pub enum Entry<'a, M> {
    Item(Item<M>),
    Heading(String),
    Separator,
    /// Rows of its own: the calls menu's reorderable lists.
    Custom(Element<'a, M>),
}
pub struct Item<M> {
    pub label: String,
    pub on_choose: Option<M>,      // None: shown disabled
    pub checked: Option<bool>,     // the role menu's tick
    pub detail: Option<String>,    // the role menu's level
    pub hint: Option<String>,      // the calls menu's service hints
}
impl<M> Item<M> {
    pub fn new(label: impl Into<String>, on_choose: M) -> Self;
    pub fn disabled(label: impl Into<String>) -> Self;
    pub fn checked(self, on: bool) -> Self;
    pub fn detail(self, text: impl Into<String>) -> Self;
    pub fn hint(self, text: impl Into<String>) -> Self;
}
pub enum Width { Menu /* 170 */, At(f32), Fill }
/// One look: `style::toolbar`, 13 pt, [4, 10], ellipsis.
pub fn view<'a, M: Clone + 'a>(entries: Vec<Entry<'a, M>>, width: Width) -> Element<'a, M>;
```
- Placement stays: `context_menu(content, menu, on_open)` at the cursor, `popover(anchor, menu, side, edge)` below a button. Both `.on_dismiss(Message::ClosePopup)`.
- Pure builders, tested, replace the item code:
  - `message_menu::{entries, media_entries, link_entries}` (were `items`, `media_items`, `link_items`).
  - `edit_menu::entries`.
  - `sidebar::{room_entries, category_entries, space_entries, add_entries}`.
  - `members::{role_entries, more_entries}`.
  - `actions::unsent_entries` (Retry, "Cancel and put back"), now a menu, not a row.
  - `info::{picture_entries, add_to_space_entries}`.
  - `calls::entries` (headings, `Custom` reorder lists).
- `Popup` gains `CardPicture`, `CardAddToSpace` and `Viewer`. The three bools go (Q4).
- `Message::ClosePopup` replaces the 17 close messages. Each menu keeps its own open message, since opening builds state.
- Downloads: a downloads list fits as another `Popup` variant using `menu::view`, or a panel (`docs/downloads.md` decides).
- The menu bar keeps its own table (Q3).

**As built (T)**
- `Entry::map` and `Item::map` turn a builder's messages into the caller's, as the field's menu does (`edit_menu::entries` sends `edit_menu::Message`).
- The space bar's "+" has its own builder, `sidebar::new_space_entries`. The Show menu stays a panel of its own in `Popup::ShowMenu`.
- A message's entries are made only while its menu is open. `message_menu::offers` tells whether there is one, without making them.
- The new popups open with `info::Message::{OpenPictureMenu, OpenSpacesMenu}` and `Message::OpenViewerMenu`. `info::Shown` carries the window's `popup`.
- Esc closes the popup first, then the modal, the card and so on. Opening a modal (settings, image, PDF) now closes the popup, as a dialog already did, so an open popup is always on top. This is how Esc reaches the viewer's menu before the viewer.
- The downloads list is a popup too: `Popup::Downloads { menu: Option<DownloadId> }`, opened by `downloads::Message::OpenList`, with `downloads::Message::Menu` for a download's menu in it. `downloads.rs` keeps only the list and its errors. Esc closes the innermost first: the download's menu, then the list (`Popup::escape`). Another popup replaces the list, and it closes once the list is empty.

**Deleted**
- `context_menu::menu`, `sidebar::menu`, `add_menu_view`'s column, `role_item`, `unsent_menu`'s row, `calls::menu_view`'s frame.
- The 17 close messages and their handlers.
- `info::Message::{PictureMenu, SpacesMenu}`, `Message::ViewerMenu`, `Viewer.menu`, the viewer case in `Escape`.

---

## 3. Desktop: one MessageField

**Today**
The composer and the inline edit are two copies of one field:
- Composer view: `room.rs:968-1080`. Inline edit view: `actions.rs:270-344`.
- Both: `text_editor` with an id, the Markdown highlighter (`Marks`), `style::editor`, `clipboard::paste` keys, Enter (without Shift), `selection::editor_key`, `edit_menu::with_menu`, `emoji::in_field`, an emoji popover, `max_height`.
- Messages, in two enums:
  - `room::Message::{Composer, EditMenu(Field, _), Paste(Field, _), Pasted(Field, _), ComposerEmoji, InsertEmoji}` (`room.rs:107, 157-169`).
  - `actions::Message::{EditAction, EditEmoji, EditInsertEmoji}` (`actions.rs:71, 78-80`).
- The edit menu's handling serves both through `Field` (`app.rs:2728-2809`), with `in_field` (`app.rs:4968`) to send an action to either.

**Where they disagree**

| | Composer | Inline edit |
|---|---|---|
| Placeholder | yes | no |
| Mention completion | yes (`room.rs:996-1006`) | no → **Q5** |
| Esc | cancels a reply | cancels the edit (window's Esc) |
| ↑ in an empty field | edits our last message | — |
| ⌘↑/⌘↓ room keys | passed to the window (`room.rs:986`) | taken by the editor |
| Attachments | chips and a paperclip | — |
| Emoji panel | above | below, compact |
| Save/Cancel row | — | yes |

The room-key difference is a bug: in an edit, ⌘↑ moves the cursor instead of going to another room.

**Desktop: new `message_field.rs`**
```rust
pub struct MessageField { field: Field, content: text_editor::Content }

#[derive(Debug, Clone)]
pub enum Message {
    Action(text_editor::Action),
    Menu(edit_menu::Message),
    Paste(clipboard::Paste),
    Pasted(Option<clipboard::Pasted>),
    Emoji(emoji::Message),
    InsertEmoji(String),
}

/// What the field asks of its owner.
pub enum Event { Edited, Submit, Cancel, EditLast, Mention(mention::Message), Attach(Vec<PathBuf>), AttachImage(Picture) }

pub struct Look<'a> {
    pub placeholder: Option<&'a str>,
    pub marks: Arc<[String]>,          // mentions highlighted
    pub completing: bool,              // ↑ ↓ Enter Tab go to the completion
    pub cancellable: bool,             // Esc sends Cancel
    pub edit_last: bool,               // ↑ in an empty field sends EditLast
    pub emoji_side: Side, pub compact: bool,
    pub top_inset: f32, pub max_height: f32,
    pub menu: Option<&'a edit_menu::Menu>, pub emoji: Option<&'a emoji::Panel>,
    pub frequent: &'a [String],
}

impl MessageField {
    pub fn new(field: Field, text: &str) -> Self;
    pub fn text(&self) -> String;  pub fn set_text(&mut self, text: &str);  pub fn is_empty(&self) -> bool;
    pub fn insert(&mut self, text: &str);
    pub fn focus<T>(&self) -> Task<T>;
    pub fn update(&mut self, m: Message, popup: &mut Option<Popup>) -> (Task<Message>, Option<Event>);
    pub fn view<'a, M: Clone + 'a>(&'a self, look: Look<'a>, to: impl Fn(Message) -> M + Clone + 'a) -> Element<'a, M>;
}
```
- `room::Message::Field(Field, message_field::Message)` replaces the six composer and field messages.
- `Room.composer: MessageField`. `InlineEdit { target, field: MessageField, original }`.
- The composer wraps the field with attachments and the Send button. The edit wraps it with Save/Cancel.
- Both pass the window's room keys through.

**As built (S)**
- The field owns its mention completion. Its keys, a click on a candidate and Esc's close come as `Message::Mention` and go up as `Event::Mention`. `Room::mention` gives `MessageField::mention` the candidates, from `mention::Mentionable` (members, mention texts, us, `@room`).
- `Look` keeps the key flags, `side`, `compact`, `max_height` and `placeholder`. It takes `padding` (for `top_inset`) and `tools` (the composer's paperclip, beside the smiley). The room's part is `message_field::Context`: `mentions`, `avatars`, `frequent` and the window's `popup`, from which the field reads its own menu and panel.
- `view` returns `Element<'a, app::Message>`, not a generic `M`: the completion shows avatars, which send app messages (`wanting`).
- `Event::EmojiShown`: opening the panel reloads the most used emoji, which needs the session.
- Enter, Esc and ↑ are `Message::{Submit, Cancel, EditLast}`, sent up as the same events. The edit leaves Esc to the window, as before.
- `Field` moved from `clipboard` into `message_field`. `clipboard::read` takes whether files and pictures are wanted.

**Deleted**
- The composer's `text_editor` block (`room.rs:974-1080`) and `InlineEdit::view`'s editor (`actions.rs:279-321`).
- `room::Message::{Composer, EditMenu, Paste, Pasted, ComposerEmoji, InsertEmoji}`, `actions::Message::{EditAction, EditEmoji, EditInsertEmoji}`.
- `app.rs:2728-2809`, `in_field`, `field_emoji` (`app.rs:5009`), `room::in_field` (`room.rs:1803`).

---

## 4. Desktop: shared selectable-text geometry

**Today**
Two widgets (`docs/text-selection.md`) each write the same pointer handling and their own geometry:
- `selectable.rs` (1140 lines): rich text over cosmic-text.
  - Geometry on `State`: `offset_at` (`:288`), `press` (`:302`), `word_at` (`:323`), `line_step` (`:333`), `line_edge` (`:358`), `highlight` (`:375`).
  - Pointer handling in `update` (`:581-700`): click counting, link press with `CLICK_SLOP`, `on_click`, drag reporting, hovered link.
  - Its own selection for info cards (`update_own`, `:770`).
- `selectable_image.rs` (752 lines): a typeset message over its image.
  - Geometry on `Text`: `hit` (`:81`), `link_at` (`:111`), `word_at` (`:119`), `highlight` (`:133`).
  - The same pointer handling again in `update` (`:326-433`).
- Both repeat `overlay` (link status), `mouse_interaction`, the find bar's `operate`, and highlight drawing.

**Where they disagree**
- Triple click: whole text in `Selectable`, the unit in the image. Both are the paragraph, since a `Selectable` is one unit. No change.
- Double click at a word's end: the image counts it (`word.end == pos.offset`), text does not. Take the image's rule, so a double click just after a word selects it in both.
- `on_click` exists only on text. The image has no use for it.
- The link under the press: text compares span indices, the image compares URLs. One rule: URLs.

**Desktop: new `selectable/geometry.rs`** (`selectable.rs` → `selectable/text.rs`, `selectable_image.rs` → `selectable/image.rs`)
```rust
/// Hit-testing and highlighting of laid-out text, in the units of `selection::Pos`.
pub trait Geometry {
    fn hit(&self, point: Point) -> Option<Pos>;                 // nearest place
    fn link_at(&self, point: Point) -> Option<&str>;
    fn word_at(&self, pos: Pos) -> Range<usize>;                // in pos.unit; formulas whole
    fn unit_len(&self, unit: usize) -> usize;
    fn highlight(&self, from: Pos, to: Pos) -> Vec<Rectangle>;
    fn line_step(&self, pos: Pos, up: bool) -> Option<Pos> { None }
    fn line_edge(&self, pos: Pos, forward: bool) -> Option<Pos> { None }
}
pub struct Handlers<'a, M> {
    pub on_link: Option<fn(String) -> M>,
    pub on_link_menu: Option<Rc<dyn Fn(String) -> M + 'a>>,
    pub on_gesture: Option<Rc<dyn Fn(Gesture) -> M + 'a>>,
    pub on_click: Option<Rc<dyn Fn() -> M + 'a>>,
}
/// The pointer side both widgets share.
#[derive(Default)]
pub struct Pointer { last_click: Option<mouse::Click>, modifiers: Modifiers,
                     link_pressed: Option<(String, Point)>, click_pressed: Option<Point>,
                     hovered_link: Option<String>, link_status: link_status::Status }
impl Pointer {
    pub fn update<M>(&mut self, g: &impl Geometry, h: &Handlers<'_, M>, dragging: bool,
                     event: &Event, bounds: Rectangle, cursor: mouse::Cursor, shell: &mut Shell<'_, M>);
    pub fn interaction<M>(&self, h: &Handlers<'_, M>, dragging: bool, over: bool) -> mouse::Interaction;
    pub fn overlay<'b, M>(&'b mut self, h: &Handlers<'_, M>) -> Option<overlay::Element<'b, M, Theme, Renderer>>;
}
pub fn draw_highlights(renderer: &mut Renderer, rects: impl Iterator<Item = Rectangle>, at: Vector, color: Color, border: Border);
pub fn report_found(operation: &mut dyn Operation, rects: impl Iterator<Item = Rectangle>, fallback: Rectangle);
```
- `impl Geometry for selectable::text::Layout` (a paragraph is one unit). `impl Geometry for selectable::image::Text`.
- `line_step`/`line_edge` stay text-only, as now (the image steps by paragraph, `text-selection.md`).
- The info cards' own selection (`update_own`) stays in the text widget and uses `Pointer` for clicks.

**Deleted**
- The two pointer `update` bodies, two `overlay`s, two `mouse_interaction`s, two highlight loops, two find `operate`s.
- `State::press`, both `word_at`s (one in `Geometry`).

---

## 5. Desktop: view-driven avatar, preview and math loads

**Today**
Update code requests what it thinks the views show:
- `load_avatars()` is called after 20 messages, `load_previews()` after 9, `render_math()` after 13 (`app.rs:4151, 4167, 3914`).
- What is shown is listed a second time, beside the views, by eleven enumerators:
  - `Main::avatars_on_screen` (`app.rs:586-609`);
  - `sidebar::avatars` (`sidebar.rs:184`), `Room::avatars` (`room.rs:602`), `members::avatars`/`avatars_of` (`members.rs:124, 129`);
  - `actions::reader_avatars`/`reactor_avatars`/`listed_avatars` (`actions.rs:885-917`);
  - `SpaceRooms::avatars` (`directory.rs:336`), `Join::avatars` (`join.rs:277`), `Overview::avatars` (`overview.rs:185`), `Card::avatar` (`info.rs:234`);
  - `Room::previewed` (`room.rs:644`) and `Room::math_documents` (`room.rs:592`).
- The 12 `avatars.view(..)` sites must agree with these lists. A picture shown but not listed never loads. A new view must remember to add both.

**Desktop: new `want.rs`**
```rust
/// `content`, asking for `want` the first time it is drawn (once per widget state).
pub fn wanting<'a, M: Clone + 'a>(content: impl Into<Element<'a, M>>, want: Option<M>) -> Element<'a, M>;
```
- It sends `want` from its `update` on the first `window::Event::RedrawRequested`, as `drag_scroll.rs:134` already reacts to redraws.
- Avatars: `Avatars::view` keeps its signature and returns `Element<'a, app::Message>`. A picture not loaded (nor pending, nor failed within `RETRY_AFTER`) is wrapped with `want = Some(Message::Media(media::Message::AvatarWanted(key)))`.
- Previews: `media::inline_image` wants `PreviewWanted(Media)`.
- Math: `room::event_view` wants `MathWanted(math::Request { document, params })` for a math message without its picture (Q6). A resize or text-size change gives new keys, so new pictures are wanted without `rescaled` asking.
- `MediaState::update` runs today's `request` functions. They already skip what is cached or pending.

**Deleted**
- `load_avatars`, `load_previews`, `render_math` and their 42 call sites. `rescaled` keeps only the PDF.
- `avatars_on_screen` and the eleven enumerators above.

---

## 6. Desktop and iOS: tasks with an owner

**Today**
- Desktop:
  - 80 `Task::perform`/`future`/`run` in `app.rs`, 14 more in other modules.
  - 39 copies of `.map_err(|e| e.to_string())` in `app.rs`.
  - Loads for a room run on after it closes. Their result is dropped by id (`room_left`, `app.rs:2837`). The same for card loads (`app.rs:4126`) and the space's call service (`app.rs:2993`).
  - Ad hoc ownership for six: `Room.seek` and `Room._reply_wait` (`room.rs:201, 236`), `login` browser (`login.rs:90`), the dialog's user search (`app.rs:2913`), the join dialog's request (`app.rs:4037, 4058`), Google sign-in (`app.rs:3126`).
  - `Task::future(..).discard()` drops errors silently: `mark_read` (`app.rs:5364`), typing (`:2319`).
- iOS:
  - 58 `Task {}` and 13 `.task {}`.
  - In views, 18 button actions start `Task { await model.x() }` (e.g. `RoomView.swift:45, 139, 400, 512-513`, `JoinView.swift:52, 138`, `SpaceOverviewView.swift:157, 198, 227`, `RoomInfoView.swift:159, 491`, `TopicViews.swift:207`).
  - Each model has its own "run, show the error" helper: `TimelineModel.act` (`TimelineModel.swift:444`), `RoomInfoModel.call`/`change` (`RoomInfoModel.swift:186-199`), `SessionModel.setNotesAtTop` (`SessionModel.swift:158`), `PersonInfoModel.invite` (`PersonInfoModel.swift:80`).
  - `Subscriptions` (`Core/Feed.swift:77`) owns feeds and some tasks, in 7 models.
- Android already has the two kinds: `viewModelScope` for loads, an `actions` scope that outlives the screen for requests (`TimelineViewModel.kt:120-123`).

**Where they disagree**
- Requests (send, react, pin…) run to the end after leaving the room on all three. Android and iOS still apply the result to the model. The desktop drops it, except a scheduled send (`app.rs:2843`). Keep both: the model is gone on the desktop (Q7).
- Loads: Android cancels them with the screen; iOS cancels `.task {}` and `Subscriptions` ones but not model `Task {}`s; the desktop never cancels. Android's rule for all.

**Desktop: new `task.rs`**
```rust
/// Loads a component owns: aborted when it is dropped, or when one with the same key starts.
#[derive(Default)]
pub struct Owned<K: Hash + Eq = &'static str>(HashMap<K, task::Handle>);
impl<K: Hash + Eq> Owned<K> {
    pub fn run<M>(&mut self, key: K, task: Task<M>) -> Task<M>;
    pub fn cancel(&mut self, key: &K);
    pub fn running(&self, key: &K) -> bool;
}
/// A request to the server: runs to its end whoever asked; its error as text.
pub fn request<T: Send + 'static, M>(
    work: impl Future<Output = mtrx_core::Result<T>> + Send + 'static,
    done: impl FnOnce(Result<T, String>) -> M + Send + 'static,
) -> Task<M>;
/// No result wanted: a failure is logged, not dropped.
pub fn background<M>(what: &'static str, work: impl Future<Output = mtrx_core::Result<()>> + Send + 'static) -> Task<M>;
```
- `Room.tasks: Owned` for members, pinned timeline, call service, seek, reply wait, pagination.
- `Card.tasks: Owned` for user info, overview listing and members.
- `Modal`'s dialog, join and settings hold their searches and sign-ins in `Owned`.
- Downloads: `MediaState.downloads: Owned<MediaRef>`, so one can be cancelled from its list.

**iOS: `Core/Work.swift`** (replaces `Subscriptions`)
```swift
@MainActor
final class Work {
    /// Keep `handle` running and apply each of `feed`'s states.
    func follow<Value>(_ handle: TaskHandle, _ feed: Feed<Value>, _ apply: @escaping @MainActor (Value) -> Void)
    /// A load that ends with the model; replaces one under `key`.
    func load(_ key: String, _ body: @escaping @MainActor () async -> Void)
    /// A request that finishes even if the model goes; a failure goes to `failed`.
    @discardableResult
    func request(_ body: @escaping @MainActor () async throws -> Void,
                 failed: @escaping @MainActor (String) -> Void) -> Task<Void, Never>
    func cancelAll()
}
```
- Model methods called from buttons become synchronous and start their own work. Views lose their `Task {}`. Tests can still await the returned `Task`.
- `.task {}` modifiers stay: they end with the view.

**As built (V)**
- Desktop:
  - A load is done once its result comes: `Owned::running` turns false before the app sees the result, so no handler resets a flag. `Owned::load` is `run` with `request`'s text error.
  - A room and a card key their loads by an enum (`room::Load`, `info::Load`). Components with one or two loads keep their string keys private, behind a method naming the load: `Login::{password_sign_in, browser_sign_in}`, `Dialog::{search_users, offer_owners}`, `Join::browse`, `settings::Panel::{google_sign_in, load_space}`.
  - The room also owns its draft's restore and ⌘R's timeline (`Room.reloading` goes). `Room::reload` cancels the old timeline's pagination and seek.
  - The space's room listing is owned by `SpaceRooms`, by space, not by the card: the room search shares it. `Listed.loading` goes.
  - The image and PDF viewers own their download, so closing one stops it. Login owns both sign-ins; its `State` goes.
  - Downloads stay in core, which cancels them (#84): no `MediaState.downloads`.
  - Not owned: opening a room (its result is still checked against `Pane::Opening`), timers (the sending clock, the mark on a message gone to), and the session-wide caches (avatars, previews, math, contacts, frequent emoji).
  - Errors that are not core's keep their own text: Google, call links, the dialog's requests, recovery's progress. `background` takes core results; work that cannot fail stays `Task::future(..).discard()`.
  - `room_left` stays, for requests' results (7a).

**Deleted**
- Desktop: `Room.seek`, `Room._reply_wait`, `Room.loading_members`, `Room.paginating` (→ `tasks.running`), the handle fields in `login`, `dialog`, `join` and `settings`, the 39 error mappings, `room_left`'s drop path for loads.
- iOS: `Subscriptions`, the 18 view `Task {}`s, `TimelineModel.act`, `RoomInfoModel.call`/`change`, the per-model error `Task {}`s.

---

## 7. iOS: split SessionModel

**Today**
`SessionModel` (`ios/Models/SessionModel.swift`, 453 lines) holds four concerns:
- The account: session, images, presence, sync and foreground (`:184-211`), connection, error, logout and forget (`:392-417`).
- The room lists: `rooms`, `loaded`, `lists`, `drafts`, `joining` (`:17-39`), `apply` (`:135`), badges (`:346-360`), `button(for:)`, notes at the top.
- Navigation and device layout: `tab`, `chatsPath`, `roomsPath`, `invite`, `selectedSpace`, `roomSort`, `foldedGroups`, `sectionLayouts` (`:26-64`), `select`, `open`, `left` (`:274-327`).
- Encryption: `encryption`, `hasOtherDevice`, `verification`, recovery (`:46-58`, `:213-268`).

30 views observe it. Every room-list update re-renders all of them, Settings and verification included.

**iOS** (four objects, Q12)
```swift
@MainActor final class SessionModel: ObservableObject {      // the account
    let session: Session; let images: ImageCache; let presence: PresenceModel
    let rooms: RoomListModel; let home: HomeNavigation; let encryption: EncryptionModel
    @Published private(set) var connection: ConnectionState
    @Published var error: String?
    func start(); func setForeground(_ on: Bool)
    func accept(_ room: RoomSummary) async throws; func decline(_ room: RoomSummary) async throws
    func logout() async throws; func forget() async throws
}
@MainActor final class RoomListModel: ObservableObject {
    @Published private(set) var rooms: [RoomSummary]; loaded: Bool; lists: RoomLists; drafts: Set<String>; joining: Set<String>
    func room(_ id: String) -> RoomSummary?; func button(for room: DirectoryRoom) -> JoinButton
    var chatsBadge: Int; var roomsBadge: Int; var spacesBadge: Int; func backBadge(besides id: String) -> Int
    var notesAtTop: Bool; func setNotesAtTop(_ on: Bool)
}
@MainActor final class HomeNavigation: ObservableObject {
    init(rooms: RoomListModel, defaults: UserDefaults)
    @Published var tab: HomeTab; chatsPath: [String]; roomsPath: [String]; invite: RoomRef?
    @Published private(set) var selectedSpace: SpaceKey?; roomSort: RoomSort; foldedGroups: Set<String>
    let sectionLayouts: SectionLayouts
    func select(_ tab: HomeTab); func open(_ id: String, kind: RoomKind?); func left(_ id: String)
    func selectSpace(_ entry: SpaceKey?); var spacesPath: [SpaceKey]; var roomGroups: [ListSection]
    func toggleGroupFold(_ key: String); func setRoomSort(_ sort: RoomSort); func forget()
}
@MainActor final class EncryptionModel: ObservableObject {
    @Published private(set) var state: EncryptionState?; hasOtherDevice: Bool?; verification: VerificationModel?
    @Published private(set) var requestingVerification: Bool; @Published var verificationError: String?
    let recovery: RecoveryModel; var needsSetup: Bool
    func loadDevices() async; func verifyWithAnotherDevice(); func received(_ request: Verification); func closeVerification()
}
```
- `RootView` puts all four (and `images`, `presence`) in the environment.
- `HomeNavigation` drops a selected space that is gone, by following `RoomListModel` (today `apply`, `:144`).
- `timeline(for:)` and `info(for:)` go: views build the models from `session.session`.

**Deleted**
- Nothing is rewritten: the room-list, navigation and encryption members move out, and `SessionModel` keeps about 120 lines.
- `SessionModel.timeline(for:)` and `info(for:)`.

---

## 8. iOS: a scoped SwipeBack

**Today**
- `ios/Core/SwipeBack.swift:6-16` overrides `viewDidLoad` of every `UINavigationController` in the process (a `@retroactive` extension) and makes it its own pop-gesture delegate.
- It applies to every navigation controller: sheets, `PhotosPicker`, share sheets, web sign-in.
- Overriding a UIKit method in an extension is unsupported: if UIKit or another extension overrides it too, which one runs is undefined.
- It's only needed where `backWithUnread` hides the system back button (`HomeView.swift:633-670`).

**iOS: `Core/SwipeBack.swift`**
```swift
/// Swiping from the left edge goes back on this screen, which hides the system back button.
struct SwipeBack: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> Controller
    func updateUIViewController(_ controller: Controller, context: Context) {}
    final class Controller: UIViewController, UIGestureRecognizerDelegate {
        override func didMove(toParent parent: UIViewController?)   // sets navigationController's pop delegate, keeping the old one
        override func viewWillDisappear(_ animated: Bool)           // puts the old delegate back
        func gestureRecognizerShouldBegin(_ g: UIGestureRecognizer) -> Bool   // viewControllers.count > 1
    }
}
extension View { func swipeBack() -> some View { background(SwipeBack().frame(width: 0, height: 0)) } }
```
- `BackWithUnread` applies `.swipeBack()`.

**Deleted**
- The `UINavigationController` extension.

---

## 9. Android: a SignedInSession holder

**Today**
`Account` (`android/model/Account.kt`, 539 lines) is both the account and the signed-in session:
- Nullable session state: `session`, `subscriptions`, `encryptionSubscription`, `presence`, `recoveryJob` (`:144-154`).
- Ten checks of `session ?: return` or `session === current` (`:224, 260, 312, 335, 351, 357, 365, 391, 424, 450`).
- `close()` resets about 15 fields by hand (`:514-538`).
- `AccountState.SignedIn(session, images, presence)` (`:42`). 18 places reach into it, e.g. `ui/Navigation.kt:79, 121, 122, 179, 293, 321, 331, 336, 340, 377, 425`.
- Session flows live on `Account` and outlast the session: `rooms`, `connection`, `encryption`, `devices`, `recovery`, `verification`, `verificationRequest`.

**Android: `model/SignedInSession.kt`**
```kotlin
/** A signed-in session and what lives with it; closed at sign-out. Used on [scope]'s (main) thread. */
class SignedInSession(
    val session: Session,
    private val scope: CoroutineScope,
    selectedSpace: Preference<SpaceKey?>,      // item 10
    sharePresence: Preference<Boolean>,
    private val notifications: RoomNotifications,
    private val log: Logger,
) : AutoCloseable {
    val info: SessionInfo
    val images: Images
    val presence: PresenceModel
    val rooms: StateFlow<RoomListsInterface>
    val spaceSelection: SpaceSelection
    val connection: StateFlow<ConnectionState>
    val encryption: StateFlow<EncryptionState?>
    val devices: StateFlow<List<DeviceInfo>?>
    val recovery: StateFlow<Recovery>
    val verification: StateFlow<VerificationModel?>
    val verificationRequest: StateFlow<Verification?>
    val messages: SharedFlow<String>

    fun loadDevices(): Job; fun recover(recoveryKey: String); fun resetRecovery()
    fun verifyWithAnotherDevice(); fun acceptVerificationRequest(); fun declineVerificationRequest()
    fun closeVerification(model: VerificationModel)
    fun setNotesAtTop(on: Boolean): Job
    fun accept(roomId: String, name: String, onDone: (Boolean) -> Unit = {}): Job
    fun leave(roomId: String, name: String, onDone: (Boolean) -> Unit = {}): Job
    internal suspend fun startSync(); internal suspend fun stopSync()   // by Account, under its lock
    override fun close()
}
sealed interface AccountState {
    data object Loading : AccountState
    data class SignedOut(val error: String? = null) : AccountState
    data class SignedIn(val session: SignedInSession) : AccountState
}
```
- `Account` keeps: restore, sign-in (password and browser), sign-out, forget, foreground, push registration, the requested room, the open room, and the device preferences (`roomSort`, `sections`, `sharePresence`).
- Screens take a `SignedInSession` instead of `Account` plus `AccountState.SignedIn`.

**Deleted**
- `Account`'s nullable session fields, the ten checks, `close()`'s field resets, and its session flows.

---

## 10. Android: one Preference<T>

**Today**
The same holder is written four times: a store interface, a `MutableStateFlow`, a `changed` flag so a late load doesn't overwrite a choice, and a save `Mutex`.
- `SharePresence` (`model/Presence.kt:50-79`) with `SharePresenceStore` (`:39`).
- `RoomSortChoice` (`model/Spaces.kt:137-159`) with `RoomSortStore`.
- `SpaceSelection` (`model/Spaces.kt:52`) with `SelectedSpaceStore` (`:27`). It adds the "wait for a list that has it" rule.
- `SectionLayouts` (`rooms/Sections.kt:58-105`) with `SectionStore` (`:43`).
- Four DataStore stores (`model/ViewPreferences.kt:23-143`).
- Tests use a fake per store (`RoomSortChoiceTest`, `SpaceSelectionTest`, `PresenceTest`).

`PushStore` (`model/PushPreferences.kt`) is read on demand by the push service, outside a scope. It stays as it is.

**Android: `model/Preference.kt`**
```kotlin
/** Where one value is kept across launches. */
interface PreferenceStore<T> {
    /** The saved value; null if none, or it cannot be read. */
    suspend fun load(): T?
    suspend fun save(value: T)
}

/** A device preference: [value] at once (the default until the saved one is read), saved in order on change. */
class Preference<T>(private val store: PreferenceStore<T>, default: T, private val scope: CoroutineScope) {
    val value: StateFlow<T>
    /** The value, once the saved one is read. */
    suspend fun current(): T
    fun set(value: T)
    fun update(change: (T) -> T)
}

/** One key of [data], as text; [encode] null removes it. */
class DataStoreText<T>(
    private val data: DataStore<Preferences>,
    private val key: Preferences.Key<String>,
    private val encode: (T) -> String?,
    private val decode: (String) -> T?,
) : PreferenceStore<T>
class DataStoreFlag(private val data: DataStore<Preferences>, private val key: Preferences.Key<Boolean>) : PreferenceStore<Boolean>

/** For tests and previews. */
class MemoryStore<T>(var saved: T? = null) : PreferenceStore<T>
```
- `sharePresence: Preference<Boolean>`, `roomSort: Preference<RoomSort>`.
- `SectionLayouts` wraps a `Preference<Map<String, SectionPrefs>>` and keeps its change functions.
- `SpaceSelection` wraps a `Preference<SpaceKey?>` and keeps its list rule.

**Deleted**
- `SharePresence`, `RoomSortChoice`, the four `*Store` interfaces, the four DataStore classes (the JSON encoding of sections stays as `encode`/`decode`), three `changed` flags and four save mutexes. The test fakes become `MemoryStore`.

---

## 11. Android: the timeline model apart from the screen

**Today**
- `TimelineViewModel` (`android/timeline/TimelineViewModel.kt`, 563 lines) is the whole room screen's model:
  - rows, paging, seek, read (`:65-262, 486-512`);
  - composer, draft and typing (`:266-431`);
  - pinned messages (`:103-109, 186-218`);
  - rooms shared with a person (`:98-101, 440-451`);
  - saving the topic (`:164-173`);
  - the person sheet (`:94-96, 433-456`).
- `TimelineScreen.kt` is 1259 lines: the screen, `TimelineContent` with its overlays (`:287-600`), the composer (`:722`), bubbles and rows (`:808-1259`).
- iOS splits it:
  - `TimelineModel` has rows, composer, actions and the person sheet.
  - `PinnedModel` (`ios/Models/Topics.swift:96`) has the pinned messages.
  - The topic is saved by the room's `RoomInfoModel` (`RoomInfoModel.swift:71`, held by `RoomView.swift:12`).
  - The shared rooms are loaded by their view (`RoomView.swift:325-331`).
- Android's `RoomInfoModel` subscribes to settings and loads members in its constructor (`rooms/RoomInfoModel.kt:51-62`). iOS loads them in `load()` when the info shows.

**Android** (as iOS, Q11)
```kotlin
class TimelineViewModel(roomId: String, session: Session, onRead: () -> Unit, onInput: () -> Unit) : ViewModel(), TimelineActions
    // rows, paging, seek, read, composer, typing, message actions, person sheet (as iOS TimelineModel)

class PinnedModel(roomId: String, session: Session) : ViewModel() {
    val pinned: StateFlow<List<EventRow>?>
    fun show(); fun hide()
}

class RoomInfoModel(...) : ViewModel(), RoomInfoActions {
    fun load()                                  // settings and members, when the info shows (as iOS)
    fun saveTopic(topic: Topic)                 // moved from TimelineViewModel
}

@Composable fun SharedRoomsPanel(userId: String, session: Session, ...)   // loads in a LaunchedEffect, as iOS
```
- `RoomPane` (`Navigation.kt:275`) holds the room's `TimelineViewModel`, `PinnedModel` and `RoomInfoModel` in its store.
- `TimelineActions` loses `saveTopic`, `showPinned`, `hidePinned`, `showSharedRooms`.
- `TimelineScreen.kt` is split into `TimelineScreen.kt` (screen and overlays), `Composer.kt` and `EventRows.kt`.

**Deleted**
- From `TimelineViewModel`: pinned, shared rooms and topic saving (about 110 lines).

---

## 12. Android: typed navigation destinations

**Today**
- Routes are strings (`ui/Navigation.kt:61-67`).
- Nine `navigate` calls (`:100-137, 224, 260`). Five build their string with `Uri.encode`, one with a literal query.
- Arguments are read back by name, e.g. `entry.arguments?.getString("roomId")` (`:149, 154, 202, 203`).
- A typo or a missing `Uri.encode` compiles.

**Android: `ui/Destination.kt`**
```kotlin
sealed interface Destination {
    data object Home : Destination
    data class Info(val roomId: String) : Destination
    data class Space(val spaceId: String) : Destination
    data object Join : Destination
    data class New(val space: Boolean, val parent: String?) : Destination
    data object Recovery : Destination
    data object Verification : Destination
}

/** A kind of destination: its route pattern and arguments, and the destination from an entry. */
class Route<D : Destination>(val pattern: String, val arguments: List<NamedNavArgument>, val read: (Bundle) -> D?) {
    companion object { val HOME: Route<Destination.Home>; val INFO: Route<Destination.Info>; /* … */ }
}

fun Destination.route(): String                                     // the one place routes are written
fun NavHostController.go(to: Destination, options: NavOptionsBuilder.() -> Unit = {})
fun <D : Destination> NavGraphBuilder.screen(route: Route<D>, content: @Composable (D) -> Unit)
```
- No new dependency (Q9).

**Deleted**
- The seven route constants, the six string builds and the four argument reads.

---

## 13. FFI: delete unused functions

**Today** (checked against `ios/`, `android/`, `android-test/`, `apps/android/app/src/androidTest/`; the desktop and CLI use core, not FFI)

| FFI | Where | Used by apps | Core counterpart |
|---|---|---|---|
| `Session::encryption_state` | `ffi/encryption.rs:258` | no (phones subscribe) | used by `subscribe_encryption_state` |
| `Session::recovery_status` | `ffi/encryption.rs:279` | no (`EncryptionState.recovery`) | CLI `cli/main.rs:466` |
| `Session::enable_recovery` | `ffi/encryption.rs:306` | no | CLI, desktop tests |
| `Session::is_device_verified` | `ffi/encryption.rs:311` | no | desktop `app.rs:1021, 1989`, CLI |
| `Session::media_file` (+ `media_file_name`, its test) | `ffi/session.rs:120-203` | no | — |
| `Room::set_picture`, `remove_picture` | `ffi/room.rs:83, 88` | no (no picture change on phones) | desktop `app.rs:1453, 1458` |
| `role_level` | `ffi/manage.rs:471`, export `ffi/lib.rs:42` | no | — |
| `Member.role` | `ffi/manage.rs:406, 440` | only `android-test/Fakes.kt:634` | core sorts by it (`core/members.rs:108`) |
| `Presence.currently_active` | `ffi/presence.rs:75, 94, 111, 271, 288` | only `android-test/Fakes.kt:772` | CLI `cli/presence.rs:70` |

Found besides (Q10):

| FFI | Where | Used by apps |
|---|---|---|
| `RoomLists::parents`, `ancestors`, `tree` (+ `TreeEntry`) | `ffi/room_list.rs:472, 483, 494` (#73) | only `NoRooms` overrides (`android/rooms/RoomLists.kt:24, 51, 74`) |
| `TaskHandle::is_finished` | `ffi/runtime.rs:116` | only fakes (`android-test/Fakes.kt:110`, `ios/Core/UITesting.swift:58`) |

Kept:
- `Role` (for `DirectoryRoom.role`), `RecoveryStatus` (for `EncryptionState.recovery`), `Session::pushers` (Android end-to-end test, `PushEndToEndTest.kt:116`).
- `Session::reconnect`: item 14 wires it up.

**Change**
- Delete the FFI items above. Core keeps its functions where the desktop or CLI use them.
- Core `Role::level` (`core/members.rs:55`) stays: the desktop tests use it (`app/tests.rs:64`).
- Regenerate the bindings; update the fakes.

---

## 14. Wire up reconnect

**Today**
- Core: `Session::reconnect` (`core/connection.rs:139`) is `sync.start()`, the same as `start_sync` (`core/session.rs:232`). It does so even while online or signed out.
- Sync runs in offline mode (`core/session.rs:182`): while offline it checks the server with a back-off.
- Desktop: sync never stops. `reconnect` is called on "Retry now" in the offline bar (`app.rs:4635`) and on ⌘R while offline (`app.rs:1533`). Nothing on focus, wake or network change.
- iOS: `scenePhase` (`AppModel.swift:75`) → `SessionModel.setForeground` (`:188`) stops sync in the background and starts it in the foreground. Nothing on network change. The FFI `reconnect` is never called.
- Android: `ProcessLifecycleOwner` (`MtrxApplication.kt:84`) → `Account.setForeground` (`:174`) → `updateSync` (`:491`), the same as iOS. Nothing on network change.

**Where they disagree**
- The phones restart sync on every foreground. The desktop waits for the back-off after sleep or a network change, up to the SDK's longest interval.

**Core**
```rust
impl Session {
    /// Try the server now if it was unreachable: the network came back, or the app came to the front.
    /// Nothing while online or signed out.
    pub async fn reconnect(&self);
}
```
- A test: online, `reconnect` leaves sync alone.

**Apps**: reconnect when the network comes back, and when the app comes to the front while offline.
- Desktop:
  - `WindowFocused(true)` while `Connection::Offline` → `reconnect`.
  - macOS: `NSWorkspaceDidWakeNotification` → `reconnect` (Q8).
  - "Retry now" and ⌘R stay.
- iOS: `Core/NetworkMonitor.swift` wraps `NWPathMonitor` (Network framework). A path becoming `.satisfied` → `session.reconnect()`. It runs while in the foreground.
- Android: `SignedInSession` registers `ConnectivityManager.registerDefaultNetworkCallback`. `onAvailable` → `reconnect()`. It runs while in the foreground.
- Foreground on the phones stays `startSync`.

**FFI**
- No change but the doc comment.

---

## PR split

Most PRs are on one platform, so the step-2/3 pattern (core and FFI, then desktop, iOS, Android) applies only to P, Q and V. Each commit builds. On the desktop, each commit keeps `app/tests.rs` passing.

| PR | Item | Commits | Size | Depends on |
|---|---|---|---|---|
| P | 13 unused FFI | 1 FFI and core; 2 iOS (fake); 3 Android (fakes, `NoRooms`) | S | — |
| Q | 14 reconnect | 1 core and FFI; 2 desktop; 3 iOS; 4 Android | S | X, AB |
| R | 5 view-driven loads | 1 `wanting` and avatars; 2 previews; 3 math | M | — |
| S | 3 `MessageField` | 1 the field and the composer; 2 the inline edit | M | — |
| T | 2 menu model | 1 `menu::Entry` and `view`, builders switch; 2 card and viewer menus into `Popup`, `ClosePopup` | M | — |
| U | 4 selectable geometry | 1 `Geometry`, `Pointer`, text widget; 2 image widget | M | — |
| V | 6 owned tasks | 1 desktop `Owned`, `request`, `background`; 2 iOS `Work` | M | AB |
| W | 1 `app.rs` into components | 1 nested `Message` and `Cx` (no moves); 2 `window`, `start`; 3 `Room::update` and feature updates; 4 `Sidebar`, `Pane`; 5 `Card`, `Modal`; 6 `Account`, `People`, `MediaState` | L | R, S, T, V |
| X | 9 `SignedInSession` | 1 Android | M | — |
| Y | 10 `Preference<T>` | 1 Android | S | X |
| Z | 11 Android timeline split | 1 `PinnedModel`, lazy `RoomInfoModel` with `saveTopic`, shared rooms in their view; 2 `TimelineScreen.kt` into three files | M | — |
| AA | 12 typed destinations | 1 Android | S | X |
| AB | 7 `SessionModel` split | 1 iOS | M | — |
| AC | 8 scoped `SwipeBack` | 1 iOS | S | — |

Order:
- First the small independent ones: P, AC, U.
- Desktop: R, S, T, then V, then W. The first four shrink `app.rs` (about 42 load calls, the field and menu handlers, 39 error mappings) so W mostly moves code.
- iOS: AB before V's iOS commit and Q.
- Android: X, then Y, AA and Z in any order.
- Q last.

Rough size:
- W is the widest: about 4500 lines move, about 300 change. The desktop tests change their top-level message constructors (about 70 in `app/tests.rs`).
- AB touches the 30 views that observe `SessionModel`.
- X touches the 9 files that take `Account`.

---

## Open questions

1. **How desktop components reach each other.**
   - (a) `update(&mut self, message, &mut Cx) -> Task<app::Message>`. Other components are reached by sending app messages. Views keep producing `app::Message`.
   - (b) Each component has its own `Message` and `Action`. Views `.map` into the parent's messages. The parent runs each `Action`.
   - (c) (b) for new components only, (a) for the room.
   - Recommend (a). It matches today's views (`room::Message::to(id)`) and the existing `Task::done(Message::LinkClicked(..))`. The `Action` enums that exist stay.

2. **Shape of the desktop `Message`.**
   - (a) One variant per component (`Message::Sidebar(sidebar::Message)` and so on).
   - (b) Keep it flat and only move the handlers.
   - Recommend (a). The routing then says which component owns what.

3. **The macOS menu bar and the in-window menus.**
   - (a) In-window menus share `menu::Entry`. The menu bar keeps its static table, with selectors and shortcuts.
   - (b) One generic `Entry<A>` for both.
   - Recommend (a). They share no items at run time; the Edit items already reach fields as standard actions.

4. **Menus kept outside `Popup`** (the card's picture and add-to-space menus, the viewer's menu).
   - (a) Move them into `Popup`, closed by `ClosePopup`.
   - (b) Keep their local bools.
   - Recommend (a). It fixes Esc closing the whole card (a bug).

5. **Mentions in the inline edit** (desktop only; the phones edit in the composer).
   - (a) One `MessageField`; the inline edit gains mention completion.
   - (b) Edit in the composer, as iOS and Android.
   - (c) One `MessageField`, no completion in edits.
   - Recommend (a). Where an edit is shown is platform UI (the desktop edits in place, decided earlier). Completing mentions while editing is what iOS does, and the shared field gives it for free.

6. **Math pictures view-driven too.**
   - (a) Yes, with avatars and previews.
   - (b) Avatars and previews only.
   - Recommend (a). It deletes `render_math`'s 13 calls, and a resize needs no request.

7. **Results of requests after their room closed.**
   - (a) Keep today: requests run to the end; the desktop drops the result (except a scheduled send), the phones apply it to their model.
   - (b) Cancel requests too when the room closes.
   - Recommend (a). A send or reaction must not depend on staying in the room.

8. **Desktop network and wake detection.**
   - (a) Focus while offline on all platforms, plus `NSWorkspaceDidWakeNotification` on macOS. This adds the `NSWorkspace` and `NSNotification` features to the existing `objc2-app-kit` and `objc2-foundation` dependencies. No new crate.
   - (b) A new crate that reports network changes on all three desktop platforms (for example `if-watch`). This is a new dependency.
   - (c) Focus only.
   - Recommend (a). Waking with the window already focused is the case focus misses.

9. **Typed destinations on Android.**
   - (a) A hand-written `Destination` and `Route` (item 12). No new dependency.
   - (b) Navigation's own type-safe routes. These need the `kotlinx-serialization` plugin and runtime: new dependencies.
   - Recommend (a). Seven destinations don't justify a new plugin.

10. **Unused FFI found besides the review's list** (`RoomLists.parents/ancestors/tree`, `TreeEntry`, `TaskHandle.is_finished`).
    - (a) Delete them in P too.
    - (b) Keep `tree` for nested spaces in the phones' space overview (desktop-only so far).
    - Recommend (a). They come back with the feature that uses them, with its own design.

11. **Scope of the Android timeline split.**
    - (a) As iOS: pinned, topic and shared rooms leave `TimelineViewModel`; the composer stays; `RoomInfoModel` loads lazily.
    - (b) Also move the composer into its own model, on both phones.
    - Recommend (a). iOS keeps the composer in `TimelineModel`, and (b) would change iOS too.

12. **How far to split iOS `SessionModel`.**
    - (a) Four objects: `SessionModel`, `RoomListModel`, `HomeNavigation`, `EncryptionModel`.
    - (b) Two: the session (with lists and encryption) and navigation.
    - Recommend (a). Room-list updates then re-render only the views that show rooms.

---

## Decisions (2026-11-06)

All recommendations taken: 1a, 2a, 3a, 4a, 5a, 6a, 7a, 8a, 9a, 10a, 11a, 12a. The user asked for performant, robust and conceptually clean code and left the choices to the recommendations.
