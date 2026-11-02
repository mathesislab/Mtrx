# Step 3: core consolidation

Goal: one type per concept in core, and one way to follow state. Step 2 gave the apps finished state. This step removes the parallel types and repeated loads that remain behind it, in core and FFI as well as the apps.

Rules from step 2 that still apply:
- iOS behaviour is canonical, unless it's clearly a bug.
- Core returns English text (Q6).
- Rows and seek stay on the FFI `Timeline`, and `DraftKeeper` stays its own object (Q5).

Paths: `core/` = `crates/mtrx-core/src/`, `ffi/` = `crates/mtrx-ffi/src/`, `app/` = `crates/mtrx-app/src/`, `cli/` = `crates/mtrx-cli/src/`, `ios/` = `apps/ios/Mtrx/`, `android/` = `apps/android/app/src/main/kotlin/org/mtrx/app/`.

Note: the generated Swift in `apps/ios/MtrxFFI/Sources/MtrxFFI/mtrx.swift` is stale (it has no `RoomLists` or `TimelineRow`). The FFI crate is the reference.

---

## 1. One SpaceGraph

**Today**
Space parent/child links are read from `m.space.child` in five places and walked in six:
- Core:
  - `rooms.rs:192-203` `summarize` builds `RoomSummary.parents` on every room-list update (via `rooms.rs:289` `space_children`).
  - `spaces.rs:11` `top_level_spaces` and `spaces.rs:21` `descendants`.
  - `spaces.rs:101` `SpaceMembers`.
  - `lists.rs:493-515` `first_space_of`, `chat_spaces`, `in_space`.
  - `room_settings.rs:231` `space_links` loops over every joined space and reads its child event again (`room_settings.rs:471` `child_link`).
  - `room_settings.rs:433` `parent_spaces` does the same loop a third time, for `set_room_access(SpaceMembers)`.
  - `permissions.rs:178` `let_everyone_edit_space` walks children with its own stack.
- Desktop:
  - `app/sidebar.rs:47-69` `ancestors` (BFS up `parents`), used at `app.rs:3394` (call service) and `app.rs:5425` (people suggestions).
  - `app/directory.rs:421-443` `searched_spaces` (with `descendants`).
  - `app/directory.rs:400-403` counts children from `parents`.
  - `app/dialog.rs:900-907` (AddExisting) and `app/info.rs:618, 1197-1203` read direct `parents` only.
  - `app/overview.rs:316-363` expands nested spaces recursively.
- CLI: `cli/rooms.rs:12-41, 54-70` builds its own children map and top-spaces rule.
- iOS/Android: no graph logic. Both read `RoomLists`. iOS reads `RoomSummary.parents` once (`ios/Views/RoomInfoView.swift:244`).

A space's rooms as the server lists them (`directory.rs:203` `Session::space_children`, the hierarchy API, unjoined rooms too) are cached separately in each app:

| | Cache | Joined rooms the server omits |
|---|---|---|
| iOS | `ios/Models/SessionModel.swift:438-447`; never invalidated | not shown (footer explains, `SpaceOverviewView.swift:223`) |
| Android | `android/rooms/Directory.kt:69-81`; per process | not shown |
| Desktop | `app/directory.rs:266-379` `SpaceRooms`; fresh 60 s | merged in from `parents` (`:345`) → **Q3** |

**Core: new `core/space_graph.rs`**
```rust
/// Joined spaces and what they list (m.space.child with `via`), from the room list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpaceGraph {
    children: HashMap<OwnedRoomId, Vec<Child>>,   // space -> children, in event order
    parents: HashMap<OwnedRoomId, Vec<OwnedRoomId>>,
}
pub struct Child { pub room: OwnedRoomId, pub suggested: bool, pub via: Vec<OwnedServerName> }

impl SpaceGraph {
    pub(crate) async fn read(spaces: impl Iterator<Item = &Room>) -> Self;   // replaces summarize's loop
    pub fn parents(&self, room: &RoomId) -> &[OwnedRoomId];                 // direct
    pub fn ancestors(&self, room: &RoomId) -> Vec<OwnedRoomId>;             // nearest first, cycle-safe
    pub fn children(&self, space: &RoomId) -> &[Child];
    pub fn descendants(&self, space: &RoomId) -> HashSet<OwnedRoomId>;      // cycle-safe
    pub fn top_level(&self, rooms: &[RoomSummary]) -> Vec<OwnedRoomId>;     // joined spaces in no joined space, list order
    pub fn tree(&self, space: &RoomId) -> Vec<(usize, OwnedRoomId)>;         // depth-first, for overview and CLI
    pub fn child(&self, space: &RoomId, room: &RoomId) -> Option<&Child>;   // replaces child_link
}
```
- `RoomListState` and `RoomLists` carry `spaces: Arc<SpaceGraph>`. Session keeps the latest in a `watch`, so `space_links`, `parent_spaces`, `let_everyone_edit_space` and `contacts` read it instead of the store.
- `RoomSummary.parents` goes (Q8).
- `RoomLists` gains `parents(room)`, `ancestors(room)` and `tree(space)`. `first_space_of` and `chat_spaces` use the graph.
- Server listing: `Session::space_children` caches per space (60 s) and merges in joined children from the graph (Q3). It stays one-shot.
- The space call service: `RoomHandle::call_service()` (item 7) walks `ancestors` in core. This replaces the desktop loop at `app.rs:3416-3425`.
- `m.space.parent` is still not read (no change).

