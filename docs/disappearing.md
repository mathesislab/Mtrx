# Disappearing messages

Goal: my messages disappear (are deleted for everyone) a set time after every other member has read them. Chosen per message, or as a room's mode for all my messages there. A maximum lifetime (default 1 day, changeable in Settings) applies regardless, since some members are never seen to read. The timer runs in core, like send later; the desktop offers it first, the phones later.

Paths: `core/` = `crates/mtrx-core/src/`, `app/` = `crates/mtrx-app/src/`, `sdk/` = `~/.cargo/registry/src/index.crates.io-*/matrix-sdk-0.19.1/src/`.

---

## What Matrix offers

- No per-message expiry in the spec. MSC2228 (self-destructing events) was never merged, and no server or major client implements it.
- MSC1763 (`m.room.retention`) is server-side, for a whole room. Synapse purges by it only if its admin turns it on, and lifetimes are usually days. It knows nothing of reading.
- So it is up to the client: a deletion (redaction) sent when the time is up.

## What it can't do (said in the app)

- **Only while Mtrx runs.** The desktop app (or `mtrx watch`) deletes; closed, nothing happens until it starts again, and then it deletes what is overdue at once.
- **Deleted, not gone.**
  - Others see "Message deleted" in its place.
  - Notifications already shown, copies and screenshots stay.
  - The server keeps the deleted content for its own retention (Synapse's `redaction_retention_period`, 7 days by default).
  - Files: the event and, in encrypted rooms, its key go; the encrypted file stays on the media server (Matrix has no way for a user to delete media).
- **Read receipts.** A member who keeps receipts private (or a client that sends none) never counts as having read. The maximum lifetime catches these.
- **Only my own messages**, and only those sent with the marker below.

## Today

- **Send later** (`core/scheduled.rs`), the model for this:
  - A JSON list in the SDK's encrypted state store under `mtrx.scheduled_messages` (`get_custom_value`/`set_custom_value`).
  - `Schedule` (`:72`) holds it in a `watch`; `modify()` (`:96`) persists and publishes on change.
  - `start_scheduler()` (`:285`) runs a loop that wakes at the next due time, at most every 30 s (sleep), or when the list changes. It stops when the `Session` drops (`AbortOnDrop`, `session.rs:60`).
  - The desktop starts it after sync (`app/app/start.rs:150`); `mtrx watch` too. The FFI does not.
- **Sending:** `send_message` (`core/timeline.rs:740`) puts the text, then each file, into the room's send queue. Each is its own event.
- **Read receipts** reach core only through an open timeline (`timeline.rs:1378`, `Event.read_by`). The SDK keeps every member's latest receipt in the store: `Room::load_user_receipt(Read, thread, user)` (`sdk/room/mod.rs:3594`), filled by the sliding-sync receipt extension, which is on for every room.
- **Deleting:** `room.send_queue().redact(event_id, reason)` (`sdk/send_queue/mod.rs:603`) is persisted and retried. Deleting one's own events needs only the right to send `m.room.redaction`.
- **Account data:** core keeps global account data in `Synced<T>` (`core/account_data.rs:41`): loaded, watched, saved with rollback, followed through an event handler. There is no room account data yet; the SDK has `Room::account_data_static` and `set_account_data` (`sdk/room/mod.rs:1578`, `:1623`).

## Design

### The marker in the message

A message to disappear carries it in its content:

```json
"org.mtrx.disappearing": { "after_read": 300, "at_most": 86400 }
```

(seconds). The sending device writes it; the timer looks for it. So:
- One path for everything: a message sent from any of my devices disappears, as long as one Mtrx with the timer runs.
- A room's mode is only the composer's default.
- Other Mtrx users see the marker and can show that the message will disappear. In encrypted rooms only members see it.
- `at_most` is fixed when sending, so a later change in Settings does not change messages already sent.

### Settings (account data, shared by my devices)

- **Room mode:** room account data `org.mtrx.disappearing` = `{ "after_read": 300 }`, absent for off. It is mine alone: others' messages in the room do not change. A per-room variant of `Synced<T>`.
- **Maximum lifetime:** global account data `org.mtrx.disappearing_limit` = `{ "at_most": 86400 }` in `Synced<T>`; default 1 day.

### Core: `core/disappearing.rs`

```rust
/// One of my messages waiting to disappear.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Disappearing {
    pub room_id: OwnedRoomId,
    pub event_id: OwnedEventId,
    /// Its server time.
    pub sent_at: SystemTime,
    pub after_read: Duration,
    pub at_most: Duration,
    /// When every other member had read it, once seen.
    pub read_at: Option<SystemTime>,
    /// My edits of it, deleted with it.
    pub edits: Vec<OwnedEventId>,
}

impl Disappearing {
    /// When it goes: `after_read` after everyone read it, at the latest
    /// `at_most` after it was sent.
    pub fn due(&self) -> SystemTime;
}
```

- **Store:** `mtrx.disappearing_messages` in the state store, a list as send later has it; a `Watch` for the app (`disappearing_updates() -> Live<…>`).
- **Recording:** an event handler for my own messages in sync: one with the marker is added (also for files, each its own event); an edit of one (`m.replace`) adds its id to `edits`.
- **Who has read it:** each joined member but me. A member has read it when their latest read receipt (unthreaded or main) is on it or on a later event. Later: by place in the event cache when both are there, else by server time (`load_or_fetch_event`). A receipt sent before the message was is a quick no.
- **When to check:** on every receipt in a room with waiting messages (event handler on `SyncReceiptEvent`), when someone leaves such a room, and once when the timer starts. The first time all have read sets `read_at`.
- **Timer:** `start_disappearing()`, started and stopped as `start_scheduler()`. It wakes at the next `due()`, at most every 30 s. When one is due it deletes the message and its edits through the send queue, without a reason (a reason would be readable by the server), and drops it from the list. Overdue after a closed app: deleted at once.
- **Twice:** with two desktops running, both may delete; before deleting, a message already deleted is just dropped from the list.

### Desktop

- **Composer:** a timer button beside the clock of send later. A click opens a menu: Off, 1 minute, 5 minutes, 1 hour, 1 day after read. It starts at the room's mode and applies to the messages sent until changed. A scheduled message keeps its choice (`ScheduledMessage` gets the marker).
- **Room menu:** "Disappearing messages…" with the same choices sets the room's mode. The topic panel and the composer show a timer icon while it is on.
- **On a message:** a small timer icon by the time.
  - Hover, mine: "Disappears 5 minutes after everyone has read it (at most 1 day after sending)", then "Everyone has read it: disappears in 3 minutes".
  - Hover, someone else's: "Disappears after it was read".
- **Settings:** "Disappearing messages at most … after sending": 1 hour, 6 hours, 1 day, 7 days.

### Phones (later)

Through the FFI: send with the marker, show the icon, set the room's mode. The timer stays on the desktop and in `mtrx watch`, so a phone's disappearing messages need one of them running.

## Plan

One PR, core and desktop:
1. Core: the marker, the room mode and the limit as account data; tests.
2. Core: the list, recording from sync, the read check and the timer; tests against the test server (`testserver/`): two users, the message goes once the other read it, and at the limit when they never do.
3. Desktop: composer menu, room menu, message icon, Settings.
4. Docs.

## Decided (2026-11-16)

- Choices: after read 1 min, 5 min, 1 h, 1 day; at most 1 h, 6 h, 1 day (default), 7 days.
- A room's mode is mine alone (room account data), not a room state event for everyone.
- Overdue after the app was closed: deleted at once on start, not held for confirmation as an overdue scheduled message is.
