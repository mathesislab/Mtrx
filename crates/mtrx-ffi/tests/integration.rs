//! The apps' path through the bindings, against the local Synapse from
//! `testserver/`. Calls are awaited without a tokio runtime, as Swift and
//! Kotlin do.
//!
//! ```sh
//! testserver/run.sh
//! cargo test -p mtrx-ffi --test integration -- --ignored
//! ```

#![recursion_limit = "256"]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use futures_executor::block_on;
use mtrx_core::{
    Attachment, NewRoom, Profile, SecretStore,
    test_support::{PASSWORD, homeserver, register, unique_name},
};
use mtrx_ffi::{
    ConnectionListener, ConnectionState, EncryptionState, EncryptionStateListener, EventContent,
    MediaKind, Message, MessageKind, Mtrx, MtrxError, NotificationContent, Presence,
    PresenceListener, PresenceState, PusherInfo, QuoteState, Reaction, Reader, RecoveryListener,
    RecoveryStatus, RecoveryStep, ReplyQuote, RoomKind, RoomLists, RoomListsListener, RoomSort,
    SecretBackend, SecretError, SectionLayout, SendState, Session, SpaceKey, TimelineEvent,
    TimelineItem, TimelineItemContent, TimelineListener, TypingListener, UserProfile, Verification,
    VerificationListener, VerificationRequestListener, VerificationState,
};
use tempfile::TempDir;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Memory(Mutex<HashMap<String, String>>);

impl SecretBackend for Memory {
    fn load(&self, key: String) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().unwrap().get(&key).cloned())
    }

    fn save(&self, key: String, value: String) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert(key, value);
        Ok(())
    }

    fn delete(&self, key: String) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(&key);
        Ok(())
    }
}

/// A listener that forwards what it gets.
struct Forward<T>(Mutex<mpsc::Sender<T>>);

fn forward<T>() -> (Arc<Forward<T>>, mpsc::Receiver<T>) {
    let (tx, rx) = mpsc::channel();
    (Arc::new(Forward(Mutex::new(tx))), rx)
}

impl<T> Forward<T> {
    fn send(&self, value: T) {
        self.0.lock().unwrap().send(value).ok();
    }
}

impl RoomListsListener for Forward<Arc<RoomLists>> {
    fn on_update(&self, lists: Arc<RoomLists>) {
        self.send(lists);
    }
}

/// IDs of the rooms `key` lists, in the list's order.
fn listed(lists: &RoomLists, key: SpaceKey) -> Vec<String> {
    (lists.search(Some(key), String::new()).into_iter())
        .map(|r| r.id)
        .collect()
}

impl TimelineListener for Forward<Vec<TimelineItem>> {
    fn on_update(&self, items: Vec<TimelineItem>) {
        self.send(items);
    }
}

impl TypingListener for Forward<Vec<String>> {
    fn on_update(&self, names: Vec<String>) {
        self.send(names);
    }
}

impl ConnectionListener for Forward<ConnectionState> {
    fn on_update(&self, state: ConnectionState) {
        self.send(state);
    }
}

impl EncryptionStateListener for Forward<EncryptionState> {
    fn on_update(&self, state: EncryptionState) {
        self.send(state);
    }
}

impl RecoveryListener for Forward<RecoveryStep> {
    fn on_update(&self, step: RecoveryStep) {
        self.send(step);
    }
}

impl VerificationListener for Forward<VerificationState> {
    fn on_update(&self, state: VerificationState) {
        self.send(state);
    }
}

/// `None`: the server does not share presence.
type Presences = Option<HashMap<String, Presence>>;

impl PresenceListener for Forward<Presences> {
    fn on_update(&self, presences: HashMap<String, Presence>) {
        self.send(Some(presences));
    }

    fn on_unavailable(&self) {
        self.send(None);
    }
}

impl VerificationRequestListener for Forward<Arc<Verification>> {
    fn on_request(&self, verification: Arc<Verification>) {
        self.send(verification);
    }
}

/// Wait for an update matching `pred`.
fn wait_for<T>(what: &str, updates: &mpsc::Receiver<T>, pred: impl Fn(&T) -> bool) -> T {
    wait_within(TIMEOUT, what, updates, pred)
}

/// As [`wait_for`], for up to `timeout`.
fn wait_within<T>(
    timeout: Duration,
    what: &str,
    updates: &mpsc::Receiver<T>,
    pred: impl Fn(&T) -> bool,
) -> T {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match updates.recv_timeout(left) {
            Ok(update) if pred(&update) => return update,
            Ok(_) => {}
            Err(e) => panic!("{what}: {e}"),
        }
    }
}

fn message_body(item: &TimelineItem) -> Option<&str> {
    match &item.content {
        TimelineItemContent::Event { event } => match &event.content {
            EventContent::Message { message } => Some(&message.body),
            _ => None,
        },
        _ => None,
    }
}

