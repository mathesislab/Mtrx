# Quiet: pause, quiet hours, and muting a room for a time

Goal: two ways to stop notifications for a while.
1. **Pause** all notifications: for 1 hour, until tomorrow morning, until a time picked, or until turned back on. And **quiet hours**: from–to, on chosen weekdays. On all the user's devices with Mtrx. Pause is pause: mentions too, unless a setting lets mentions and keywords through. Unread counts keep counting.
2. **Mute a room for a time**: 1 hour, 8 hours, until tomorrow, or until unmuted. With the room's push rule, so every Matrix app sees it muted. Afterwards the room gets back the mode it had (All messages or Mentions).

Rules from steps 2–3 that apply:
- Shared state lives in core and reaches the apps as finished state (`Live<T>` in core; listener + `TaskHandle` via `follow` in FFI).
- Apps keep no parallel copies.
- Core returns English text. FFI labels and derived values are record fields, not functions.

Paths: `core/` = `crates/mtrx-core/src/`, `ffi/` = `crates/mtrx-ffi/src/`, `app/` = `crates/mtrx-app/src/`, `ios/` = `apps/ios/Mtrx/`, `android/` = `apps/android/app/src/main/kotlin/org/mtrx/app/`, `sdk/`, `sdk-base/`, `sdk-ui/` = `~/.cargo/registry/src/index.crates.io-*/matrix-sdk{,-base,-ui}-0.19.1/src/`, `ruma/` = `…/ruma-common-0.20.1/src/`.

---

## What Matrix offers

- Push rules decide what notifies: on the server (pushes) and in each client. The SDK evaluates them for every synced event (`sdk-base/response_processors/timeline.rs:136-145`).
- **`.m.rule.master`**: one override rule that, when enabled, stops every notification.
  - It is always the first rule (`ruma/push/predefined.rs:113-118`, `ruma/push.rs:146`). No rule of ours can come before it, so it can't let mentions through.
  - The SDK can switch it: `NotificationSettings::set_push_rule_enabled(Override, ".m.rule.master", on)` (`sdk/notification_settings/mod.rs:171-188`).
  - Element shows it as "Enable notifications for this account".
- **No timed mute.** A room is muted by an override rule of its own with no actions. Nothing in the spec ends it, and no MSC proposes an end time.
- Proposals that come close:
  - **MSC4359 "Do not Disturb"** (open, needs an implementation): global account data `dm.filament.do_not_disturb` = `{"rooms": {"*": {}}}`, or per room. Clients hold back pop-ups and sounds; counts go on; pushes still arrive. No end time, no schedule, no mentions through. ruma 0.35.1 has the type behind `unstable-msc4359`. Neither Element nor matrix-sdk acts on it.
  - **MSC3767**: a `time_and_day` push condition, for quiet hours on the server. A draft. ruma 0.20 can't evaluate it, so the SDK's local counts couldn't either.
  - **MSC3890**: "silence this session" as per-device account data (Element Web). Per device, no end time.
  - **MSC3881**: switch a pusher off without deleting it. Synapse has it only behind an experimental flag.
- So the pause is ours: account data, like MSC4359 but with an end, a schedule and mentions. A timed mute is a push rule plus a timer.

## Today

**Core: how a notification is decided**
- Desktop path, from sync (`core/notifications.rs`):
  - An event handler sees every synced timeline event with its push actions (`:100-105`).
  - `triage` (`:218-233`) ignores our own events and those sent before the session started. Encrypted ones go to `Decrypt`. Others notify if `should_notify(actions)` (`:235-237`).
  - `Decrypt` is skipped when the room's own mode is Mute (`muted`, `:129`, `:189-194`). Otherwise the SDK's `NotificationClient` decrypts it, and the decrypted event's actions decide (`:140-173`).
  - `describe` (`:240-273`) builds the `Notification`, with `highlight` (a mention or keyword) and `noisy` from the actions. Each event notifies once (`Seen`, `:317-352`).
  - `Session::notifications()` (`:354-359`) is the stream. Nothing in it knows of a pause.
- Android path, per push (`core/push.rs`):
  - The pusher sends only IDs (`event_id_only`, `:54-67`).
  - `Notification::fetch` (`:149-178`) restores a client from the profile, without syncing. The SDK's `NotificationClient` runs a short sliding sync for the event with the account-data extension on (`sdk-ui/notification_client.rs:677-683`). So the store's account data is current once the event is fetched.
  - `pushed` (`:182-216`) returns `None` for our own events and those the rules don't notify for. Unknown actions count as notify.