**FFI**
- `RoomLists` gains `parents(room_id)`, `ancestors(room_id)` and `tree(space_id) -> Vec<TreeEntry{depth, room: RoomSummary}>`.
- `RoomSummary.parents` goes.

**Deleted**
- Core: `spaces::descendants` and `top_level_spaces` as free functions, `rooms::space_children` (moves into the graph), the loops in `space_links`, `parent_spaces` and `let_everyone_edit_space`.
- Desktop: `sidebar::ancestors`, the children count in `directory::from_summary`, the recursion in `overview::push_rooms`, `SpaceRooms` caching and merging (`directory.rs:266-379`).
- CLI: `rooms.rs` children map and top-spaces rule (it uses `tree`).
- iOS: `SpaceChildrenCache`.
- Android: `SpaceChildrenCache`.

---

## 2. One JoinRule, one Membership

**Today**
Join rule: three types, five mappings from ruma.
- Core:
  - `room_settings.rs:70` `Access`, from `Option<JoinRule>` (`:81`).
  - `directory.rs:31` `JoinAccess`, from `JoinRuleSummary` (`:154`) and `JoinRuleKind` (`:165`). Same four cases, different order.
  - `rooms.rs:63` `RoomSummary.public: bool` (from `JoinRule::Public`).
- FFI: `ffi/manage.rs:21` `Access`, `ffi/rooms.rs:11` `JoinAccess`, `RoomSummary.public`.
- Desktop:
  - `app/directory.rs:404-413` maps `public` back to `JoinAccess::{Public, Invite}`, losing Knock and SpaceMembers.
  - `app/sidebar.rs:1433, 1494` `access_symbol(public)`.
  - Text in `app/info.rs:1141-1147, 1205-1215, 1282-1292` and `app/directory.rs:221-229`.
  - A local `struct Access` in `app/settings.rs:250` (same name, different meaning).
- iOS text: `ios/Views/RoomInfoView.swift:241-248` (picker), `:433-438` (facts), `ios/Views/SpaceOverviewView.swift:335-351` (directory).
- Android text: `android/rooms/SettingsText.kt:21-27, 47`, `android/rooms/SpaceOverview.kt:345-349`.

Membership: a bool in three places, plus a three-case enum.
- `RoomSummary.invited`, `Member.invited`, and `directory.rs:44` `Membership{Joined, Invited, Asked}` (directory rows only).
- Joined/asked is recomputed in each app with local overlays:
  - iOS: `SessionModel.joined` (`:182-184`), `JoiningModel.shown` and its `asked` set (`ios/Models/DirectoryModels.swift:97-123`).
  - Android: `joinedIds()` (`android/ui/Navigation.kt:245`, used at `:174, :416`), `withMembership()` (`android/rooms/Directory.kt:65`), the same `act()` in `JoinModel.kt:156-169` and `SpaceOverviewModel.kt:70-81`.
  - Desktop: `directory::membership` (`app/directory.rs:35-42`); the asked/busy state three times (`join.rs:104`, `overview.rs:90`, `directory.rs:281`); join/knock in three paths (`app.rs:1655-1681, 4134-4152, 5225-5248`).
- The join-button rule is written three times: iOS `DirectoryRows.action` (`DirectoryModels.swift:72-87`), Android `rowAction` (`Directory.kt:12-35`), desktop `directory.rs:176-203`.

**Where they disagree** (iOS wins)

| What | iOS | Android | Desktop |
|---|---|---|---|
| Directory row of a room we're invited to | "Invited" → invite | "Join" | "Invited" → opens |
| Rows built from the room list | — | — | Public or Invite only (loses Knock, SpaceMembers) |
| Facts line | 4 lines ("Private: space members…") | "Public" / "Private" | same as iOS |
| "Space members" offered | room has `parents`, or it's current | `settings.spaces` non-empty, or current | `parents` non-empty, or current |