#[test]
#[ignore = "needs the local Synapse (testserver/run.sh)"]
fn sign_in_list_rooms_read_and_send_then_sign_out() {
    let core_runtime = tokio::runtime::Runtime::new().unwrap();
    let name = unique_name("ffi");
    core_runtime.block_on(register(&name)).unwrap();

    // Another device of the same user makes a room with an image in it.
    let other_dir = tempfile::tempdir().unwrap();
    let image = red_dot_png();
    let room_id = core_runtime.block_on(async {
        let profile = Profile::new("other", other_dir.path(), SecretStore::File).unwrap();
        let other = mtrx_core::Session::login(profile, &homeserver(), &name, PASSWORD)
            .await
            .unwrap();
        let room = other
            .create_room(NewRoom {
                name: Some("FFI".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        other.start_sync().await;
        let timeline = other.timeline(&room).await.unwrap();
        let attachment = Attachment {
            filename: "dot.png".into(),
            mimetype: "image/png".into(),
            bytes: image.clone(),
        };
        let sent = timeline.send("", vec![attachment], None).await.unwrap();
        tokio::time::timeout(TIMEOUT, sent.sent())
            .await
            .unwrap()
            .unwrap();
        other.stop_sync().await;
        room.to_string()
    });

    let (data, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let secrets = Arc::new(Memory::default());
    let mtrx = Mtrx::new(
        data.path().to_string_lossy().into(),
        cache.path().to_string_lossy().into(),
        secrets.clone(),
    );
    assert!(block_on(mtrx.restore()).unwrap().is_none());
    let session = block_on(mtrx.login(homeserver(), name.clone(), PASSWORD.into())).unwrap();
    assert!(session.user_id().starts_with(&format!("@{name}:")));
    assert!(secrets.0.lock().unwrap().contains_key("profiles/default"));
    // Restoring while signed in opens the same account.
    drop(session);
    let session = block_on(mtrx.restore()).unwrap().expect("saved session");

    let (connection, connections) = forward();
    let _connection = session.subscribe_connection(connection);
    let (rooms, room_updates) = forward();
    let rooms_handle = block_on(session.subscribe_room_lists(rooms)).unwrap();
    block_on(session.start_sync());
    wait_for("online", &connections, |c| *c == ConnectionState::Online);
    let state = wait_for("room list with the room", &room_updates, |s| {
        s.loaded() && s.rooms().iter().any(|r| r.id == room_id)
    });
    assert_eq!(
        state.rooms().iter().find(|r| r.id == room_id).unwrap().name,
        "FFI"
    );
    rooms_handle.cancel();

    let timeline = block_on(session.timeline(room_id.clone())).unwrap();
    let (items, item_updates) = forward();
    let _timeline_handle = timeline.subscribe(items);
    block_on(timeline.paginate_back(20)).unwrap();
    let with_image = wait_for("the image", &item_updates, |items| {
        items.iter().any(|item| match &item.content {
            TimelineItemContent::Event { event } => matches!(
                &event.content,
                EventContent::Message { message } if message.media.is_some()
            ),
            _ => false,
        })
    });
    let media = with_image
        .iter()
        .find_map(|item| match &item.content {
            TimelineItemContent::Event { event } => match &event.content {
                EventContent::Message { message } => message.media.clone(),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    assert_eq!(media.kind, MediaKind::Image);
    assert_eq!(
        block_on(session.media_preview(media.clone())).unwrap(),
        image
    );
    let path = block_on(session.media_file(media.clone())).unwrap();
    assert!(path.starts_with(&*cache.path().to_string_lossy()));
    assert!(path.ends_with(".png"));
    assert_eq!(std::fs::read(&path).unwrap(), image);
    // A second time from the cache.
    assert_eq!(block_on(session.media_file(media)).unwrap(), path);

    block_on(timeline.send_text("hello **world**".into())).unwrap();
    let items = wait_for("our message, sent", &item_updates, |items| {
        items.iter().any(|item| {
            message_body(item) == Some("hello world")
                && matches!(&item.content, TimelineItemContent::Event { event }
                    if event.own && event.event_id.is_some())
        })
    });
    let ids: std::collections::HashSet<_> = items.iter().map(|i| &i.id).collect();
    assert_eq!(ids.len(), items.len(), "item IDs are unique");
    block_on(timeline.mark_read()).unwrap();

    block_on(session.logout()).unwrap();
    assert!(secrets.0.lock().unwrap().is_empty());
    assert!(!std::path::Path::new(&path).exists());
    assert!(block_on(mtrx.restore()).unwrap().is_none());
}

/// A signed-in app with its own directories, syncing.
struct Device {
    session: Arc<Session>,
    secrets: Arc<Memory>,
    dirs: (TempDir, TempDir),
}

impl Device {
    /// Another `Mtrx` on this device's data, as the app's push handler
    /// makes in a process of its own.
    fn push_handler(&self) -> Arc<Mtrx> {
        Mtrx::new(
            self.dirs.0.path().to_string_lossy().into(),
            self.dirs.1.path().to_string_lossy().into(),
            self.secrets.clone(),
        )
    }
}

fn sign_in(name: &str) -> Device {
    let (data, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let secrets = Arc::new(Memory::default());
    let mtrx = Mtrx::new(
        data.path().to_string_lossy().into(),
        cache.path().to_string_lossy().into(),
        secrets.clone(),
    );
    let session = block_on(mtrx.login(homeserver(), name.into(), PASSWORD.into())).unwrap();
    block_on(session.start_sync());
    Device {
        session,
        secrets,
        dirs: (data, cache),
    }
}

/// Another device of user `name`, outside the bindings (which cannot make
/// rooms yet), makes an encrypted room and signs out.
fn encrypted_room(core_runtime: &tokio::runtime::Runtime, name: &str) -> String {
    core_runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let profile = Profile::new("other", dir.path(), SecretStore::File).unwrap();
        let other = mtrx_core::Session::login(profile, &homeserver(), name, PASSWORD)
            .await
            .unwrap();
        let room = other
            .create_room(NewRoom {
                name: Some("diary".into()),
                encrypted: true,
                ..Default::default()
            })
            .await
            .unwrap();
        other.logout().await.unwrap();
        room.to_string()
    })
}

/// Wait until the room list has `room_id`, known to be encrypted. A room
/// can show before its encryption event has synced (the server creates its
/// state one event at a time); what is sent until then goes out unencrypted.
fn wait_for_encrypted_room(session: &Session, room_id: &str) {
    let (rooms, updates) = forward();
    let _handle = block_on(session.subscribe_room_lists(rooms)).unwrap();
    wait_for(
        "the encrypted room in the room list",
        &updates,
        |s: &Arc<RoomLists>| s.loaded() && s.rooms().iter().any(|r| r.id == room_id && r.encrypted),
    );
}

/// Send `text` to `room_id` and wait until the server has it.
fn send(session: &Session, room_id: &str, text: &str) {
    let timeline = block_on(session.timeline(room_id.into())).unwrap();
    let (items, updates) = forward();
    let _handle = timeline.subscribe(items);
    block_on(timeline.send_text(text.into())).unwrap();
    wait_for("sent", &updates, |items: &Vec<TimelineItem>| {
        items.iter().any(|item| {
            message_body(item) == Some(text)
                && matches!(&item.content, TimelineItemContent::Event { event }
                    if event.event_id.is_some())
        })
    });
}

fn is_undecryptable(item: &TimelineItem) -> bool {
    matches!(&item.content, TimelineItemContent::Event { event }
        if event.content == EventContent::UnableToDecrypt && event.utd_cause.is_some())
}

/// Load the whole room into `timeline`.
fn paginate_to_start(timeline: &mtrx_ffi::Timeline) {
    for _ in 0..10 {
        if block_on(timeline.paginate_back(20)).unwrap() {
            return;
        }
    }
    panic!("the room's start was not reached");
}

/// Poll `session`'s encryption state until `pred` holds.
fn wait_for_encryption(session: &Session, what: &str, pred: impl Fn(&EncryptionState) -> bool) {
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        let state = block_on(session.encryption_state()).unwrap();
        if pred(&state) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{what}: still {state:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
#[ignore = "needs the local Synapse (testserver/run.sh)"]
fn recovery_key_decrypts_history_on_a_new_device() {
    let core_runtime = tokio::runtime::Runtime::new().unwrap();
    let name = unique_name("ffi-recovery");
    core_runtime.block_on(register(&name)).unwrap();

    // The first device sets up cross-signing; it writes in an encrypted
    // room, then turns on recovery.
    let first = sign_in(&name);
    let room_id = encrypted_room(&core_runtime, &name);
    wait_for_encrypted_room(&first.session, &room_id);
    send(&first.session, &room_id, "written before recovery");
    assert_eq!(
        block_on(first.session.encryption_state()).unwrap(),
        EncryptionState {
            device_verified: true,
            recovery: RecoveryStatus::Disabled,
            keys_arriving: false,
        }
    );
    let key = block_on(first.session.enable_recovery()).unwrap();
    assert_eq!(
        block_on(first.session.recovery_status()),
        RecoveryStatus::Enabled
    );

    // A new device cannot read it, and says why.
    let second = sign_in(&name);
    wait_for_encrypted_room(&second.session, &room_id);
    wait_for_encryption(&second.session, "unverified, recovery incomplete", |s| {
        *s == EncryptionState {
            device_verified: false,
            recovery: RecoveryStatus::Incomplete,
            keys_arriving: false,
        }
    });
    let (listener, states) = forward();
    let _states_handle = second.session.subscribe_encryption_state(listener);
    assert_eq!(
        states.recv_timeout(TIMEOUT).expect("the state now"),
        EncryptionState {
            device_verified: false,
            recovery: RecoveryStatus::Incomplete,
            keys_arriving: false,
        }
    );
    let timeline = block_on(second.session.timeline(room_id.clone())).unwrap();
    let (items, item_updates) = forward();
    let _timeline_handle = timeline.subscribe(items);
    paginate_to_start(&timeline);
    wait_for("an undecryptable message", &item_updates, |items| {
        items.iter().any(is_undecryptable)
    });

    // A mistyped key, or another backup's, fails while checking it.
    let another = matrix_sdk_crypto::secret_storage::SecretStorageKey::new().to_base58();
    for wrong in ["not a recovery key".to_owned(), another] {
        let (steps, step_updates) = forward();
        let refused = block_on(second.session.recover(wrong.clone(), steps));
        assert!(
            matches!(&refused, Err(MtrxError::InvalidInput { reason })
                if reason == "The recovery key is not valid"),
            "{wrong}: {refused:?}"
        );
        assert_eq!(
            step_updates.try_iter().collect::<Vec<_>>(),
            [RecoveryStep::CheckingKey]
        );
    }

    // The key unlocks it; the open timeline shows the message.
    let (steps, step_updates) = forward();
    block_on(second.session.recover(key, steps)).unwrap();
    let steps: Vec<_> = step_updates.try_iter().collect();
    assert_eq!(
        steps[..2],
        [RecoveryStep::CheckingKey, RecoveryStep::ImportingIdentity]
    );
    assert!(
        steps[2..]
            .iter()
            .all(|s| matches!(s, RecoveryStep::DownloadingKeys { .. })),
        "{steps:?}"
    );
    assert!(!steps[2..].is_empty(), "keys of the room were fetched");
    wait_for("the message, decrypted", &item_updates, |items| {
        items
            .iter()
            .any(|i| message_body(i) == Some("written before recovery"))
            && !items.iter().any(is_undecryptable)
    });
    // The listener saw keys arriving, then done, without asking.
    wait_for("keys arriving", &states, |s| s.keys_arriving);
    wait_for("verified, recovery enabled", &states, |s| {
        *s == EncryptionState {
            device_verified: true,
            recovery: RecoveryStatus::Enabled,
            keys_arriving: false,
        }
    });
    assert_eq!(
        block_on(second.session.encryption_state()).unwrap(),
        EncryptionState {
            device_verified: true,
            recovery: RecoveryStatus::Enabled,
            keys_arriving: false,
        }
    );
}

#[test]
#[ignore = "needs the local Synapse (testserver/run.sh)"]
fn emoji_verification_between_two_devices() {
    let core_runtime = tokio::runtime::Runtime::new().unwrap();
    let name = unique_name("ffi-verify");
    core_runtime.block_on(register(&name)).unwrap();
    let old = sign_in(&name);
    let room_id = encrypted_room(&core_runtime, &name);
    wait_for_encrypted_room(&old.session, &room_id);
    send(&old.session, &room_id, "written before verification");

    let new = sign_in(&name);
    assert!(block_on(old.session.is_device_verified()).unwrap());
    assert!(!block_on(new.session.is_device_verified()).unwrap());
    let (listener, new_states) = forward();
    let _states_handle = new.session.subscribe_encryption_state(listener);
    wait_for("not verified", &new_states, |s: &EncryptionState| {
        !s.device_verified
    });
    // The new device cannot read what was written before it existed.
    wait_for_encrypted_room(&new.session, &room_id);
    let timeline = block_on(new.session.timeline(room_id)).unwrap();
    let (items, item_updates) = forward();
    let _timeline_handle = timeline.subscribe(items);
    paginate_to_start(&timeline);
    wait_for("an undecryptable message", &item_updates, |items| {
        items.iter().any(is_undecryptable)
    });
    // A request from a device the other does not know yet is dropped.
    for (device, other) in [(&old, &new), (&new, &old)] {
        let other = other.session.device_id();
        let deadline = std::time::Instant::now() + TIMEOUT;
        while !block_on(device.session.devices())
            .unwrap()
            .iter()
            .any(|d| d.id == other && !d.current)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "devices know each other"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // The new device asks; the old one accepts.
    let (requests, incoming) = forward();
    let _requests_handle = old.session.subscribe_verification_requests(requests);
    let outgoing = block_on(new.session.request_self_verification()).unwrap();
    assert_eq!(outgoing.other_user(), new.session.user_id());
    let (listener, outgoing_states) = forward();
    let _outgoing_handle = outgoing.subscribe(listener);
    let accepted = incoming.recv_timeout(TIMEOUT).expect("incoming request");
    assert_eq!(accepted.other_user(), old.session.user_id());
    let (listener, accepted_states) = forward();
    let _accepted_handle = accepted.subscribe(listener);
    block_on(accepted.accept()).unwrap();

    // Both show the same emoji; both confirm.
    let emojis = |state: &VerificationState| matches!(state, VerificationState::Emojis { .. });
    let a = wait_for("emoji (new)", &outgoing_states, emojis);
    let b = wait_for("emoji (old)", &accepted_states, emojis);
    assert_eq!(a, b, "both devices show the same emoji");
    let VerificationState::Emojis { emojis } = &a else {
        unreachable!()
    };
    assert_eq!(emojis.len(), 7);
    assert!(
        emojis
            .iter()
            .all(|e| !e.symbol.is_empty() && !e.description.is_empty())
    );
    assert_eq!(outgoing.state(), a);

    block_on(outgoing.confirm()).unwrap();
    block_on(accepted.confirm()).unwrap();
    let done = |state: &VerificationState| *state == VerificationState::Done;
    wait_for("done (new)", &outgoing_states, done);
    wait_for("done (old)", &accepted_states, done);
    assert!(block_on(new.session.is_device_verified()).unwrap());
    wait_for("verified, through the listener", &new_states, |s| {
        s.device_verified
    });

    // Now it can, in the timeline already open.
    wait_for("the message, decrypted", &item_updates, |items| {
        items
            .iter()
            .any(|i| message_body(i) == Some("written before verification"))
            && !items.iter().any(is_undecryptable)
    });
}

/// `ids` are in the order of `lists.rooms()`.
fn in_list_order(lists: &RoomLists, ids: &[String]) -> bool {
    let rooms = lists.rooms();
    let positions: Vec<_> = ids
        .iter()
        .map(|id| rooms.iter().position(|r| &r.id == id).unwrap())
        .collect();
    positions.is_sorted()
}

#[test]
#[ignore = "needs the local Synapse (testserver/run.sh)"]
fn rooms_by_space_with_direct_chats_and_invites() {
    let core_runtime = tokio::runtime::Runtime::new().unwrap();
    let [alice, bob, carol] = ["ffi-spaces", "ffi-spaces-bob", "ffi-spaces-carol"].map(|prefix| {
        let name = unique_name(prefix);
        core_runtime.block_on(register(&name)).unwrap();
        name
    });

    // Alice's other device makes two spaces, rooms in one of them (one
    // through a nested space) and a room outside; Bob joins the space and
    // has a direct chat with her. Bob invites her to a room in the space;
    // Carol, in no space, invites her to a direct chat and a room.
    let dir = tempfile::tempdir().unwrap();
    let ids = core_runtime.block_on(async {
        let login = |name: &str, profile: &str| {
            let profile = Profile::new(profile, dir.path(), SecretStore::File).unwrap();
            let name = name.to_owned();
            async move {
                mtrx_core::Session::login(profile, &homeserver(), &name, PASSWORD)
                    .await
                    .unwrap()
            }
        };
        let (a, b, c) = (
            login(&alice, "alice").await,
            login(&bob, "bob").await,
            login(&carol, "carol").await,
        );
        let make = |name: &str, space: bool, parent: Option<&mtrx_core::OwnedRoomId>| NewRoom {
            name: Some(name.into()),
            space,
            parent: parent.cloned(),
            ..Default::default()
        };
        let team = a.create_room(make("Team", true, None)).await.unwrap();
        let other = a.create_room(make("Other", true, None)).await.unwrap();
        let lab = a
            .create_room(make("Lab", false, Some(&team)))
            .await
            .unwrap();
        let sub = a.create_room(make("Sub", true, Some(&team))).await.unwrap();
        let deep = a
            .create_room(make("Deep", false, Some(&sub)))
            .await
            .unwrap();
        let loose = a.create_room(make("Loose", false, None)).await.unwrap();
        a.invite(&team, &b.user_id()).await.unwrap();
        b.join(<&mtrx_core::RoomOrAliasId>::from(&*team), &[])
            .await
            .unwrap();
        let with_bob = a.direct_chat(&b.user_id()).await.unwrap();
        b.join(<&mtrx_core::RoomOrAliasId>::from(&*with_bob), &[])
            .await
            .unwrap();
        let offer = b
            .create_room(NewRoom {
                name: Some("Offer".into()),
                invite: vec![a.user_id()],
                ..Default::default()
            })
            .await
            .unwrap();
        a.add_to_space(&team, &offer).await.unwrap();
        let with_carol = c.direct_chat(&a.user_id()).await.unwrap();
        let party = c
            .create_room(NewRoom {
                name: Some("Party".into()),
                invite: vec![a.user_id()],
                ..Default::default()
            })
            .await
            .unwrap();
        [
            team, other, lab, sub, deep, loose, with_bob, offer, with_carol, party,
        ]
        .map(|id| id.to_string())
    });
    let [
        team,
        other,
        lab,
        sub,
        deep,
        loose,
        with_bob,
        offer,
        with_carol,
        party,
    ] = ids;

    let device = sign_in(&alice);
    let (rooms, updates) = forward();
    let _handle = block_on(device.session.subscribe_room_lists(rooms)).unwrap();
    let team_key = || SpaceKey::Space { id: team.clone() };
    let state = wait_for("the rooms by space", &updates, |s: &Arc<RoomLists>| {
        s.loaded()
            && listed(s, team_key()).contains(&with_bob)
            && [&loose, &party]
                .iter()
                .all(|id| listed(s, SpaceKey::Outside).contains(id))
            && s.rooms().iter().any(|r| r.id == with_carol)
    });

    // The spaces page: the two top-level spaces, in list order, then Home.
    let spaces: Vec<_> = (state.top_level_spaces().into_iter())
        .map(|s| s.id)
        .collect();
    assert_eq!(
        spaces.iter().collect::<std::collections::HashSet<_>>(),
        [&team, &other]
            .into_iter()
            .collect::<std::collections::HashSet<_>>()
    );
    assert!(in_list_order(&state, &spaces));
    assert_eq!(state.spaces().last().unwrap().key, SpaceKey::Outside);

    // Team lists its rooms through the nested space (not the nested space
    // itself), the invite, and the direct chat with its member Bob.
    let team_rooms = listed(&state, team_key());
    let mut expected = vec![lab, deep, offer.clone(), with_bob.clone()];
    expected.sort();
    let mut sorted = team_rooms.clone();
    sorted.sort();
    assert_eq!(sorted, expected, "{:#?}", state.rooms());
    assert!(in_list_order(&state, &team_rooms));
    assert!(!team_rooms.contains(&sub));
    assert!(listed(&state, SpaceKey::Space { id: other }).is_empty());

    // Outside any space: the room and the invite to a room, in list order.
    let outside = listed(&state, SpaceKey::Outside);
    assert_eq!(outside.len(), 2, "{:#?}", state.rooms());
    assert!(in_list_order(&state, &outside));

    // The Chats tab: both direct chats; Carol's still an invite.
    let rooms = state.rooms();
    let summary = |id: &str| rooms.iter().find(|r| r.id == id).unwrap();
    assert_eq!(summary(&with_bob).kind, RoomKind::Direct);
    assert_eq!(
        (summary(&with_carol).kind, summary(&with_carol).invited),
        (RoomKind::Direct, true)
    );
    assert!(summary(&offer).invited);
    assert_eq!(summary(&sub).kind, RoomKind::Space);
}

/// Through the bindings alone: a space and a room in it made, inviting
/// Bob; its settings and topic changed; the space's rooms listed; the invite
/// accepted, a role given, and the room left.
#[test]
#[ignore = "needs the local Synapse (testserver/run.sh)"]
fn rooms_made_set_up_joined_and_left() {
    let core_runtime = tokio::runtime::Runtime::new().unwrap();
    let [alice, bob] = ["ffi-manage", "ffi-manage-bob"].map(|prefix| {
        let name = unique_name(prefix);
        core_runtime.block_on(register(&name)).unwrap();
        name
    });
    let (a, b) = (sign_in(&alice), sign_in(&bob));
    let (a_rooms, a_updates) = forward();
    let _a_list = block_on(a.session.subscribe_room_lists(a_rooms)).unwrap();
    let (b_rooms, b_updates) = forward();
    let _b_list = block_on(b.session.subscribe_room_lists(b_rooms)).unwrap();

    assert!(block_on(a.session.offers_owners()));
    let space = block_on(a.session.create_room(mtrx_ffi::NewRoom {
        name: "Lab".into(),
        space: true,
        ..Default::default()
    }))
    .unwrap();
    let room = block_on(a.session.create_room(mtrx_ffi::NewRoom {
        name: "coffee".into(),
        encrypted: true,
        invite: vec![b.session.user_id()],
        parent: Some(space.clone()),
        ..Default::default()
    }))
    .unwrap();
    let space_key = || SpaceKey::Space { id: space.clone() };
    let state = wait_for("the room in its space", &a_updates, |s: &Arc<RoomLists>| {
        listed(s, space_key()).contains(&room)
    });
    // In the space's Rooms section, among all the desktop's sections.
    let choices = state.section_choices(space_key(), SectionLayout::default());
    let keys: Vec<_> = choices.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "invites",
            "direct_messages",
            "favourites",
            "rooms",
            "low_priority"
        ]
    );
    let sections = state.space_list(
        space_key(),
        SectionLayout::default(),
        RoomSort::ListOrder,
        String::new(),
    );
    let shown: Vec<_> = (sections.iter())
        .map(|s| {
            (
                s.key.as_str(),
                s.rooms.iter().map(|r| &r.id).collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(shown, [("rooms", vec![&room])]);
    // Notes at the top: on until turned off, for all devices.
    assert!(state.notes_at_top());
    block_on(a.session.set_notes_at_top(false)).unwrap();
    wait_for("Notes not at the top", &a_updates, |s: &Arc<RoomLists>| {
        !s.notes_at_top()
    });
    // A draft is kept for the room, which the list marks, until sent.
    let draft = mtrx_ffi::Draft {
        text: "half a thought".into(),
        reply_to: Some("$answered:example.org".into()),
    };
    block_on(a.session.save_draft(room.clone(), draft.clone())).unwrap();
    assert_eq!(
        block_on(a.session.draft(room.clone())).unwrap(),
        Some(draft)
    );
    wait_for("the room's draft", &a_updates, |s: &Arc<RoomLists>| {
        s.drafts() == [room.clone()]
    });
    block_on(a.session.save_draft(room.clone(), Default::default())).unwrap();
    assert_eq!(block_on(a.session.draft(room.clone())).unwrap(), None);
    wait_for("no drafts", &a_updates, |s: &Arc<RoomLists>| {
        s.drafts().is_empty()
    });

    // Its settings: ours to change; the topic with a call link.
    let settings = block_on(a.session.room_settings(room.clone())).unwrap();
    assert!(settings.can_rename && settings.can_change_access && settings.encrypted);
    assert_eq!(settings.version.as_deref(), Some("12"));
    assert_eq!(settings.spaces[0].space_id, space);
    let topic = mtrx_ffi::compose_topic(mtrx_ffi::Topic {
        text: "Breaks".into(),
        links: vec![mtrx_ffi::RoomLink {
            call: true,
            title: Some("Daily".into()),
            url: "https://meet.example.org/coffee".into(),
            passcode: None,
            name: String::new(),
        }],
    });
    block_on(a.session.set_room_topic(room.clone(), topic)).unwrap();
    block_on(
        a.session
            .set_notify_mode(room.clone(), mtrx_ffi::NotifyMode::Mentions),
    )
    .unwrap();
    block_on(
        a.session
            .set_room_access(room.clone(), mtrx_ffi::Access::SpaceMembers),
    )
    .unwrap();
    let settings = block_on(a.session.room_settings(room.clone())).unwrap();
    assert_eq!(
        (settings.notify, settings.access),
        (
            mtrx_ffi::NotifyMode::Mentions,
            mtrx_ffi::Access::SpaceMembers
        )
    );
    let state = wait_for("the topic", &a_updates, |s: &Arc<RoomLists>| {
        s.rooms()
            .iter()
            .any(|r| r.id == room && r.topic.as_deref().is_some_and(|t| t.contains("[call")))
    });
    let rooms = state.rooms();
    let summary = rooms.iter().find(|r| r.id == room).unwrap();
    let parsed = mtrx_ffi::parse_topic(summary.topic.clone().unwrap());
    assert_eq!(
        (parsed.text.as_str(), parsed.links[0].name.as_str()),
        ("Breaks", "Daily")
    );

    // The space lists it; Bob accepts the invite.
    let children = block_on(a.session.space_children(space.clone())).unwrap();
    let listed = children.iter().find(|r| r.id == room).unwrap();
    assert_eq!(listed.access, mtrx_ffi::JoinAccess::SpaceMembers);
    wait_for("Bob's invite", &b_updates, |s: &Arc<RoomLists>| {
        s.rooms().iter().any(|r| r.id == room && r.invited)
    });
    assert_eq!(
        block_on(b.session.join(room.clone(), vec![])).unwrap(),
        room
    );
    // They share the room now (not the space, which Bob has not joined).
    let deadline = std::time::Instant::now() + TIMEOUT;
    let shared = loop {
        let shared = block_on(a.session.shared_rooms(b.session.user_id())).unwrap();
        if shared.contains(&room) {
            break shared;
        }
        assert!(std::time::Instant::now() < deadline, "the shared room");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(!shared.contains(&space), "{shared:?}");

    // Alice makes him a moderator; he may not manage her.
    let level = mtrx_ffi::role_level(mtrx_ffi::Role::Moderator, 0);
    let bob_id = b.session.user_id();
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        let members = block_on(a.session.members(room.clone())).unwrap();
        if let Some(bob) = members.iter().find(|m| m.user_id == bob_id && !m.invited) {
            assert!(bob.manage.up_to.is_some() && bob.manage.remove);
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Bob joined");
        std::thread::sleep(Duration::from_millis(200));
    }
    block_on(a.session.set_level(room.clone(), bob_id.clone(), level)).unwrap();
    let members = block_on(b.session.members(room.clone())).unwrap();
    assert!(members.iter().all(|m| !m.manage.remove));

    block_on(b.session.leave(room.clone())).unwrap();
    wait_for("Bob left", &b_updates, |s: &Arc<RoomLists>| {
        !s.rooms().iter().any(|r| r.id == room)
    });
}

/// A 1×1 PNG, opaque red.
fn red_dot_png() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8,
        0xcf, 0xc0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xdd, 0x8d, 0xb0, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ]
}

/// A timeline's items as its listener gets them, and the latest.
struct Items {
    updates: mpsc::Receiver<Vec<TimelineItem>>,
    latest: Vec<TimelineItem>,
    _handle: Arc<mtrx_ffi::TaskHandle>,
}

impl Items {
    fn follow(timeline: &mtrx_ffi::Timeline) -> Self {
        let (items, updates) = forward();
        Self {
            _handle: timeline.subscribe(items),
            updates,
            latest: Vec::new(),
        }
    }

    /// Wait until the items match `pred` (maybe they do already).
    fn wait(&mut self, what: &str, pred: impl Fn(&[TimelineItem]) -> bool) -> &[TimelineItem] {
        if !pred(&self.latest) {
            self.latest = wait_for(what, &self.updates, |items| pred(items));
        }
        &self.latest
    }
}

/// The row (its ID) and event of the message whose text is `body`.
fn find<'a>(items: &'a [TimelineItem], body: &str) -> Option<(&'a str, &'a TimelineEvent)> {
    items.iter().find_map(|item| match &item.content {
        TimelineItemContent::Event { event } if message_body(item) == Some(body) => {
            Some((item.id.as_str(), event))
        }
        _ => None,
    })
}

fn message(event: &TimelineEvent) -> &Message {
    match &event.content {
        EventContent::Message { message } => message,
        other => panic!("not a message: {other:?}"),
    }
}

/// The message with `body`, sent (with an event ID).
fn sent<'a>(items: &'a [TimelineItem], body: &str) -> Option<(&'a str, &'a TimelineEvent)> {
    find(items, body).filter(|(_, event)| event.event_id.is_some())
}

/// Wait until the room list has `room_id`.
fn wait_for_room(session: &Session, room_id: &str) {
    let (rooms, updates) = forward();
    let _handle = block_on(session.subscribe_room_lists(rooms)).unwrap();
    wait_for(
        "the room in the room list",
        &updates,
        |s: &Arc<RoomLists>| s.rooms().iter().any(|r| r.id == room_id && !r.invited),
    );
}

/// Users `names` made, and a room made by the first, which the others
/// joined, all outside the bindings (which cannot make rooms yet).
fn room_of(core_runtime: &tokio::runtime::Runtime, prefixes: &[&str]) -> (Vec<String>, String) {
    let room = NewRoom {
        name: Some("talk".into()),
        ..Default::default()
    };
    room_like(core_runtime, prefixes, room)
}

/// As [`room_of`], the room made as `room` says.
fn room_like(
    core_runtime: &tokio::runtime::Runtime,
    prefixes: &[&str],
    room: NewRoom,
) -> (Vec<String>, String) {
    let names: Vec<String> = prefixes
        .iter()
        .map(|prefix| {
            let name = unique_name(prefix);
            core_runtime.block_on(register(&name)).unwrap();
            name
        })
        .collect();
    let dir = tempfile::tempdir().unwrap();
    let room = core_runtime.block_on(async {
        let mut sessions = Vec::new();
        for name in &names {
            let profile = Profile::new(name, dir.path(), SecretStore::File).unwrap();
            sessions.push(
                mtrx_core::Session::login(profile, &homeserver(), name, PASSWORD)
                    .await
                    .unwrap(),
            );
        }
        let room = sessions[0]
            .create_room(NewRoom {
                invite: sessions[1..].iter().map(|s| s.user_id()).collect(),
                ..room
            })
            .await
            .unwrap();
        for other in &sessions[1..] {
            other
                .join(<&mtrx_core::RoomOrAliasId>::from(&*room), &[])
                .await
                .unwrap();
        }
        for session in sessions {
            session.logout().await.unwrap();
        }
        room.to_string()
    });
    (names, room)
}

#[test]
#[ignore = "needs the local Synapse (testserver/run.sh)"]
fn reactions_replies_edits_receipts_and_typing() {
    let core_runtime = tokio::runtime::Runtime::new().unwrap();
    let (names, room_id) = room_of(&core_runtime, &["ffi-talk", "ffi-talk-bob"]);
    let (alice, bob) = (sign_in(&names[0]), sign_in(&names[1]));
    let (alice_id, bob_id) = (alice.session.user_id(), bob.session.user_id());
    let reader = |id: &str, name: &str| Reader {
        user_id: id.into(),
        name: Some(name.into()),
        avatar_url: None,
    };
    let (alice_reader, bob_reader) = (reader(&alice_id, &names[0]), reader(&bob_id, &names[1]));
    wait_for_room(&alice.session, &room_id);
    wait_for_room(&bob.session, &room_id);
    let alice_timeline = block_on(alice.session.timeline(room_id.clone())).unwrap();
    let bob_timeline = block_on(bob.session.timeline(room_id.clone())).unwrap();
    let mut alice_items = Items::follow(&alice_timeline);
    let mut bob_items = Items::follow(&bob_timeline);

    // Bob asks; Alice replies, quoting him.
    block_on(bob_timeline.send_text("Which dataset?".into())).unwrap();
    let items = alice_items.wait("Alice gets the question", |i| {
        sent(i, "Which dataset?").is_some()
    });
    let (question_row, question) = sent(items, "Which dataset?").unwrap();
    let (question_row, question_id) = (question_row.to_owned(), question.event_id.clone());
    assert!(!question.editable);
    block_on(alice_timeline.reply(question_row.clone(), "The **new** one".into())).unwrap();
    let items = bob_items.wait("Bob sees the reply with its quote", |i| {
        find(i, "The new one").is_some_and(|(_, e)| {
            message(e)
                .reply_to
                .as_ref()
                .is_some_and(|q| q.state == QuoteState::Loaded && q.sender_name.is_some())
        })
    });
    let reply = find(items, "The new one").unwrap().1;
    assert_eq!(
        message(reply).reply_to,
        Some(ReplyQuote {
            event_id: question_id.unwrap(),
            state: QuoteState::Loaded,
            sender_id: Some(bob_id.clone()),
            sender_name: Some(names[1].clone()),
            kind: Some(MessageKind::Text),
            body: Some("Which dataset?".into()),
        })
    );
    let bob_question_row = find(items, "Which dataset?").unwrap().0.to_owned();

    // Reactions, counted and named on both sides, toggled on and off.
    let reactions = |items: &[TimelineItem]| {
        find(items, "Which dataset?").map(|(_, e)| message(e).reactions.clone())
    };
    let thumbs = |count, by_us, senders: &[&Reader]| {
        Some(vec![Reaction {
            key: "👍".into(),
            count,
            by_us,
            senders: senders.iter().map(|&r| r.clone()).collect(),
        }])
    };
    block_on(alice_timeline.react(question_row.clone(), "👍".into())).unwrap();
    bob_items.wait("Bob sees Alice's reaction", |i| {
        reactions(i) == thumbs(1, false, &[&alice_reader])
    });
    block_on(bob_timeline.react(bob_question_row.clone(), "👍".into())).unwrap();
    alice_items.wait("Alice sees both reactions", |i| {
        reactions(i) == thumbs(2, true, &[&alice_reader, &bob_reader])
    });
    bob_items.wait("Bob sees both reactions", |i| {
        reactions(i) == thumbs(2, true, &[&alice_reader, &bob_reader])
    });
    block_on(alice_timeline.react(question_row, "👍".into())).unwrap();
    alice_items.wait("Alice sees only Bob's", |i| {
        reactions(i) == thumbs(1, false, &[&bob_reader])
    });
    bob_items.wait("Bob sees only his", |i| {
        reactions(i) == thumbs(1, true, &[&bob_reader])
    });
    block_on(bob_timeline.react(bob_question_row, "👍".into())).unwrap();
    alice_items.wait("no reactions left", |i| reactions(i) == Some(Vec::new()));

    // Alice edits her reply, then deletes it.
    let items = alice_items.wait("Alice's reply sent", |i| sent(i, "The new one").is_some());
    let (reply_row, reply) = sent(items, "The new one").unwrap();
    assert!(reply.editable);
    assert_eq!(message(reply).raw_body, "The **new** one");
    let reply_row = reply_row.to_owned();
    block_on(alice_timeline.edit(reply_row.clone(), "The *newest* one".into())).unwrap();
    let items = bob_items.wait("Bob sees the edit", |i| {
        find(i, "The newest one").is_some_and(|(_, e)| message(e).edited)
    });
    let bob_reply_row = find(items, "The newest one").unwrap().0.to_owned();
    let items = alice_items.wait("Alice sees the edit", |i| {
        find(i, "The newest one").is_some_and(|(_, e)| message(e).edited)
    });
    assert_eq!(
        message(find(items, "The newest one").unwrap().1).raw_body,
        "The *newest* one"
    );
    block_on(alice_timeline.delete(reply_row)).unwrap();
    bob_items.wait("Bob sees it deleted", |i| {
        i.iter().any(|item| {
            item.id == bob_reply_row
                && matches!(&item.content, TimelineItemContent::Event { event }
                    if event.content == EventContent::Redacted)
        })
    });

    // Typing, by name.
    let (typing, names_typing) = forward();
    let _typing = bob_timeline.subscribe_typing(typing);
    block_on(alice_timeline.set_typing(true)).unwrap();
    wait_for("Bob sees Alice typing", &names_typing, |n: &Vec<String>| {
        *n == [names[0].clone()]
    });
    block_on(alice_timeline.set_typing(false)).unwrap();
    wait_for("Bob sees Alice stop", &names_typing, |n: &Vec<String>| {
        n.is_empty()
    });

    // Bob reads Alice's latest message: his receipt moves there, with his
    // name.
    block_on(alice_timeline.send_text("Thanks".into())).unwrap();
    bob_items.wait("Bob gets the thanks", |i| sent(i, "Thanks").is_some());
    let read_by_bob = |items: &[TimelineItem]| -> Vec<String> {
        items
            .iter()
            .filter(|item| {
                matches!(&item.content, TimelineItemContent::Event { event }
                if event.read_by.contains(&bob_reader))
            })
            .map(|item| item.id.clone())
            .collect()
    };
    let items = alice_items.wait("the thanks sent", |i| sent(i, "Thanks").is_some());
    let thanks_row = sent(items, "Thanks").unwrap().0.to_owned();
    assert!(!read_by_bob(items).contains(&thanks_row));
    block_on(bob_timeline.mark_read()).unwrap();
    alice_items.wait("Alice sees Bob's receipt", |i| {
        read_by_bob(i) == [thanks_row.clone()]
    });
}

#[test]
#[ignore = "needs the local Synapse (testserver/run.sh)"]
fn failed_sends_are_retried_or_discarded() {
    let core_runtime = tokio::runtime::Runtime::new().unwrap();
    let (names, room_id) = room_of(&core_runtime, &["ffi-unsent"]);
    let device = sign_in(&names[0]);
    wait_for_room(&device.session, &room_id);
    let timeline = block_on(device.session.timeline(room_id)).unwrap();
    let mut items = Items::follow(&timeline);

    // The server refuses events over 64 KiB, whatever we try.
    let too_long = "x".repeat(70_000);
    let failed = |i: &[TimelineItem]| {
        find(i, &too_long).is_some_and(|(_, e)| matches!(e.send_state, SendState::Failed { .. }))
    };
    block_on(timeline.send_text(too_long.clone())).unwrap();
    let row = find(items.wait("shown as failed", failed), &too_long)
        .unwrap()
        .0
        .to_owned();

    // Sent again: sending, then failed again.
    block_on(timeline.retry(row.clone())).unwrap();
    items.wait("sending again", |i| {
        find(i, &too_long).is_some_and(|(_, e)| e.send_state == SendState::Sending)
    });
    items.wait("failed again", failed);

    // What is queued after it waits for it; once it is dropped, it is sent.
    block_on(timeline.send_text("after the long one".into())).unwrap();
    block_on(timeline.discard(row)).unwrap();
    let items = items.wait("dropped, and the next one sent", |i| {
        find(i, &too_long).is_none()
            && sent(i, "after the long one").is_some_and(|(_, e)| e.send_state == SendState::Sent)
    });
    let row = sent(items, "after the long one").unwrap().0.to_owned();
    assert!(matches!(
        block_on(timeline.retry(row.clone())),
        Err(MtrxError::Other { .. })
    ));
    assert!(matches!(
        block_on(timeline.discard(row)),
        Err(MtrxError::Other { .. })
    ));
    assert!(matches!(
        block_on(timeline.react("no-such-row".into(), "👍".into())),
        Err(MtrxError::InvalidInput { .. })
    ));
}

#[test]
#[ignore = "needs the local Synapse (testserver/run.sh)"]
fn profiles_and_inviting_to_a_space() {
    let core_runtime = tokio::runtime::Runtime::new().unwrap();
    let [alice, bob] = ["ffi-invite", "ffi-invite-bob"].map(|prefix| {
        let name = unique_name(prefix);
        core_runtime.block_on(register(&name)).unwrap();
        name
    });
    // Alice's other device makes a space, outside the bindings (which
    // cannot make rooms yet).
    let space_id = core_runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let profile = Profile::new("other", dir.path(), SecretStore::File).unwrap();
        let other = mtrx_core::Session::login(profile, &homeserver(), &alice, PASSWORD)
            .await
            .unwrap();
        let space = other
            .create_room(NewRoom {
                name: Some("Group".into()),
                space: true,
                ..Default::default()
            })
            .await
            .unwrap();
        other.logout().await.unwrap();
        space.to_string()
    });
    let bob_name = bob.clone();
    let (alice, bob) = (sign_in(&alice), sign_in(&bob));
    let bob_id = bob.session.user_id();
    wait_for_room(&alice.session, &space_id);

    // Alice looks Bob up: his name (the server's default), no picture.
    assert_eq!(
        block_on(alice.session.user_profile(bob_id.clone())).unwrap(),
        UserProfile {
            user_id: bob_id.clone(),
            name: Some(bob_name),
            avatar_url: None,
        }
    );
    assert!(matches!(
        block_on(alice.session.user_profile("bob".into())),
        Err(MtrxError::InvalidInput { .. })
    ));

    // She invites him to the space; his room list shows the invite.
    let (rooms, updates) = forward();
    let _handle = block_on(bob.session.subscribe_room_lists(rooms)).unwrap();
    block_on(alice.session.invite(space_id.clone(), bob_id.clone())).unwrap();
    // The invite can show before its state (the space's type and name)
    // has been read: wait for that too.
    let deadline = std::time::Instant::now() + TIMEOUT;
    let mut seen = None;
    let state = loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let Ok(state) = updates.recv_timeout(left) else {
            panic!("the space invite, named and a space; last seen: {seen:?}");
        };
        seen = state
            .rooms()
            .iter()
            .find(|r| r.id == space_id && r.invited)
            .cloned();
        if seen
            .as_ref()
            .is_some_and(|r| r.kind == RoomKind::Space && r.name == "Group")
        {
            break state;
        }
    };
    assert!(listed(&state, SpaceKey::Outside).contains(&space_id));
}

#[test]
#[ignore = "needs the local Synapse (testserver/run.sh)"]
fn pushers_are_set_listed_and_deleted() {
    let core_runtime = tokio::runtime::Runtime::new().unwrap();
    let name = unique_name("ffi-pusher");
    core_runtime.block_on(register(&name)).unwrap();
    let device = sign_in(&name);
    let session = &device.session;
    // Synapse takes any gateway with the notify path; it is never reached.
    let gateway = "http://127.0.0.1:9/_matrix/push/v1/notify".to_owned();
    let pushkey = "https://ntfy.example.org/upTEST?up=1".to_owned();
    let set = |device_name: &str| {
        block_on(session.set_pusher(
            gateway.clone(),
            pushkey.clone(),
            "org.mtrx.app".into(),
            "Mtrx".into(),
            device_name.into(),
            "en".into(),
        ))
    };
    let pusher = |device_name: &str| PusherInfo {
        pushkey: pushkey.clone(),
        app_id: "org.mtrx.app".into(),
        app_name: "Mtrx".into(),
        device_name: device_name.into(),
        lang: "en".into(),
        gateway_url: Some(gateway.clone()),
        event_id_only: true,
    };
    assert!(block_on(session.pushers()).unwrap().is_empty());

    set("Pixel").unwrap();
    assert_eq!(block_on(session.pushers()).unwrap(), vec![pusher("Pixel")]);
    // Again, renamed: still one.
    set("Pixel 9").unwrap();
    assert_eq!(
        block_on(session.pushers()).unwrap(),
        vec![pusher("Pixel 9")]
    );

    block_on(session.delete_pusher(pushkey.clone())).unwrap();
    assert!(block_on(session.pushers()).unwrap().is_empty());
    // Deleting again is fine.
    block_on(session.delete_pusher(pushkey.clone())).unwrap();

    assert!(matches!(
        block_on(session.set_pusher(
            "not a url".into(),
            pushkey.clone(),
            "org.mtrx.app".into(),
            "Mtrx".into(),
            "Pixel".into(),
            "en".into(),
        )),
        Err(MtrxError::InvalidInput { .. })
    ));
}

/// The sent message whose text ends with `tail`: the timeline shows a
/// mention as the person's name, not the `@user:server` typed.
fn sent_ending<'a>(items: &'a [TimelineItem], tail: &str) -> Option<&'a TimelineEvent> {
    items.iter().find_map(|item| match &item.content {
        TimelineItemContent::Event { event }
            if event.event_id.is_some()
                && message_body(item).is_some_and(|b| b.ends_with(tail)) =>
        {
            Some(event)
        }
        _ => None,
    })
}

/// Send `text` to the room of `timeline` and return its event ID once
/// `items` (the room's timeline on the sender's side) shows it sent, as
/// a message ending with `tail`.
fn send_for_id(timeline: &mtrx_ffi::Timeline, items: &mut Items, text: &str, tail: &str) -> String {
    block_on(timeline.send_text(text.into())).unwrap();
    let items = items.wait(text, |items| sent_ending(items, tail).is_some());
    sent_ending(items, tail).unwrap().event_id.clone().unwrap()
}

#[test]
#[ignore = "needs the local Synapse (testserver/run.sh)"]
fn a_push_handler_reads_messages_while_the_app_syncs() {
    let core_runtime = tokio::runtime::Runtime::new().unwrap();
    let room = NewRoom {
        name: Some("lab".into()),
        encrypted: true,
        ..Default::default()
    };
    let (names, room_id) = room_like(&core_runtime, &["ffi-push", "ffi-push-bob"], room);
    let (alice, bob) = (sign_in(&names[0]), sign_in(&names[1]));
    let bob_id = bob.session.user_id();
    wait_for_encrypted_room(&alice.session, &room_id);
    wait_for_encrypted_room(&bob.session, &room_id);
    let alice_timeline = block_on(alice.session.timeline(room_id.clone())).unwrap();
    let bob_timeline = block_on(bob.session.timeline(room_id.clone())).unwrap();
    let mut alice_items = Items::follow(&alice_timeline);
    let mut bob_items = Items::follow(&bob_timeline);

    // Bob's app syncs on; his push handler reads what Alice sent. The
    // app has the message's key once it shows the message.
    let push = bob.push_handler();
    let fetch = |event_id: &str| -> NotificationContent {
        block_on(push.notification(room_id.clone(), event_id.into()))
            .unwrap()
            .expect("something to show")
    };
    let mention = format!("@{} the run finished", &bob_id[1..]);
    let event_id = send_for_id(
        &alice_timeline,
        &mut alice_items,
        &mention,
        " the run finished",
    );
    bob_items.wait("Alice's mention", |items| {
        sent_ending(items, " the run finished").is_some()
    });
    let content = fetch(&event_id);
    assert_eq!(content.room_id, room_id);
    assert_eq!(content.event_id, event_id);
    assert_eq!(content.room_name, "lab");
    assert_eq!(content.sender_id, alice.session.user_id());
    assert_eq!(content.sender_name, names[0]);
    // The message's plain text, as typed (the timeline shows the name).
    assert_eq!(content.body, mention);
    assert!(content.is_mention, "{content:?}");
    assert!(content.should_notify, "{content:?}");

    // Without a mention: the room has two members, so it notifies too.
    let plain = "plots attached";
    let event_id = send_for_id(&alice_timeline, &mut alice_items, plain, plain);
    bob_items.wait("Alice's message", |items| sent(items, plain).is_some());
    let content = fetch(&event_id);
    assert_eq!(content.body, plain);
    assert!(!content.is_mention && content.should_notify, "{content:?}");

    // Bob's own message, sent from his app: nothing to show (the push
    // rules never notify for one's own).
    let own = "thanks";
    let event_id = send_for_id(&bob_timeline, &mut bob_items, own, own);
    assert_eq!(
        block_on(push.notification(room_id.clone(), event_id)).unwrap(),
        None
    );

    // The app's store is still sound: it decrypts what comes next.
    let later = "one more";
    send_for_id(&alice_timeline, &mut alice_items, later, later);
    bob_items.wait("Alice's later message", |items| {
        sent(items, later).is_some()
    });

    // A malformed ID, and an event that is not there.
    assert!(matches!(
        block_on(push.notification(room_id.clone(), "nope".into())),
        Err(MtrxError::InvalidInput { .. })
    ));
    assert!(matches!(
        block_on(push.notification(room_id.clone(), "$missing:localhost".into())),
        Err(MtrxError::Other { .. })
    ));
}

#[test]
#[ignore = "needs the local Synapse (testserver/run.sh)"]
fn presence_is_set_read_and_followed() {
    let core_runtime = tokio::runtime::Runtime::new().unwrap();
    // Synapse shows presence to those who share a room.
    let (names, _room) = room_of(&core_runtime, &["ffi-presence", "ffi-presence-bob"]);
    let stranger = unique_name("ffi-presence-stranger");
    core_runtime.block_on(register(&stranger)).unwrap();
    let (alice, bob) = (sign_in(&names[0]), sign_in(&names[1]));
    let alice_id = alice.session.user_id();

    block_on(
        alice
            .session
            .set_presence(PresenceState::Online, Some("In a meeting".into())),
    )
    .unwrap();
    let seen = block_on(bob.session.presence(alice_id.clone())).unwrap();
    assert_eq!(seen.state, PresenceState::Online);
    assert_eq!(seen.status_message.as_deref(), Some("In a meeting"));
    assert!(
        seen.currently_active && seen.last_active.is_some(),
        "{seen:?}"
    );

    let (listener, updates) = forward::<Presences>();
    let _handle = bob
        .session
        .subscribe_presence(vec![alice_id.clone()], listener)
        .unwrap();
    let state_of = |p: &Presences| p.as_ref().and_then(|p| p.get(&alice_id)).map(|p| p.state);
    wait_for("Alice online", &updates, |p| {
        state_of(p) == Some(PresenceState::Online)
    });

    // Read again within about a minute; the library keeps Alice's presence
    // meanwhile (Synapse drops one not refreshed for 30 s), so it changes
    // to unavailable, not offline.
    block_on(alice.session.set_presence(PresenceState::Unavailable, None)).unwrap();
    let changed = wait_within(
        Duration::from_secs(100),
        "Alice no longer online",
        &updates,
        |p| state_of(p) != Some(PresenceState::Online),
    );
    let changed = &changed.unwrap()[&alice_id];
    assert_eq!(changed.state, PresenceState::Unavailable);
    assert_eq!(changed.status_message, None);

    block_on(alice.session.set_presence(PresenceState::Offline, None)).unwrap();
    let seen = block_on(bob.session.presence(alice_id.clone())).unwrap();
    assert_eq!(seen.state, PresenceState::Offline);

    assert!(matches!(
        block_on(bob.session.presence("alice".into())),
        Err(MtrxError::InvalidInput { .. })
    ));
    assert!(matches!(
        bob.session
            .subscribe_presence(vec!["alice".into()], forward::<Presences>().0),
        Err(MtrxError::InvalidInput { .. })
    ));
    // Someone Bob shares no room with: Synapse refuses theirs, not his
    // own, so presence as a whole stays available.
    let server = bob.session.user_id().split_once(':').unwrap().1.to_owned();
    let stranger_id = format!("@{stranger}:{server}");
    match block_on(bob.session.presence(stranger_id)) {
        Err(MtrxError::Other { reason }) => {
            assert_ne!(reason, "This server does not share presence");
            assert!(reason.contains("M_FORBIDDEN"), "{reason}");
        }
        other => panic!("a stranger's presence: {other:?}"),
    }
}