- Room modes (`core/room_settings.rs`):
  - `NotifyMode {All, Mentions, Mute}` (`:43-93`), with titles and explanations.
  - `notify_mode` (`:251-269`): the room's own mode, else the account's default for rooms like it. `RoomSettings.notify` doesn't say which of the two.
  - `set_notify` (`:431-436`) echoes the change; `send_notify` (`:447-480`) makes it in the room's turn (`NotifyTurns`, `:198-212`) and waits for sync to confirm it.
  - The SDK's `set_room_notification_mode` (`sdk/notification_settings/mod.rs:305-355`): Mute is an override rule, and setting it deletes the room's other rules. **So the mode before a mute is lost from the rules.** `delete_user_defined_room_rules` (`:358-380`) makes a room follow the default again.
- Unread counts come from the same push actions, on the device (`sdk/event_cache/caches/read_receipts.rs:205-228`): `unread_notifications` and `unread_mentions` (`core/rooms.rs:214-216, 373-375`). `unread_messages` counts every message. A muted room has no notification count. With `.m.rule.master` on, no room would.
- Account data: `Synced<T>` (`core/account_data.rs:41-103`), global only. Disappearing messages keep a per-room map by hand (`core/disappearing.rs:342-345, 356-375, 842-856`).
- Timers that every app runs: `start_disappearing()` (`core/disappearing.rs:677-701, 804-812`). It wakes at the next due time, at most every 30 s. The desktop starts it after sync (`app/app/start.rs:150-151`), the FFI in `start_sync` (`ffi/session.rs:56-73`), and `mtrx watch` too.
- Times: `core/send_time.rs` reads and describes local times with chrono's `Local` (`:45-57`). A day without a time means 09:00 (`:8`). `local` takes the earlier of two times when clocks go back (`:104-107`).
- Presence is automatic: online in front, idle after 5 minutes, offline in the background (`core/presence.rs:349-360, 396-405`). There is no manual "away". "Share my presence" is a device setting in each app (`app/settings.rs:529`).

**Desktop**
- Notifications: one subscription (`app/app.rs:1055-1061`). `app/app/account.rs:189-201` shows each one, unless "Desktop notifications" is off (a device setting, `app/settings.rs:556-562`) or the window is focused on that room.
- Dock / taskbar count: the sum of `unread_notifications` (`app/badge.rs:24-32`).
- Room list: bold when unread; a count, red for mentions (`app/sidebar/mod.rs:1524-1545`). Nothing marks a muted room.
- Space bar, bottom: downloads (while there are any), then the gear (`app/sidebar/mod.rs:297-325`).
- Room card: "Notifications" is a pick list of the three modes, with the explanation (`app/info.rs:858-871`, `:411-414`, `Notify` `:1486-1495`).

**iOS**
- No notifications at all: no push, no local ones. The app syncs only in front.
- Room info: a `Picker` of `notifyChoices` (`ios/Views/RoomInfoView.swift:239-243`).
- Settings: Account, Room list, Privacy, Encryption (`ios/Views/SettingsView.swift:14-29`). No Notifications section.

**Android**
- Every notification comes through a push, also in front (`android/push/PushHandler.kt:69-97`). A fetch that fails or times out posts a fallback for the room (`:99-101`).
- Settings → Notifications: push on/off and the distributor (`android/settings/NotificationSettings.kt:89`, `SettingsContent.kt:70-71`).
- Room info: a `ChoiceRow` of `notifyChoices` (`android/rooms/RoomInfo.kt:360-367`).

**FFI**
- `Mtrx.notification(room_id, event_id)` → `Notification::fetch` (`ffi/client.rs:84-91`, `ffi/push.rs:132-144`).
- `NotifyChoice {mode, title, explanation}` and `RoomSettings.notify_choices` (`ffi/manage.rs:103-107, 241-243, 275-280`). `Room.set_notify` (`ffi/room.rs:119-122`).

## Where they disagree

| | Desktop | Android | iOS |
|---|---|---|---|
| Who decides | core, from sync | core, per push | — |
| Off on this device | "Desktop notifications" | system settings, or push off | — |
| A room's mode | card pick list | room info row | room info picker |
| Muted room in the list | not shown | not shown | not shown |
| Pause, quiet hours | no | no | no |

Core already decides on both paths. A pause decided in core covers both.

---

## What it can't do (said in the app)

- **The pause is Mtrx only.** Element and other apps on the account keep notifying.
- **The server still pushes during a pause.** An Android phone still wakes for each push, fetches it and shows nothing: the same battery and data as now.
- **iPhone.** It shows no notifications yet, paused or not. Its Settings set the pause for the other devices. Once iOS gets pushes, the same core decision applies there.
- **A timed mute ends only while Mtrx runs.** The room is muted in every Matrix app, since it's a push rule. When the time passes, a running Mtrx (the desktop, `mtrx watch`, a phone in front) unmutes it. With none running, it stays muted, in Element too, until one starts. Then it's unmuted at once.
- **Another app's Mute during a timed mute.** Mtrx still unmutes the room when the time comes. Matrix can't tell the two apart.

## Global pause and quiet hours

### How (Q1)