**Core: `core/rooms.rs`**
```rust
/// Who may join a room. One type for settings, directory rows and the room list.
pub enum JoinRule { Invite, SpaceMembers, Knock, Public }      // order of the settings picker
impl From<Option<&ruma::JoinRule>> for JoinRule; impl From<&JoinRuleSummary> for JoinRule; impl From<&JoinRuleKind> for JoinRule;
impl JoinRule { pub fn title(self) -> &'static str; pub fn fact(self) -> &'static str; }   // iOS wording (Q7)

/// Our membership of a room.
pub enum Membership { Joined, Invited, Asked }

/// What a listed room's button does (iOS rules).
pub enum JoinAction { Open, Join, AskToJoin, Invited, Asked, InviteOnly }
impl DirectoryRoom { pub fn action(&self, membership: Option<Membership>) -> JoinAction; }
impl JoinAction { pub fn title(self) -> &'static str; }
impl RoomLists {
    /// From the list, rooms joining (step-2 echo) and asks made here (new `Echoes.knocking`).
    pub fn membership(&self, room: &RoomId) -> Option<Membership>;
}
```
- `RoomSummary`: `invited` becomes `membership: Membership`, and `public` becomes `join_rule: JoinRule` (Q8).
- `Member`: `invited` becomes `membership`.
- `knock` records an `Asked` echo, the same as `join` records `joining`.
- `RoomSettings.join_rule_choices: Vec<JoinRule>` (core decides when Space members is offered: the room has parents in the graph, or it's the current rule).
- `MembershipChange` (timeline events) stays: it's a change, not a state.
- To check in the PR: whether the SDK room list includes knocked rooms. Today these would show as joined.

**FFI**
- One `JoinRule` enum replaces `Access` and `JoinAccess`. One `Membership` enum.
- `RoomSummary.membership` and `.join_rule`; `Member.membership`.
- `DirectoryRoom.membership` stays (server's view). Apps use `lists.membership(id)` for what to show.
- `JoinAction` enum plus `join_action(room: DirectoryRoom, membership: Option<Membership>) -> JoinAction` and `join_action_title(action)` (records have no methods). Alternatively, `RoomLists.join_action(room)`.
- `join_rule_title`, `join_rule_fact`.

**Deleted**
- iOS: `DirectoryRows.action` and its titles (`DirectoryModels.swift:46-87`), `JoiningModel.asked/shown/relisted`, `SessionModel.joined`, the three `Access` label switches, `DirectoryRows.role`.
- Android: `RowAction`, `rowAction`, `withMembership`, `joinedIds`, the duplicated `act()`, the `Access` labels in `SettingsText.kt`, `accessOptions`.
- Desktop: `directory::membership`, `from_summary` access mapping, `access_symbol`, the three asked flags, two of the three join paths, the `info.rs` text mappings, and the access choices filter (`info.rs:607-623`).
- Core: `Access`, `JoinAccess`, `directory.rs:154-172`.

---

## 3. One Rights value from one power_levels()

**Today**
Each right has its own read of the power levels:
- Core:
  - `permissions.rs:203` `power_levels`; `pins.rs:93-94` reads `room.power_levels()` directly; `calls.rs:287-293` `can_send_state` reads it a third way (without the "not known yet" check).
  - `pins.rs:74` `can_pin`, `calls.rs:245` `can_change_topic`, `calls.rs:268` `can_set_space_call_service`, `permissions.rs:144` `can_change_permissions`, `permissions.rs:150` `can_mention_room`.
  - `room_settings.rs:146-175` has 12 `can_*` fields, plus `SpaceLink.can_change`, each space's levels read again (`:238`).
  - `members.rs:146` `manage` (per target); `directory.rs:183` `own_role`; `timeline.rs:1043` `can_pin` for rows.
- Desktop: seven separate async loads (`app.rs`):
  - `room_settings` (`:5372`);
  - `can_change_topic` from three places (`:3396, :922, :5124`);
  - `can_pin` (`:3409`; core also computes it for rows);
  - `can_mention_room` (`:5260`);
  - `permissions` + `can_change_permissions` (`:5331-5341`, again for the space);
  - `can_set_space_call_service` (`:3002-3008`).
  Four of them fire on every room open.
- iOS: one `roomSettings` per `RoomInfoModel`. `RoomView` makes its own just for `canChangeTopic` (`ios/Views/RoomView.swift:53, 78`).
- Android: `TimelineViewModel.kt:170-186` loads `roomSettings` again just for `canEditTopic`.
- `Session.can_pin` (`ffi/manage.rs:560`) is unused on both phones.

**Where they disagree**
- Inviting someone to a space from their person sheet:
  - iOS offers every top-level space (`ios/Views/PersonInfoView.swift:21`).
  - Android does the same (`android/timeline/PersonInfoSheet.kt:99`).
  - The desktop doesn't check in `members.rs:63` or `sidebar.rs:1137, 1197`, but does check `can_invite` in `info.rs:501` and `overview.rs:417`.
- "New room in space": no app checks the right (`ios/Views/HomeView.swift:172`, `android/ui/HomeScreen.kt:233`, desktop add menu).

Offering an action the server will refuse looks like a bug → **Q4**.

**Core: `core/permissions.rs`**
```rust
/// What we may do in a room, from its power levels. All false while they are not known yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rights {
    pub send: bool, pub redact_others: bool, pub pin: bool, pub mention_room: bool,
    pub rename: bool, pub change_topic: bool, pub change_picture: bool,
    pub invite: bool, pub kick: bool, pub ban: bool,
    pub change_join_rule: bool, pub change_history: bool, pub change_address: bool,
    pub change_levels: bool, pub encrypt: bool, pub add_rooms: bool, pub set_call_service: bool,
    pub level: Option<i64>, pub default_level: i64,   // ours (None = creator); a plain member's
}
impl Rights { pub fn of(levels: &RoomPowerLevels, own: &UserId) -> Self; }   // pure, tested once
pub(crate) async fn levels(room: &Room) -> Option<RoomPowerLevels>;          // the one reader
pub(crate) async fn rights(room: &Room) -> Rights;
```
- `Permissions{Everyone, Moderators}` stays. It's a preset, not a right.
- `Manage` (per member) is computed from the same `levels` call.
- Rows take `rights.pin`.
- Where it lives → **Q1**. Recommended: `RoomSummary.rights` (live through the room list), and `RoomSettings.rights` is the same value.
  - Cost: the room list reads power levels per room on each update. It's cached per room by the power-levels event ID, so only rooms whose levels changed are read again.

**FFI**
- A `Rights` record; `RoomSummary.rights`, `RoomSettings.rights`.
- `can_pin` goes.

**Deleted**
- Core: `can_pin` (both), `can_change_topic`, `can_change_permissions`, `can_mention_room`, `can_set_space_call_service`, `can_send_state`, `own_role`'s own read, the 12 `can_*` fields, and `SpaceLink.can_change` (from the space's `rights.add_rooms`).
- Desktop: `load_can_edit_topic`, `can_pin` load, `can_mention_room` load, `load_access`, `pins.rs:36` `can_pin` state, `Room.can_mention_room`.
- iOS: `RoomView`'s own `RoomInfoModel` for `canChangeTopic`. `levelsHint` takes the levels from `Rights` (`ios/Views/MembersViews.swift:195`).
- Android: the `roomSettings` load in `TimelineViewModel.kt:170-186`, `canEditTopic`.

---

## 4. One User type

**Today**
Six shapes of "a person", and the display-name fallback written about ten times.
- Core:
  - `rooms.rs:337` `UserSummary`, `timeline.rs:141` `Reader`, `timeline.rs:76-78` `Event.sender/sender_name/sender_avatar`, `timeline.rs:311-312` `Reply.sender/sender_name`.
  - `members.rs:18` `Member{id, name, avatar, ...}`, `room_settings.rs:139` `Banned{id, name, reason}`, `people.rs:19, 38` `Contact.user`, `UserInfo.user`.
  - `notifications.rs:40` and `push.rs:139` (sender name strings).
- FFI:
  - `ffi/types.rs:102` `UserProfile`, `ffi/types.rs:277` `Reader`, the sender fields of `TimelineEvent` (`:189`) and `ReplyQuote` (`:299`).
  - `ffi/manage.rs:241` `Member`, `ffi/manage.rs:63` `Banned`, `ffi/push.rs:42` `NotificationContent.sender_*`.
- Name fallbacks:
  - Core `members.rs:259` `shown_name` → full ID.
  - `push.rs:224` and `notifications.rs:232` → localpart.
  - iOS `TimelineRows.swift:70-97, 133-137` → localpart; `MembersViews.swift:76-79` → full ID; `RoomInfoView.swift:379` → full ID.
  - Android `TimelineRows.kt:107, 154, 181-184` → localpart; `RoomInfo.kt:445, 537` → full ID.
  - Desktop `members.rs:153`, `info.rs:363`, `dialog.rs:1062-1065`, `mention.rs:194`, `actions.rs:865` → localpart.
- iOS works out "the person of a direct chat" twice (`PresenceModel.swift:55-58`, `TimelineModel.swift:90`).

**Where they disagree**
iOS itself uses the localpart for senders and readers, but the full ID for members and banned people → **Q2**.

**Core: `core/people.rs`**
```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct User { pub id: OwnedUserId, pub name: Option<String>, pub avatar: Option<OwnedMxcUri> }
impl User {
    pub fn display_name(&self) -> String;   // trimmed name, else per Q2
    pub fn localpart(&self) -> &str;
}
```
- `Event.sender: User`, `Reply.sender: Option<User>`, `Reaction.by: Vec<User>`, `readers: Vec<User>`.
- `Member{user: User, role, level, membership, own, manage}`, `Banned{user: User, reason}`.
- `Contact.user`, `UserInfo.user`, `Notification.sender` (item 5).
- `RoomSummary.direct_user: Option<User>` (name and avatar from the heroes, as `direct_avatar` already reads).
- `user_profile`, `search_users` → `User`.
- `UserSummary`, `Reader` and `shown_name` go.

**FFI**
- One `User` record `{user_id, name, avatar_url, display_name}`. `display_name` is filled in when built, since records have no methods.
- It replaces `UserProfile`, `Reader` and the flat sender fields: `TimelineEvent.sender`, `ReplyQuote.sender`, `Member.user`, `Banned.user`, `Notification.sender`, `RoomSummary.direct_user`.

**Deleted**
- iOS: `displayName`, `localpart`, `senderLabel`, `Member.shownName`, `PresenceModel.person(of:)` / `TimelineModel.person` (they use `directUser`).
- Android: `displayName`, `personName`, `localpart`, `memberName`, the banned fallback, `personCard()` name logic.
- Desktop: the five localpart fallbacks.
- CLI: its own sender formatting.

---

## 5. One Notification type

**Today**
- Core has two:
  - `notifications.rs:40` `Notification` (in-process, from sync; desktop): room id/name, sender name, body, highlight, direct.
  - `push.rs:139` `PushNotification` (a push, fetched without sync; Android): adds event id, avatars, sender id, noisy, timestamp, `should_notify`.
- Each has its own `body()`:
  - `notifications.rs:249` gives the raw body (an image shows its file name).
  - `push.rs:242` names the kind ("Image", "Encrypted message", "Invited you").
- Sender names differ too: `member.name()` vs `sender_display_name`, then the localpart.
- FFI: `ffi/push.rs:42` `NotificationContent`, from `Mtrx.notification` (`ffi/client.rs:86`).
- Apps:
  - Desktop `app.rs:1864-1899, 5294-5328` (title "sender in room", body cut at 200).
  - Android `android/push/PushHandler.kt:69-101`, `MessageNotifications.kt:52-185`; it checks `shouldNotify` and ignores `isMention` and `roomAvatarUrl`.
  - iOS has no notifications (push on hold).

**Where they disagree**
The body of a file message: the desktop shows its file name, Android shows "Image" → **Q9**.

**Core: `core/notifications.rs`**
```rust
pub struct Notification {
    pub room_id: OwnedRoomId, pub event_id: OwnedEventId,
    pub room_name: String, pub room_avatar: Option<OwnedMxcUri>, pub direct: bool,
    pub sender: User, pub body: String,            // one body(): the push wording
    pub highlight: bool, pub noisy: bool, pub timestamp: SystemTime,
}
impl Session { pub fn notifications(&self) -> impl Stream<Item = Notification>; }   // in process
impl Notification { pub async fn fetch(profile, room, event) -> Result<Option<Notification>>; } // push
```
- `should_notify` goes: core returns `None` whenever it shouldn't notify, including our own event when the rules can't be read.
- `PushNotification` goes.

**FFI**
- `Notification` record replaces `NotificationContent`. `Mtrx.notification` returns it.

**Deleted**
- Android: the `shouldNotify` check (`PushHandler.kt`).
- Desktop: nothing beyond field renames (`sender.display_name`).

---

## 6. Live RoomSettings stream

**Today**
- Core `room_settings.rs:178` is a one-shot read.
- Changes return only once sync has brought them back:
  - `synced.rs` (`send_state`, `until`, `CONFIRM` 10 s) is used by `set_room_access`, `set_address`, `set_history`, `set_permissions`, `let_everyone_edit`, `set_suggested`, `remove_from_space`, `add_to_space` (`rooms.rs:532`), `unban` (`room_settings.rs:397-409`).
  - `set_notify_mode` (`:285-321`) has its own confirm loop.
- Apps read the settings again after each change:
  - iOS `RoomInfoModel.change` (`ios/Models/RoomInfoModel.swift:187-198`): shows the change at once, then reloads. Rename and address aren't shown at once. Pull to refresh (`RoomInfoView.swift:31`). One model per screen, nothing shared.
  - Android `RoomInfoModel.change` (`android/rooms/RoomInfoModel.kt:114-127`): "Saving…", then reloads. Nothing is shown before the server answers.
  - Desktop `load_settings` (`app.rs:5372`) after every `Saved` (`:1496`), `reload_card` (`:4054-4061, 2958, 4009`), overlays in `info.rs:237-250, 271-276, 1115-1129`. The card doesn't follow room-list changes.
  - Desktop Settings has its own "who may edit" section (`app/settings.rs:250-327`), although room settings now live only in the info panel (decision 2026-11-01).

**Where they disagree** (iOS wins)
- Shown at once: iOS for most settings, the desktop for notify/access, Android for none. Core echoes all of them, name and address too.

**Core: `core/room_settings.rs` on `RoomHandle` (item 7)**
```rust
pub struct RoomSettings {
    pub rights: Rights,
    pub notify: NotifyMode,
    pub join_rule: JoinRule, pub join_rule_choices: Vec<JoinRule>,
    pub address: Option<OwnedRoomAliasId>,
    pub listed: Option<bool>,
    pub history: History, pub permissions: Permissions,
    pub encrypted: bool, pub version: Option<String>,
    pub spaces: Vec<SpaceLink>,          // SpaceLink{space, suggested, can_change}
    pub can_add_to: Vec<OwnedRoomId>,
    pub banned: Vec<Banned>,
}
impl RoomHandle { pub fn settings(&self) -> Live<RoomSettings>; }
```
- The stream follows:
  - the room's info (state: join rule, history, alias, levels, encryption);
  - member updates (bans);
  - `notification_settings` changes;
  - the session's `SpaceGraph` (item 1);
  - settings echoes.
  - `listed` isn't room state. It's read once when the stream starts and again after `set_listed`.
- Echoes (`echo.rs`, step 2 pattern):
  - New `Pending` entries for join rule, history, address, listed, permissions, notify, encrypted, name, suggested, space add/remove and unban.
  - Each is recorded before the request, dropped on failure, when synced, or after `ECHO_TIMEOUT`.
- Setters return once the server took the request.
  - The notify confirm loop stays, inside core: changes to one room's mode go one after another, so a second change starts from the server's rules. Callers don't wait for it.
- `synced.rs` goes.

**FFI**
- `Room.subscribe_settings(listener) -> TaskHandle` (item 7).
- `Session.room_settings` goes.

**Deleted**
- Core: `synced.rs`, the waits in `unban` and `set_notify_mode` (moved into the internal queue), `Session::room_settings`.
- iOS: `RoomInfoModel.change`'s overlay and reload, `loadSettings`, pull to refresh for settings, `saving` (unused).
- Android: the `change()` reload, `_settings` one-shot loads (`RoomInfoModel.kt:45-68, 114-127`).
- Desktop: `load_settings`, `reload_card`, `info.rs` overlays and `apply()`, the `saving` flag, and `settings.rs:250-327` (`Access` section) with its `let_everyone_edit_space` button moved to the space's info panel.

---

## 7. RoomHandle in core, Room object in FFI

**Today**
- About 40 `Session` methods take a room ID (`calls.rs`, `categories.rs:363`, `directory.rs:203`, `pins.rs`, `members.rs`, `permissions.rs`, `room_settings.rs`, `rooms.rs:456-530`).
- Each one starts with `self.room(id)?` (`rooms.rs:570`).
- Usage: the desktop calls about 43 of them, iOS about 24, Android about 30, all with a room ID string.

**Conflict with Q5:** none, if the Room object only takes the room-ID calls that are on `Session` now.
- Rows, seek, sending, reacting and so on stay on `Timeline`.
- The draft stays on `DraftKeeper`.
- Their constructors stay too (`Session.timeline(room_id)`, `Session.pinned_timeline`, `Session.draft_keeper(room_id)`) → **Q6**.

**Core: `core/room.rs`**
```rust
#[derive(Clone)]
pub struct RoomHandle { session: Arc<Session>, room: matrix_sdk::Room }   // Q5
impl Session {
    pub fn room(self: &Arc<Self>, id: &RoomId) -> Result<RoomHandle>;       // known room (joined or invited)
}
impl RoomHandle {
    pub fn id(&self) -> &RoomId;
    pub fn settings(&self) -> Live<RoomSettings>;                          // item 6
    pub async fn members(&self) -> Result<Vec<Member>>;
    // state
    pub async fn set_name(&self, name: &str) -> Result<()>;  pub async fn set_topic(&self, topic: &str) -> Result<()>;
    pub async fn set_picture(&self, path: &Path) -> Result<()>;  pub async fn remove_picture(&self) -> Result<()>;
    pub async fn set_join_rule(&self, rule: JoinRule) -> Result<()>;  pub async fn set_history(&self, h: History) -> Result<()>;
    pub async fn set_address(&self, name: Option<&str>) -> Result<()>;  pub async fn set_listed(&self, on: bool) -> Result<()>;
    pub async fn set_permissions(&self, p: Permissions) -> Result<()>;  pub async fn encrypt(&self) -> Result<()>;
    pub async fn set_notify(&self, mode: NotifyMode) -> Result<()>;   pub async fn set_tag(&self, tag: RoomTag) -> Result<()>;
    pub async fn set_pinned(&self, event: &EventId, pin: bool) -> Result<()>;
    // people
    pub async fn invite(&self, user: &UserId) -> Result<()>;  pub async fn remove(&self, user: &UserId, ban: bool, reason: Option<&str>) -> Result<()>;
    pub async fn unban(&self, user: &UserId) -> Result<()>;   pub async fn set_level(&self, user: &UserId, level: i64) -> Result<()>;
    pub async fn leave(&self) -> Result<()>;
    // spaces (on the space's handle)
    pub async fn add_child(&self, room: &RoomId) -> Result<()>;  pub async fn remove_child(&self, room: &RoomId) -> Result<()>;
    pub async fn set_suggested(&self, room: &RoomId, on: bool) -> Result<()>;
    pub async fn let_everyone_edit_all(&self) -> Result<SpaceUpdate>;
    // calls
    pub async fn call_service(&self) -> Result<Option<CallService>>;       // own, else nearest ancestor's
    pub async fn set_call_service(&self, s: Option<CallService>) -> Result<()>;
    pub async fn post_call(&self, service: &str, url: &Url) -> Result<()>;
}
```
- These stay on `Session`: `join`/`knock` (by address, before there's a room), `create_room`, `direct_chat`, `timeline`, `pinned_timeline`, `draft_keeper`, `space_children` (server listing), `room_lists`.
- The internal `Session::room` becomes `sdk_room`.

**FFI**
```rust
#[derive(uniffi::Object)] pub struct Room { inner: InRuntime<RoomHandle> }
#[uniffi::export] impl Session { pub fn room(&self, room_id: String) -> Result<Arc<Room>, MtrxError>; }
#[uniffi::export] impl Room { /* the methods above with String IDs */ pub fn subscribe_settings(&self, l: Arc<dyn RoomSettingsListener>) -> Arc<TaskHandle>; }
```
- iOS: `RoomInfoModel` holds a `Room` and one settings `Subscriptions` entry.
- Android: `RoomInfoModel` holds a `Room` and a `TaskHandle`.
- The desktop's `Room` and the info card hold a `RoomHandle`.

**Deleted**
- The room-ID forms on `Session` in core and FFI: about 35 methods.

---

## 8. Live<T> and AbortOnDrop everywhere

**Today**
"A value now, then changes, kept while alive" is written five ways:
- `rooms.rs:99-135` `RoomList`: watch, `JoinHandle`, its own `Drop`.
- `timeline.rs:375-385` `RoomTimeline`: two `JoinHandle`s, its own `Drop`.
- `encryption.rs:92-97` `Verification`: its own `Drop`.
- `presence.rs:609` `PresenceWatcher`, `connection.rs:40`, `scheduled.rs:370`, `drafts.rs:202`: `session::AbortOnDrop` (`session.rs:81`, `pub(crate)`).
- `account_data.rs:84` `Synced::updates`: watch.
- `lists.rs:640` `room_lists`: an `unfold` stream that runs only while polled.

The FFI has its own generic `AbortOnDrop<T>` (`ffi/runtime.rs:37`). Each `subscribe_*` writes the same spawn-and-forward loop: `session.rs:88`, `timeline.rs:70, 183`, `encryption.rs:230, 279, 357`, `presence.rs:212`, `room_list.rs:527`.

**Core: `core/live.rs`**
```rust
pub struct AbortOnDrop<T = ()>(JoinHandle<T>);            // pub; FFI uses it too

/// A value kept up to date by a task, while any clone is alive.
#[derive(Clone)]
pub struct Live<T> { value: watch::Receiver<T>, _task: Option<Arc<AbortOnDrop>> }
impl<T: Clone + Send + Sync + 'static> Live<T> {
    pub fn get(&self) -> T;
    pub fn updates(&self) -> impl Stream<Item = T> + use<T>;   // now, then each change; ends with the task
    pub async fn wait_for(&self, done: impl Fn(&T) -> bool) -> T;
    pub(crate) fn spawn<F>(initial: T, run: impl FnOnce(watch::Sender<T>) -> F) -> Self where F: Future<Output = ()> + Send + 'static;
    pub(crate) fn from_watch(rx: watch::Receiver<T>) -> Self;
}
```
- State streams become `Live`:
  - `RoomList` → `Live<RoomListState>`, and `room_lists` → `Live<Arc<RoomLists>>`, so it runs once however many read it.
  - `connection_updates`, `room_categories_updates`, `room_list_colours_updates`, `directory_servers_updates`, `rooms_with_drafts_updates`, `scheduled_updates`, `encryption_states`.
  - `RoomTimeline::rows`, `typing_updates`, `uploads`.
  - `Verification::states`, `RoomHandle::settings`.
- Event streams stay `Stream`: `notifications`, `verification_requests`, `PresenceWatcher::updates`.
- Every spawned task is held in an `AbortOnDrop`. The `JoinHandle` + `Drop` pairs go.

**FFI**
- One helper `fn follow<T, R>(live: Live<T>, map: fn(T) -> R, on_update: impl Fn(R)) -> Arc<TaskHandle>` used by every `subscribe_*`.
- `TaskHandle` stays as the apps' handle (Q10). The apps' `Feed`/`Subscriptions` (iOS) and listener + `TaskHandle` pattern (Android) are unchanged.

**Deleted**
- Core: `RoomList`, `RoomTimeline` and `Verification` `Drop` impls, `session::AbortOnDrop`, `lists.rs` `unfold`.
- FFI: `runtime::AbortOnDrop`, eight copies of the forward loop.
- Desktop: the `Shared(...)` stream wrappers in `app.rs:4244-4380` become `live.updates()`.
- The apps' own fire-and-forget tasks (iOS about 67 `Task {}`, desktop `Task::perform`) are step 4.

---

## 9. google.rs into its own crate

**Today**
- `core/google.rs` (797 lines) is Google sign-in (OAuth with PKCE, a local redirect server from `matrix_sdk::utils::local_server`) and the Meet API.
- It uses core's `Secret` (`secrets.rs:62`, `pub(crate)`) and `Error::{Google, GoogleNotSignedIn, GoogleSignInExpired}` (`error.rs:82-87`).
- Used by:
  - Desktop: `app.rs:256, 778, 2980, 3114-3175, 5344-5355`, `settings.rs:179-247, 530-570, 795-860`.
  - CLI: `cli/google.rs:28-59`, `cli/main.rs:168, 535`.
- Not used by FFI, iOS or Android.

**New crate `crates/mtrx-google`**
- Moves in: `Google`, `GoogleClient`, its own `Error` (`NotSignedIn`, `SignInExpired`, `Other`), and its tests.
- Dependencies: `mtrx-core` (for `Secret`, made `pub`, → **Q11**), plus `matrix-sdk` (local server), `reqwest`, `sha2`, `base64`, `serde`, `url`, `tokio`, all already in the workspace. No new outside dependencies.
- `CallService::google_meet()` and `is_google_meet` stay in core: they only recognise links.
- Desktop and CLI depend on `mtrx-google`. `new_call_link` maps its error.

**Deleted**
- Core: `google.rs`, the three `Error` variants, the `pub use`.

---

## 10. Drop org.mtrx.call_link

**Today**
- Core:
  - `calls.rs:170-197` `CallLinkEventContent` and `call_link_change`.
  - `calls.rs:203-232` `move_call_link_to_topic`, `calls.rs:235-243` `set_old_call_link` (test-support).
  - `timeline.rs:182, 247-249, 1252` `EventContent::CallLink`.
  - `rows.rs:233, 247-250` its summary line, and the test at `rows.rs:539`.
  - `tests/integration.rs:577-632` (the old-link part).
- FFI: `types.rs:563-565` maps it to `OtherState`.
- Desktop:
  - `app.rs:3400-3407` migrates on every room open (one request per open).
  - `room.rs:1601`.
- CLI: `timeline.rs:70-73`.
- iOS and Android never migrated.

**Change**
- Delete all of the above.
- Old `org.mtrx.call_link` events become unknown state and are hidden, as Android already does.
- A room whose link was never moved loses it. The desktop has moved those in every room it opened since 0.4.0-beta.3 (2026-10-27).

---

## PR split

As in step 2, each PR has four commits, and each commit builds:
1. Core and FFI: new types, with tests, next to the old ones.
2. Desktop switches (and CLI).
3. iOS switches.
4. Android switches, and the old FFI is removed.

Where a platform has nothing to do, its commit is left out.

| PR | Items | Size | Depends on |
|---|---|---|---|
| G | 10 call_link, 9 google crate | S | — |
| H | 8 `Live<T>`, `AbortOnDrop` | M | — |
| I | 7 `RoomHandle` / `Room` | L | H |
| J | 4 `User` | M | — |
| K | 2 `JoinRule`, `Membership`, `JoinAction` | M | I |
| L | 1 `SpaceGraph` | M | H |
| M | 3 `Rights` | M | I, L (`can_add_to`) |
| N | 6 live `RoomSettings`, settings echoes, drop `synced.rs` | L | H, I, K, L, M |
| O | 5 `Notification` | S | J |

Order:
- G first (small, desktop and CLI only).
- Then H, then I: the handle goes in early, so the apps change their call sites only once and later PRs change the types behind them.
- Then J, K, L, M, N. O any time after J.
- J, L and O can run in parallel with I/K.
- One release after step 3 (decided 2026-11-02).

Rough size of the apps' changes:
- I is the widest: about 35 call sites per app, mechanical.
- N deletes the most app code: about 150 lines per app.

---

## Open questions

1. **Where `Rights` lives.**
   - (a) `RoomSummary.rights` (live via the room list), with `RoomSettings.rights` the same value.
   - (b) Only in `RoomSettings`, so the header and lists open a settings stream to know a right.
   - (c) `RoomHandle::rights()`, one-shot.
   - Recommend (a): no extra loads, and lists can check "invite to space" and "new room in space".

2. **Display name fallback for `User`** (iOS is itself split: senders use the localpart, members the full ID).
   - (a) Localpart everywhere. Member and person rows keep showing the full ID as their second line.
   - (b) Full ID everywhere.
   - (c) Keep the split, as two methods.
   - Recommend (a).

3. **A space's rooms as listed by the server.**
   - (a) Core caches 60 s and merges in joined children the server leaves out (the desktop's behaviour).
   - (b) Server's list only, no cache (iOS footer stays).
   - (c) Server's list only, with the 60 s cache.
   - Recommend (a). The iOS gap is a limitation, not a choice.

4. **Actions the server would refuse** (invite to a space from the person sheet; "New room in space"; invite from the desktop sidebar).
   - (a) Offer them only with the right (`rights.invite`, `rights.add_rooms`) on all three.
   - (b) Keep iOS: always offer, and show the server's error.
   - Recommend (a). It's a bug, so the iOS rule doesn't bind.

5. **What `RoomHandle` holds.**
   - (a) `Arc<Session>` (`Session::room(self: &Arc<Self>, ..)`). The desktop and FFI already hold `Arc<Session>`.
   - (b) Split `Session` into an `Arc<Shared>` (client, echoes, notification settings, send lock) that handles clone.
   - (c) Borrow `&Session` (no good across tasks).
   - Recommend (a).

6. **Constructors of `Timeline` and `DraftKeeper` under Q5.**
   - (a) Leave them on `Session` (`timeline(room_id)`, `pinned_timeline`, `draft_keeper`). `Room` only takes the other room-ID calls.
   - (b) Also offer `room.timeline()` / `room.draft_keeper()` (constructors only; methods stay put).
   - Recommend (a): no churn, Q5 literally.

7. **English labels for settings in core** (extending step-2 Q6): titles and fact lines for `JoinRule`, `History`, `NotifyMode`, `Permissions`, and `JoinAction` titles.
   - (a) Yes, in iOS wording. It deletes about 3 label switches per app.
   - (b) No, the apps keep their labels.
   - Recommend (a).

8. **`RoomSummary` fields replaced by the new types** (`parents`, `invited`, `public`).
   - (a) Replace them with graph queries on `RoomLists`, `membership` and `join_rule`.
   - (b) Keep them as derived copies next to the new fields.
   - Recommend (a). One source each, and the FFI break is in this step anyway.

9. **Notification body of a file message.**
   - (a) The push wording everywhere ("Image", "File"…). The desktop stops showing the file name.
   - (b) The raw body (file name) everywhere.
   - Recommend (a): one `body()`, and it says what kind of message it is.

10. **`Live<T>` across FFI.**
    - (a) Keep listener + `TaskHandle` per subscription, written once by a generic `follow` helper.
    - (b) One FFI object per live type, with `get()` and `subscribe()`.
    - Recommend (a). The apps' `Feed`/`Subscriptions` stay as they are.

11. **Secrets for `mtrx-google`.**
    - (a) Make core's `Secret` public and depend on `mtrx-core`.
    - (b) Move `secrets.rs` into its own `mtrx-secrets` crate used by both.
    - Recommend (a): smaller. (b) only if `mtrx-google` should not pull in `mtrx-core`.

---

## Decisions (user, 2026-11-02)

All recommendations taken: 1a, 2a, 3a, 4a, 5a, 6a, 7a, 8a, 9a, 10a, 11a.