| | (a) account data, decided in core | (b) `.m.rule.master` | (c) both: (a), plus master for "until I turn them back on" |
|---|---|---|---|
| Mentions and keywords through | yes | no (master is always first) | only when not indefinite |
| Unread counts | keep counting | room counts and dock badge stop, and don't catch up | stop while master is on |
| Ends on time, quiet hours start and end, with no app running | yes: each device compares times | no: an app must switch the rule | partly |
| Element and others quiet too | no | yes | sometimes |
| Android pushes while paused | still arrive, shown as nothing | none | none while master is on |
| Work | a check in two places | a timer that flips the rule; it fights Element's switch | both |

(b) breaks two of the decided rules (mentions through, counts). (a) is recommended.

### Global account data `org.mtrx.quiet`

```json
{
  "pause": { "until": 1763539200000 },
  "hours": { "on": true, "from": "22:00", "to": "07:00", "days": ["mon", "tue", "wed", "thu", "fri"] },
  "mentions_through": false
}
```

- `pause`: absent = not paused; `{}` = until turned back on; `{"until": ms}` = until then.
  - An instant (UTC), so every device ends it at the same moment.
  - An ended pause stays in the data, in the past. Nothing has to clear it, so no app needs to run.
- `hours`: wall-clock times without a zone. Each device applies them in its own zone (Q3).
  - `on: false` keeps the times for next time.
  - `days`: the days a quiet period starts on (Q4).
- `mentions_through`: mentions and keywords (push `highlight`) notify during both (Q5).
- Unknown fields are kept when it's rewritten, so an older Mtrx doesn't drop a newer one's.

### Core: new `core/quiet.rs`

```rust
/// When notifications are held back, shared by the user's devices
/// (global account data `org.mtrx.quiet`, in `Synced<T>`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Quiet {
    pub pause: Option<Pause>,
    pub hours: QuietHours,
    /// Mentions and keywords notify during a pause and quiet hours.
    pub mentions_through: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pause { Until(SystemTime), UntilResumed }

/// Every chosen day, in each device's local time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuietHours {
    pub on: bool,
    pub from: NaiveTime,              // default 22:00
    pub to: NaiveTime,                // default 07:00; `from == to` is the whole day
    pub days: BTreeSet<Weekday>,      // the days a period starts on; default all
}

/// What the pause menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseFor { Hour, TomorrowMorning, At(SystemTime), UntilResumed }

/// An entry of the pause menu: a pause, or "Until…" (the app asks for a time,
/// then pauses with `PauseFor::At`).
pub enum PauseEntry { Pause(PauseFor), PickTime }

/// "For 1 hour", "Until tomorrow, 08:00" (or "Until 08:00" before 05:00),
/// "Until…", "Until I turn them back on".
pub fn pause_menu(now: SystemTime) -> Vec<(PauseEntry, String)>;

/// 08:00 local: today's before 05:00, else tomorrow's (Q2).
pub const MORNING: NaiveTime = /* 08:00 */;
pub(crate) fn tomorrow_morning(now: SystemTime) -> SystemTime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuietStatus {
    On,
    Paused { until: Option<SystemTime> },
    QuietHours { until: SystemTime },
}

/// What the apps show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuietState {
    pub quiet: Quiet,
    pub status: QuietStatus,
    /// "Notifications on", "Paused until 08:00", "Paused until tomorrow 08:00",
    /// "Paused until you turn them back on", "Quiet hours until 07:00".
    pub line: String,
    /// "22:00–07:00 on weekdays", "All day on Sat, Sun", "Off".
    pub hours_line: String,
}

impl Quiet {
    /// Whether a notification is held back at `now`; `highlight`: it
    /// mentions us or a keyword.
    pub fn silences(&self, highlight: bool, now: SystemTime) -> bool;
    pub fn status(&self, now: SystemTime) -> QuietStatus;
    /// When `status` may change next: the pause's end, quiet hours' next
    /// start or end.
    pub(crate) fn next_change(&self, now: SystemTime) -> Option<SystemTime>;
}

impl Session {
    pub fn quiet(&self) -> QuietState;
    /// Then every change: from our devices, and at each start and end.
    pub fn quiet_updates(&self) -> Live<QuietState>;
    pub async fn pause_notifications(&self, pause: PauseFor) -> Result<()>;
    pub async fn resume_notifications(&self) -> Result<()>;
    pub async fn set_quiet_hours(&self, hours: QuietHours) -> Result<()>;
    pub async fn set_mentions_through(&self, on: bool) -> Result<()>;
}
```

- `Session` gains `quiet: Synced<Quiet>`. A change shows at once and is rolled back if saving fails, as for every `Synced`.
- **Desktop path:** `notifications::register` (`core/notifications.rs:86`) takes a `watch::Receiver<Quiet>`.
  - In `Notifier::synced`, after `claim` and before `send`: `quiet.silences(highlight, now)`. A silenced event stays claimed, so it doesn't pop up when the pause ends.
  - `Decrypt`: while quiet without `mentions_through`, nothing is decrypted, as for a muted room (`:129`). With it, the decrypted event's highlight decides.
- **Push path:** `Notification::fetch` reads `Quiet` from the store after `get_notification`, which has just synced the account data. `pushed(…, &quiet, now)` returns `None` when silenced.
  - If the fetch fails while the stored `Quiet` is quiet now, `fetch` returns `Ok(None)`. Android then posts no fallback during a pause.
  - The fetch is never skipped on the store's word: the store may be from before a resume on another device.
- **Which "now":** the time of deciding, not the message's time. A message that arrives late, after the pause ended, notifies.
- **After a pause:** nothing that came during it pops up. The counts show it.
- **`quiet_updates`:** a task re-reads `status` at `next_change`, at most every 30 s, like the timers. A changed clock, a new time zone, or a Mac waking from sleep shows within 30 s. (tokio's sleep runs on the monotonic clock, which stops while a Mac sleeps.)

### Time

- **A pause is an instant.**
  - "For 1 hour": now + 1 h.
  - "Until tomorrow morning": 08:00 local on the device choosing (Q2). Today's before 05:00, else tomorrow's. The title says which: "Until 08:00" or "Until tomorrow, 08:00".
  - "Until…": date and time in the device's zone. A time that doesn't exist (clocks go forward) moves to the first that does; one that exists twice takes the earlier (`send_time::local`).
  - Another device shows the same instant in its own zone. A pause until 08:00 set in New York reads "Paused until 14:00" on a laptop in Bremen.
- **Quiet hours are wall clock, per device.**
  - Compared on local date and time (`NaiveDateTime`), so daylight saving needs no special case. The night clocks go back is an hour longer.
  - A phone abroad is quiet at its own night. A desktop left at home is quiet at home's night.
- **Days:** a period belongs to the day it starts. 22:00–07:00 on Mon–Fri is quiet from Friday 22:00 to Saturday 07:00, and not on Sunday night.
- **Clocks:** each device uses its own. One set wrong ends a pause early or late there. The server's time isn't used.

## Timed room mute

### Room account data `org.mtrx.mute` (Q6)

```json
{ "until": 1763485200000, "before": "mentions" }
```

- `before`: `"all"`, `"mentions"`, or absent: the room had no rule of its own and followed the account's default.
- `{}`: no timed mute. Account data can't be deleted, so `{}` stands for none.
- Per room, so unmuting one room never overwrites a mute just set in another from a second device.
- "Mute until I unmute" writes no record. It's today's plain Mute.

### Core

New `RoomSynced<T>` in `core/account_data.rs`: `Synced<T>` per room. A map room → value, loaded from the joined rooms' account data, followed by an event handler, changed with rollback. Disappearing modes move onto it (`core/disappearing.rs:342-345, 356-375, 705-713, 842-856`). Timed mutes use it too.

```rust
/// How long a room stays muted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuteFor { Hour, EightHours, TomorrowMorning, UntilUnmuted }

/// A room muted for a time (room account data `org.mtrx.mute`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimedMute {
    pub until: SystemTime,
    /// The room's own mode before; `None`: it followed the default.
    pub before: Option<NotifyMode>,
}

/// An entry of a room's notifications menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifyAction { Mode(NotifyMode), Mute(MuteFor) }

pub struct NotifyChoice {
    pub action: NotifyAction,
    /// "All messages", "Mentions", "Mute for 1 hour", "Mute for 8 hours",
    /// "Mute until tomorrow, 08:00", "Mute until I unmute".
    pub title: String,
    pub current: bool,
}

pub struct RoomSettings {
    // as now, plus:
    pub muted_until: Option<SystemTime>,
    /// "All messages", "Mentions", "Muted", "Muted until 14:00".
    pub notify_title: String,
    /// "Every message notifies you.", "Muted until 14:00, then all messages again."
    pub notify_explanation: String,
    pub notify_choices: Vec<NotifyChoice>,
}

impl RoomHandle {
    /// Mute the room; for a time, it gets its mode back afterwards.
    pub async fn mute(&self, for_: MuteFor) -> Result<()>;
    /// As now; also ends a timed mute.
    pub async fn set_notify(&self, mode: NotifyMode) -> Result<()>;
}

impl Session {
    /// Delete disappearing messages and end timed mutes when due, until
    /// stopped or dropped. Replaces `start_disappearing`.
    pub fn start_timers(&self);
    pub fn stop_timers(&self);
}

pub struct RoomLists {
    // as now, plus:
    /// Muted rooms, with the end of a timed mute (Q9).
    pub muted: BTreeMap<OwnedRoomId, Option<SystemTime>>,
}
```

**Muting**, in the room's turn (`NotifyTurns`):
1. `before`: the room's own mode (`get_user_defined_room_notification_mode`). If it's already timed-muted, the record's `before`: a new end keeps the original mode.
2. Write the record, then set Mute (`send_notify`). If Mute fails, write `{}` back. Record first: if the record fails, nothing changed. The other order could leave a room muted with no end.
3. `set_notify(mode)` sets the mode, then writes `{}` if there was a record.

**Choices offered** (`notify_choices`, from core):
- Not muted: All messages, Mentions, then the four mutes.
- Timed-muted: All messages and Mentions (end it early), the three timed ones (a new end), Mute until I unmute.
- Muted, plainly: All messages, Mentions, Mute (current). No timed ones: there is no mode to go back to.

**Words:** the explanation always names the mode it goes back to. With `before` absent, that's the account's default for rooms like it, read when shown: "Muted until 14:00, then mentions only."

**`RoomLists.muted`:** from `notification_settings` (rooms with a rule of their own that is Mute, followed through `subscribe_to_changes`), with the records' ends. A new input to `room_lists` (`core/lists.rs:667`), like the drafts.

### Unmuting: the timer

- Runs in `start_timers()`, beside the disappearing timer. Started by the desktop, `mtrx watch`, and FFI `start_sync`.
- Wakes at the next `until`, at most every 30 s, or when a record changes.
- The first pass waits for the first sync after start. It acts on current rules and records, not on the store's from before: another device may have changed them meanwhile.
- For each record whose `until` passed, in the room's turn:
  1. If the room's own mode is still Mute: set `before` (`set_room_notification_mode`), or delete the room's rules when `before` is absent.
  2. Write `{}`, but only if the record is still the one acted on. Another device may have set a new mute meanwhile.
  3. If the room isn't muted any more (changed elsewhere): only step 2.
  4. On failure the record stays. The next pass tries again.
- **Two apps at once** (desktop and phone): both may unmute. The second finds the rules already changed. Either the SDK skips it (`sdk/notification_settings/mod.rs:313`), or the server answers "not found" for the mute rule already deleted, which is logged. Its next pass sees the room unmuted and only clears the record. Both write the same `{}`. The end state is the same.
- Rooms left: ignored. Their records stay, harmless.
- The decision is a pure function, tested on its own: `fn unmute_step(mute: &TimedMute, own_mode: Option<NotifyMode>, now: SystemTime) -> Step`, with `Step::{Wait, Restore(Option<NotifyMode>), Clear}`.

## FFI

```rust
// ffi/quiet.rs
#[derive(uniffi::Enum)] pub enum Weekday { Mon, Tue, Wed, Thu, Fri, Sat, Sun }
#[derive(uniffi::Record)]
pub struct QuietHours { pub on: bool, pub from_minute: u16, pub to_minute: u16, pub days: Vec<Weekday> }  // minutes after midnight
#[derive(uniffi::Enum)] pub enum PauseFor { Hour, TomorrowMorning, At { at_ms: u64 }, UntilResumed }
#[derive(uniffi::Enum)] pub enum PauseEntry { Pause { pause: PauseFor }, PickTime }
#[derive(uniffi::Record)] pub struct PauseMenuItem { pub entry: PauseEntry, pub title: String }

#[derive(uniffi::Record)]
pub struct QuietState {
    pub quiet: bool,              // held back now (paused or quiet hours)
    pub paused: bool,
    pub until_ms: Option<u64>,
    pub line: String,             // core's line
    pub hours: QuietHours,
    pub hours_line: String,
    pub mentions_through: bool,
}

#[uniffi::export] impl Session {
    pub fn quiet(&self) -> QuietState;
    pub fn subscribe_quiet(&self, listener: Arc<dyn QuietListener>) -> Arc<TaskHandle>;   // via `follow`
    pub fn pause_menu(&self) -> Vec<PauseMenuItem>;                                        // titles as of now
    pub async fn pause_notifications(&self, pause: PauseFor) -> Result<(), MtrxError>;
    pub async fn resume_notifications(&self) -> Result<(), MtrxError>;
    pub async fn set_quiet_hours(&self, hours: QuietHours) -> Result<(), MtrxError>;
    pub async fn set_mentions_through(&self, on: bool) -> Result<(), MtrxError>;
}

// ffi/manage.rs, ffi/room.rs
#[derive(uniffi::Enum)] pub enum MuteFor { Hour, EightHours, TomorrowMorning, UntilUnmuted }
#[derive(uniffi::Enum)] pub enum NotifyAction { Mode { mode: NotifyMode }, Mute { duration: MuteFor } }
#[derive(uniffi::Record)] pub struct NotifyChoice { pub action: NotifyAction, pub title: String, pub current: bool }
// RoomSettings gains: muted_until_ms: Option<u64>, notify_title: String, notify_explanation: String
#[uniffi::export] impl Room { pub async fn mute(&self, duration: MuteFor) -> Result<(), MtrxError>; }
// The room summary record gains `muted: bool`, filled from `RoomLists.muted`.
```

- `start_sync` / `stop_sync` call `start_timers` / `stop_timers`: a phone ends timed mutes while in front.
- `Mtrx.notification` keeps its signature. While paused it returns `None`, also when the fetch fails.
- Titles are computed when sent, so "until tomorrow, 08:00" can go stale across 05:00, like the presence line. `pause_menu()` is called when the menu opens.

## Desktop

**The bell** (`app/sidebar/mod.rs:297-325`)
- New `icons::bell` and `icons::bell_slash`.
- Order at the bottom: downloads (while any), the bell, the gear. The bell sits right above the gear and doesn't move when downloads appear. Always shown.
- Notifications on: an outline bell, faint like the gear. Paused or in quiet hours: a bell-slash in the accent colour.
- Hint: core's `line`, e.g. "Paused until 08:00", "Quiet hours until 07:00".

**Popover** (`popover`, `Side::Above`, `Edge::Left`, about 300 wide, as downloads)
- Top: the line. While paused: "Resume".
- "Pause notifications": the entries of `pause_menu`. "Until…" opens send later's calendar with hour and minute wheels (`app/send_later.rs`), without its quick picks.
- "Quiet hours": a toggle; From and To with the same wheels; seven day buttons M T W T F S S; `hours_line` below.
- "Let mentions and keywords through": a toggle.
- Footer: "Mtrx on all your devices. Other Matrix apps aren't paused."
- Esc closes the time picker, then the popover.

**Structure**
- New `app/quiet.rs`: `enum Message {Toggle, Pause(PauseFor), PickTime, Resume, Hours(QuietHours), MentionsThrough(bool)}`, `fn update(&Session, Message) -> Task`, `fn view(&QuietState, open: bool) -> Element`.
- One subscription to `session.quiet_updates()`. The popup is `Popup::Quiet { picking: Option<…> }`. `space_bar` takes the bell's element, as it takes the downloads'.

**Room card** (`app/info.rs:858-871`)
- The pick list becomes a button with `notify_title` that opens a menu of `notify_choices` (`Popup::CardNotify`). `notify_explanation` below.
- `Message::Notify(NotifyMode)` becomes `Message::Notify(NotifyAction)`: `set_notify` or `mute`.

**Room list**
- A small faint bell-slash after the name of a muted room (Q9). Muted rooms already show no count.

**Unchanged**
- "Desktop notifications": this device, always off. The pause: all devices, for a time.
- The dock count keeps counting during a pause.

## iOS

- **Settings:** a new "Notifications" section with one row, "Pause notifications", showing `line`. It opens `QuietView`:
  - Pause: the `pauseMenu()` entries. "Until…" opens a sheet with a graphical `DatePicker`. While paused: the line and "Resume".
  - Quiet hours: a `Toggle`, From and To as `DatePicker(.hourAndMinute)`, seven day toggles, `hoursLine`.
  - `Toggle("Let mentions and keywords through")`.
  - Footer as on the desktop.
- **Chats tab:** a bell-slash in the top bar while quiet. A tap opens `QuietView`.
- **Room info:** "Notifications" becomes a `Menu` labelled `notifyTitle`, with `notifyChoices`; `notifyExplanation` below.
- **Room rows:** a bell-slash when `muted`.
- `QuietModel`, owned by `SessionModel` and injected like `presence`, holds the one subscription.

## Android

- **Settings → Notifications:** a row "Pause notifications" with `line`, opening `QuietScreen` with the same content. Material 3 `TimePicker` dialogs; "Until…" is a `DatePicker`, then a `TimePicker`.
- **Chats tab:** a bell-slash in the top bar while quiet. A tap opens `QuietScreen`.
- **Room info:** the notifications `ChoiceRow` becomes a menu labelled `notifyTitle`, with `notifyChoices`; `notifyExplanation` below.
- **Room rows:** a bell-slash when `muted`.
- **Pushes:** `PushHandler` doesn't change. Core returns `null` while paused, so neither a notification nor a fallback is posted.
- The launcher's dot comes from posted notifications, so it stays off during a pause. The app's tab badges keep counting.
- The quiet model holds one `TaskHandle` for `subscribeQuiet`.

## Tests

**Unit, `core/quiet.rs`**
- `silences`: not paused; a pause ended and not ended; until resumed; `mentions_through` with and without a highlight.
- Quiet hours, on naive local times:
  - inside and outside; `from` is quiet, `to` isn't;
  - overnight by start day: Friday night quiet, Sunday night not, for Mon–Fri;
  - `from == to` is the whole day; off is never quiet;
  - the night clocks change, as wall times.
- `next_change`: a pause's end; the next start and end, skipping days not chosen.
- `tomorrow_morning`: 04:59 → today 08:00; 05:00 → tomorrow 08:00.
- Lines: "Notifications on", "Paused until 08:00", "Paused until tomorrow 08:00", "Paused until you turn them back on", "Quiet hours until 07:00"; "22:00–07:00 on weekdays", "All day on Sat, Sun".
- Content round trip; unknown fields kept; damaged content reads as the default (not quiet).
- `Notification::pushed` with a `Quiet`: held back, and a mention let through.
- A silenced synced event stays claimed.

**Unit, timed mutes**
- `unmute_step`: not due → `Wait`; due and muted → `Restore(before)`; due and not muted → `Clear`; `before` absent → the rules deleted.
- `notify_choices`, titles and explanations for each state; the default named when `before` is absent.
- Record round trip; `{}` reads as none.
- `RoomSynced`: shows a change at once, takes synced values unless a change is unsaved, rolls back on failure. The disappearing-mode tests keep passing on it.

**Integration** (local Synapse, `test-support`)
- Pause: alice on two devices. Device A pauses. Bob's message doesn't notify on device B (`notifications()`). A mention does with `mentions_through`. After "Resume", a message notifies.
- Push path: `Notification::fetch` with B's profile returns `None` while A has paused, and `Some` after A resumes. The fetch's own sync brings the change.
- Timed mute:
  - A room set to Mentions, muted until a few seconds ahead (a crate-private `mute_until(at)`): the rules show Mute and the record is there. Once due, the timer sets Mentions again and clears the record.
  - A room following the default ends with no rule of its own.
  - A and B both running timers: the same end state, no error shown.
  - Overdue at start: a record in the past; a new session unmutes after its first sync.

---

## Deleted

- **Core:**
  - `start_disappearing` / `stop_disappearing`: now `start_timers` / `stop_timers`.
  - Disappearing's own per-room map: `modes`, `unsaved_modes`, `set_mode`, `stored_mode` (`core/disappearing.rs:342-345, 705-713, 842-856`). Now `RoomSynced<DisappearingMode>`.
- **FFI:** `NotifyChoice.mode` and `.explanation`. Now `action`, and `RoomSettings.notify_explanation`. Removed in the Android commit, once neither phone reads them.
- **Desktop:** the card's pick list and `info::Notify` (`app/info.rs:1486-1495`).
- **iOS:** the notifications `Picker` (`ios/Views/RoomInfoView.swift:239-243`).
- **Android:** the notifications `ChoiceRow` (`android/rooms/RoomInfo.kt:360-367`).
- **Not deleted:** `Notifier::muted` (it still skips decrypting in muted rooms); "Desktop notifications"; `NotifyMode` and `set_notify`.

## PR split

Two PRs, one concern each. Every commit builds (CI only).

**PR 1: timed room mute** (M)
1. Core: `RoomSynced<T>`; disappearing modes move onto it. No change in behaviour.
2. Core and FFI: `org.mtrx.mute`, `mute`, the choices and words, the unmuting timer, `start_timers`, `RoomLists.muted`. Unit and integration tests. The new FFI fields sit beside the old ones.
3. Desktop: the card's menu, the room list's bell-slash, the icons.
4. iOS: room info menu, room rows.
5. Android: room info menu, room rows. The old `NotifyChoice` fields go.

**PR 2: pause and quiet hours** (M–L)
1. Core and FFI: `quiet.rs`, the checks in both paths, `quiet_updates`. Unit and integration tests.
2. Desktop: the bell, the popover, the time picker reused.
3. iOS: Settings → Notifications, `QuietView`, the Chats bell-slash.
4. Android: the same.

Size: PR 1 about 350 lines of core, 120 desktop, 80 per phone. PR 2 about 400 core, 350 desktop, 200 per phone.

Order: PR 1 first. It's smaller, and `RoomSynced` and `start_timers` are general. Otherwise they're independent. Release after PR 2.

---

## Open questions

1. **How the pause works.**
   - (a) Account data `org.mtrx.quiet`, decided in core on every Mtrx device, on both paths (sync and push).
   - (b) The `.m.rule.master` push rule. The server stops pushing and Element goes quiet too. But mentions can't get through, unread counts and the dock badge stop counting, and an app must run to end a pause or start and end quiet hours.
   - (c) (a), plus `.m.rule.master` for "until I turn them back on" without mentions through.
   - Recommend (a). (b) and (c) break "mentions through" and "counts keep counting". The cost of (a): other Matrix apps aren't paused, and Android still receives pushes and shows nothing.

2. **"Tomorrow morning"** (pause, and a room's "until tomorrow").
   - (a) 08:00 on the device choosing: today's before 05:00, else tomorrow's. The title shows the time.
   - (b) 09:00, as send later's "tomorrow" (`core/send_time.rs:8`).
   - (c) The end of quiet hours when they're on, else 08:00.
   - Recommend (a). It matches "Paused until 8:00", and the title says the time.

3. **Whose clock quiet hours follow.**
   - (a) Each device's own local time. No zone stored.
   - (b) The zone of the device that set them. Needs `chrono-tz` (a new dependency) and the zone's name in the data.
   - Recommend (a). Quiet hours mean "at night where I am". The pause is an instant, the same everywhere.

4. **Which day an overnight period belongs to.**
   - (a) The day it starts: Mon–Fri 22:00–07:00 covers Friday night, not Sunday night.
   - (b) The day it ends.
   - Recommend (a), as phones' own Do Not Disturb schedules do.

5. **What gets through.**
   - (a) One switch, for the pause and quiet hours: mentions and keywords (push `highlight`).
   - (b) Two switches, one for each.
   - (c) (a), plus direct chats.
   - Recommend (a), as decided. Direct chats can join later as a second switch.

6. **Where a timed mute is kept.**
   - (a) Room account data `org.mtrx.mute` per room, through a new `RoomSynced<T>` that disappearing modes move onto.
   - (b) One global map, `org.mtrx.muted_rooms`, in `Synced<T>`. Simpler to load, but two devices overwrite each other's rooms: the whole map is replaced on each write.
   - Recommend (a). No races across rooms, and the per-room code disappearing already wrote by hand becomes shared.

7. **Who ends timed mutes.**
   - (a) Every running Mtrx: the desktop, `mtrx watch`, a phone in front. As disappearing messages.
   - (b) (a), plus Android's push process, whenever any push arrives.
   - (c) The desktop only.
   - Recommend (a). (b) adds work to a path with a 20 s limit, for a gap the desktop mostly covers. It can come later if mutes are seen to last too long.

8. **The desktop button: bell or general status.**
   - (a) A bell: pause and quiet hours. Presence stays automatic, with "Share my presence" in Settings.
   - (b) A status button: our picture with its presence dot and a bell-slash. The popover adds "Automatic" / "Appear away" above the pause.
   - Recommend (a). Presence is set by each device as it comes and goes, so an "away" set on the desktop is overwritten as soon as a phone comes to the front. And presence sharing is a device setting, while the pause is for all devices: one popover would mix the two. The popover is a plain list, so a presence part can join later without moving the button.

9. **Muted rooms in the room list.**
   - (a) A bell-slash on every muted room, timed or not.
   - (b) Only on timed-muted rooms.
   - (c) None, as now.
   - Recommend (a). Today nothing shows that a room is muted. A timed mute is the same state with an end.

## Decided (2026-11-18)

All recommendations taken: 1a (account data `org.mtrx.quiet`, decided in core), 2a (08:00), 3a (each device's local time), 4a (the day an overnight period starts), 5a (one switch for mentions and keywords, off by default), 6a (room account data `org.mtrx.mute` through `RoomSynced<T>`), 7a (every running Mtrx ends timed mutes), 8a (a bell, not a presence button), 9a (a bell-slash on every muted room).

## As built: timed room mute (PR 114)

Differences from the design above:

- **Times in the FFI** are `SystemTime` (Swift `Date`, Kotlin `Instant`), as the FFI's other records have them: `RoomSettings.muted_until`, not `muted_until_ms`.
- **Morning and end times** live in `core/send_time.rs`, which PR 2 shares: `MORNING`, `tomorrow_morning`, `morning_words` ("tomorrow, 08:00" or "08:00"), `describe_until` ("14:00", "tomorrow 08:00", "Fri 08:00") and `local_at` (a wall-clock time as an instant, the first that exists when clocks go forward).
- **Room modes** are changed through `RoomModes` (`core/room_settings.rs`): the client, `NotificationSettings` and `NotifyTurns`, cloned into the timer. `send(turn, room, mode)` takes `Option<NotifyMode>`; `None` deletes the room's rules (it follows the default), confirmed by sync like a mode. `set_notify`, `mute` and the timer all use it.
- **`RoomSynced`**: `RoomAccountData` has a `Value` type, so the disappearing mode stays a `Duration` (`DisappearingMode` is a marker type). Unsaved changes are counted per room: one room's change doesn't hold back another's synced value. A change that leaves the value as it was isn't saved.
- **The timer's first sync** is the room list's: it acts once the room list service's state is SettingUp, Recovering or Running, which it only is after a sync since sync (re)started. A pass whose clearing fails doesn't wake again on its own rolled-back record; it tries again within 30 s.
- **Records** keep `until` to the millisecond, so a record synced back equals the one written.
- **Tests** reach `mute_until` and the room's own mode through `test-support` hooks (`mute_until_for_test`, `own_notify_for_test`): the integration tests are another crate.
- **The core and FFI commit** also gives the phones' test fakes the new record fields, so every commit builds.
- **Desktop:** the menu ticks the current mode; the room list's bell-slash has a "Muted" hint. Only `icons::bell_slash` comes here: an unused `bell` would fail clippy, so it comes with PR 2.
- **iOS:** `RoomInfoModel.notify(_:)` (a mode or a mute) replaces `setNotify(_:)`.
- **Android:** `MenuRow` (`ui/Rows.kt`) shows a value with a menu of entries; `ChoiceRow` is built on it. The bell-slash is `MtrxIcons.NotificationsOff`, from Material's path: the core icon set lacks it. `RoomInfoActions.notify(action)` replaces `setNotify(mode)`.
