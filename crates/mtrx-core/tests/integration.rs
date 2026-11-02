//! End-to-end tests against the local Synapse from `testserver/`.
//!
//! ```sh
//! testserver/run.sh
//! cargo test -p mtrx-core --test integration -- --ignored
//! ```
//!
//! Each test registers fresh users, so tests are independent and can run in
//! parallel against a long-lived server.

#![recursion_limit = "256"]

use std::{
    future::Future,
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, bail};
use futures_util::{Stream, StreamExt};
use mtrx_core::{
    Access, Activity, Attachment, CallService, Connection, DirectoryKind, Draft, EventContent,
    History, JoinAccess, Membership, MembershipChange, MessageAction, NewRoom, NotifyMode,
    Permissions, PinsChange, PresenceState, Profile, RecoveryStatus, Role, RoomKind, SecretStore,
    Session, TimelineEntry, VerificationState,
    test_support::{self, PASSWORD, homeserver, unique_name},
};
use mtrx_format::{Block, Inline, Style};
use tempfile::TempDir;

const TIMEOUT: Duration = Duration::from_secs(30);

async fn register(name: &str) -> Result<()> {
    test_support::register(name)
        .await
        .map_err(anyhow::Error::msg)
}

/// A logged-in device with its own data directory.
struct Device {
    session: Session,
    _dir: TempDir,
}

async fn login(user: &str) -> Result<Device> {
    let dir = TempDir::new()?;
    let profile = Profile::new("test", dir.path(), SecretStore::File)?;
    let session = Session::login(profile, &homeserver(), user, PASSWORD).await?;
    session.start_sync().await;
    Ok(Device { session, _dir: dir })
}

async fn new_user(prefix: &str) -> Result<(String, Device)> {
    let name = unique_name(prefix);
    register(&name).await?;
    let device = login(&name).await?;
    Ok((name, device))
}

async fn timeout<T>(what: &str, future: impl Future<Output = T>) -> Result<T> {
    tokio::time::timeout(TIMEOUT, future)
        .await
        .with_context(|| format!("timed out: {what}"))
}

/// Wait until a stream yields an item matching `pred`.
async fn wait_for<T>(
    what: &str,
    stream: impl Stream<Item = T>,
    mut pred: impl FnMut(&T) -> bool,
) -> Result<T> {
    let mut stream = std::pin::pin!(stream);
    timeout(what, async {
        while let Some(item) = stream.next().await {
            if pred(&item) {
                return Ok(item);
            }
        }
        bail!("stream ended: {what}")
    })
    .await?
}

fn has_message(entries: &[TimelineEntry], body: &str) -> bool {
    entries.iter().any(|e| match e {
        TimelineEntry::Event(ev) => {
            matches!(&ev.content, EventContent::Message(m) if m.body == body)
        }
        _ => false,
    })
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn login_restore_logout() -> Result<()> {
    let name = unique_name("login");
    register(&name).await?;
    let dir = TempDir::new()?;
    let profile = || Profile::new("p", dir.path(), SecretStore::File);

    let session = Session::login(profile()?, &homeserver(), &name, PASSWORD).await?;
    let device = session.device_id();
    drop(session);

    let session = Session::restore(profile()?).await?;
    assert_eq!(session.device_id(), device);
    assert_eq!(session.user_id().localpart(), name);
    session.logout().await?;

    assert!(matches!(
        Session::restore(profile()?).await,
        Err(mtrx_core::Error::NotLoggedIn(_))
    ));
    assert!(!dir.path().join("profiles/p").exists());

    let wrong = Session::login(profile()?, &homeserver(), &name, "wrong").await;
    assert!(wrong.is_err());
    assert!(
        !dir.path().join("profiles/p").exists(),
        "failed login leaves no data"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn room_list_shows_space_hierarchy() -> Result<()> {
    let (_, alice) = new_user("spaces").await?;
    let s = &alice.session;
    let space = s
        .create_room(NewRoom {
            name: Some("Work".into()),
            space: true,
            ..Default::default()
        })
        .await?;
    let room = s
        .create_room(NewRoom {
            name: Some("general".into()),
            encrypted: true,
            ..Default::default()
        })
        .await?;
    s.add_to_space(&space, &room).await?;

    let list = s.room_list().await?;
    let state = wait_for("room in space", list.updates(), |state| {
        state
            .rooms
            .iter()
            .any(|r| r.id == room && r.parents.contains(&space))
    })
    .await?;
    let space_entry = state
        .rooms
        .iter()
        .find(|r| r.id == space)
        .context("space listed")?;
    assert_eq!(space_entry.kind, RoomKind::Space);
    assert_eq!(space_entry.name, "Work");
    let room_entry = state.rooms.iter().find(|r| r.id == room).unwrap();
    assert!(room_entry.encrypted);
    assert_eq!(s.find_room("general").await?, room);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn encrypted_messages_between_users() -> Result<()> {
    let (_, alice) = new_user("alice").await?;
    let (_, bob) = new_user("bob").await?;
    let room = alice
        .session
        .create_room(NewRoom {
            name: Some("secret".into()),
            encrypted: true,
            invite: vec![bob.session.user_id()],
            ..Default::default()
        })
        .await?;

    let bob_list = bob.session.room_list().await?;
    wait_for("bob invited", bob_list.updates(), |s| {
        s.rooms.iter().any(|r| r.id == room && r.invited)
    })
    .await?;
    bob.session
        .join(<&mtrx_core::RoomOrAliasId>::from(&*room), &[])
        .await?;

    // Alice must see bob joined, otherwise the room key is not shared with him.
    let alice_timeline = alice.session.timeline(&room).await?;
    wait_for("bob joined", alice_timeline.updates(), |entries| {
        entries.iter().any(|e| {
            matches!(e, TimelineEntry::Event(ev) if ev.sender == bob.session.user_id()
            && matches!(ev.content, EventContent::Membership {
                change: MembershipChange::InvitationAccepted, ..
            }))
        })
    })
    .await?;

    let event_id = timeout(
        "send",
        alice_timeline
            .send_markdown("hello **bob**", None)
            .await?
            .sent(),
    )
    .await??;

    let bob_timeline = bob.session.timeline(&room).await?;
    let entries = wait_for("bob decrypts", bob_timeline.updates(), |e| {
        has_message(e, "hello **bob**")
    })
    .await?;
    let message = entries
        .iter()
        .find_map(|e| match e {
            TimelineEntry::Event(ev) if ev.event_id.as_ref() == Some(&event_id) => Some(ev.clone()),
            _ => None,
        })
        .context("event by id")?;
    let EventContent::Message(m) = message.content else {
        bail!("not a message")
    };
    assert_eq!(
        m.document.blocks,
        vec![Block::Paragraph(vec![
            Inline::Text("hello ".into()),
            Inline::Styled {
                style: Style::Bold,
                content: vec![Inline::Text("bob".into())]
            },
        ])]
    );
    assert_eq!(message.sender, alice.session.user_id());
    Ok(())
}

/// A `size`×`size` PNG with a colour gradient.
fn test_png(size: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(size, size, |x, y| {
        image::Rgb([(x * 255 / size) as u8, (y * 255 / size) as u8, 160])
    });
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn avatars_in_room_list_and_timeline() -> Result<()> {
    let (_, alice) = new_user("alice").await?;
    let (_, bob) = new_user("bob").await?;
    let avatar = bob.session.set_avatar("image/png", test_png(128)).await?;

    let room = alice
        .session
        .create_room(NewRoom {
            invite: vec![bob.session.user_id()],
            direct: true,
            ..Default::default()
        })
        .await?;
    let bob_list = bob.session.room_list().await?;
    wait_for("bob invited", bob_list.updates(), |s| {
        s.rooms.iter().any(|r| r.id == room && r.invited)
    })
    .await?;
    bob.session
        .join(<&mtrx_core::RoomOrAliasId>::from(&*room), &[])
        .await?;
    let bob_timeline = bob.session.timeline(&room).await?;
    timeout("send", bob_timeline.send_markdown("hi", None).await?.sent()).await??;

    // A DM without a room picture shows the other user's.
    let list = alice.session.room_list().await?;
    wait_for("DM with bob's avatar", list.updates(), |s| {
        s.rooms.iter().any(|r| {
            r.id == room && r.kind == RoomKind::Direct && r.avatar.as_ref() == Some(&avatar)
        })
    })
    .await?;
    let timeline = alice.session.timeline(&room).await?;
    wait_for(
        "message with sender avatar",
        timeline.updates(),
        |entries| {
            entries.iter().any(|e| {
                matches!(e, TimelineEntry::Event(ev)
                if ev.sender == bob.session.user_id() && ev.sender_avatar.as_ref() == Some(&avatar))
            })
        },
    )
    .await?;

    let thumbnail = alice.session.thumbnail(&avatar, 32).await?;
    let thumbnail = image::load_from_memory(&thumbnail)?;
    assert!(thumbnail.width() >= 32 && thumbnail.width() <= 128);
    assert_eq!(thumbnail.width(), thumbnail.height());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn recovery_decrypts_history_on_new_device() -> Result<()> {
    let (name, first) = new_user("recovery").await?;
    let room = first
        .session
        .create_room(NewRoom {
            name: Some("diary".into()),
            encrypted: true,
            ..Default::default()
        })
        .await?;
    let timeline = first.session.timeline(&room).await?;
    timeout(
        "send",
        timeline
            .send_markdown("written before recovery", None)
            .await?
            .sent(),
    )
    .await??;
    let key = first.session.enable_recovery().await?;
    assert_eq!(
        first.session.recovery_status().await,
        RecoveryStatus::Enabled
    );

    let second = login(&name).await?;
    assert!(!second.session.is_device_verified().await?);
    second.session.room_list().await?.wait_loaded().await;
    second.session.recover(&key, |_| {}).await?;
    assert!(second.session.is_device_verified().await?);

    let timeline = second.session.timeline(&room).await?;
    // Room keys come from the backup when decryption first fails.
    let load = async {
        while !timeline.paginate_back(20).await? {}
        anyhow::Ok(())
    };
    timeout("paginate", load).await??;
    wait_for("decrypt from backup", timeline.updates(), |e| {
        has_message(e, "written before recovery")
    })
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn emoji_self_verification() -> Result<()> {
    let (name, old) = new_user("verify").await?;
    let new = login(&name).await?;
    assert!(old.session.is_device_verified().await?);
    assert!(!new.session.is_device_verified().await?);
    // Both devices must know each other's keys before verifying: a request
    // from a device not known yet is dropped.
    for (device, other) in [(&old, &new), (&new, &old)] {
        let other = other.session.device_id();
        timeout("devices know each other", async {
            while !device
                .session
                .devices()
                .await?
                .iter()
                .any(|d| d.id == other)
            {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            anyhow::Ok(())
        })
        .await??;
    }

    let mut incoming = std::pin::pin!(old.session.verification_requests());
    let outgoing = new.session.request_self_verification().await?;
    let accepted = timeout("incoming request", incoming.next())
        .await?
        .context("no request")?;
    accepted.accept().await?;

    let emojis = |state: &VerificationState| matches!(state, VerificationState::Emojis(_));
    let a = wait_for("emojis (new)", outgoing.states(), emojis).await?;
    let b = wait_for("emojis (old)", accepted.states(), emojis).await?;
    assert_eq!(a, b, "both devices show the same emojis");

    outgoing.confirm().await?;
    accepted.confirm().await?;
    let done = |state: &VerificationState| matches!(state, VerificationState::Done);
    wait_for("done (new)", outgoing.states(), done).await?;
    wait_for("done (old)", accepted.states(), done).await?;
    assert!(new.session.is_device_verified().await?);
    Ok(())
}

/// Talks to matrix.org: registers the client with its OAuth provider and
/// checks we get a sign-in page URL, then cancels.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs network access to matrix.org"]
async fn matrix_org_offers_browser_login() -> Result<()> {
    assert!(Session::supports_oauth("matrix.org").await?);
    let dir = TempDir::new()?;
    let profile = Profile::new("oauth", dir.path(), SecretStore::File)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let login = Session::login_oauth(profile, "matrix.org", move |url| {
        let _ = tx.send(url);
    });
    // The login waits for the browser forever; stop once the URL is known.
    let url = tokio::select! {
        url = rx => url?,
        result = login => bail!("login ended early: {:?}", result.err()),
    };
    println!("{url}");
    assert_eq!(url.scheme(), "https");
    assert!(
        url.query_pairs()
            .any(|(k, v)| k == "redirect_uri" && v.starts_with("http://127.0.0.1:"))
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn scheduled_messages() -> Result<()> {
    let name = unique_name("later");
    register(&name).await?;
    let dir = TempDir::new()?;
    let profile = || Profile::new("p", dir.path(), SecretStore::File);
    let session = Session::login(profile()?, &homeserver(), &name, PASSWORD).await?;
    session.start_sync().await;
    let room = session
        .create_room(NewRoom {
            name: Some("later".into()),
            encrypted: true,
            ..Default::default()
        })
        .await?;
    wait_for("room synced", session.room_list().await?.updates(), |s| {
        s.rooms.iter().any(|r| r.id == room)
    })
    .await?;
    let in_secs = |s| SystemTime::now() + Duration::from_secs(s);

    assert!(matches!(
        session
            .schedule(&room, "past", Vec::new(), SystemTime::now(), None)
            .await,
        Err(mtrx_core::Error::SendTimeInPast)
    ));
    // Files are kept with the message until it is sent.
    let file = Attachment {
        filename: "late.txt".into(),
        mimetype: "text/plain".into(),
        bytes: b"sent late".to_vec(),
    };
    let late = session
        .schedule(&room, "late", vec![file], in_secs(1), None)
        .await?;
    let cancelled = session
        .schedule(&room, "cancelled", Vec::new(), in_secs(3600), None)
        .await?;
    session.unschedule(&cancelled.id).await?;
    session.stop_sync().await;
    drop(session);

    // "late" passes its time while no scheduler runs.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let session = Session::restore(profile()?).await?;
    assert_eq!(session.scheduled(), vec![late.clone()], "persisted");
    session.start_sync().await;
    let on_time = session
        .schedule(&room, "on **time**", Vec::new(), in_secs(2), None)
        .await?;
    session.start_scheduler_for_test(Duration::from_secs(1));

    let timeline = session.timeline(&room).await?;
    wait_for("sent when due", timeline.updates(), |e| {
        has_message(e, "on **time**")
    })
    .await?;
    assert!(SystemTime::now() >= on_time.send_at);
    let held = wait_for("late one held", session.scheduled_updates(), |s| {
        s.iter().all(|m| m.overdue)
    })
    .await?;
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].id, late.id);
    assert!(!has_message(&timeline.current(), "late"));

    // Editing the text keeps it overdue; a new time makes it wait again.
    session.edit_scheduled(&late.id, "late, edited").await?;
    let edited = &session.scheduled()[0];
    assert_eq!(
        (edited.text.as_str(), edited.overdue),
        ("late, edited", true)
    );
    session.reschedule(&late.id, in_secs(3600)).await?;
    let waiting = &session.scheduled()[0];
    assert_eq!(
        (waiting.text.as_str(), waiting.overdue),
        ("late, edited", false)
    );

    let pending = session.send_scheduled_now(&late.id).await?;
    timeout("rescheduled one sent now", pending.sent()).await??;
    let entries = wait_for("edited text and file sent", timeline.updates(), |e| {
        has_message(e, "late, edited") && find_media(e, "late.txt").is_some()
    })
    .await?;
    let file = find_media(&entries, "late.txt").unwrap();
    assert_eq!(session.media_content(&file.source).await?, b"sent late");
    assert!(session.scheduled().is_empty());
    Ok(())
}

/// A pinned message shows in the room list, in a timeline of its own and
/// as a notice; unpinning takes it out again.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn pinned_messages() -> Result<()> {
    let (_, alice) = new_user("pins").await?;
    let s = &alice.session;
    let room = s
        .create_room(NewRoom {
            name: Some("pins".into()),
            encrypted: true,
            ..Default::default()
        })
        .await?;
    let timeline = s.timeline(&room).await?;
    let plan = timeout(
        "send",
        timeline.send_markdown("The plan", None).await?.sent(),
    )
    .await??;
    assert!(s.can_pin(&room).await?);
    s.set_pinned(&room, &plan, true).await?;
    wait_for(
        "pinned in the list",
        s.room_list().await?.updates(),
        |state| (state.rooms.iter()).any(|r| r.id == room && r.pinned == [plan.clone()]),
    )
    .await?;
    let pinned = s.pinned_timeline(&room).await?;
    wait_for("pinned message loaded", pinned.updates(), |entries| {
        has_message(entries, "The plan")
    })
    .await?;
    let notice = EventContent::Pins(PinsChange {
        pinned: 1,
        unpinned: 0,
    });
    wait_for("notice", timeline.updates(), |entries| {
        entries
            .iter()
            .any(|e| matches!(e, TimelineEntry::Event(ev) if ev.content == notice))
    })
    .await?;

    s.set_pinned(&room, &plan, false).await?;
    wait_for("unpinned", s.room_list().await?.updates(), |state| {
        (state.rooms.iter()).any(|r| r.id == room && r.pinned.is_empty())
    })
    .await?;
    wait_for("gone from the pinned", pinned.updates(), |entries| {
        !has_message(entries, "The plan")
    })
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn space_call_services() -> Result<()> {
    let (_, alice) = new_user("calls").await?;
    let (_, bob) = new_user("calls").await?;
    let s = &alice.session;
    let space = s
        .create_room(NewRoom {
            name: Some("Lab".into()),
            space: true,
            ..Default::default()
        })
        .await?;
    let room = s
        .create_room(NewRoom {
            name: Some("meetings".into()),
            invite: vec![bob.session.user_id()],
            permissions: Permissions::Moderators,
            ..Default::default()
        })
        .await?;
    // Names come with the rooms' state, which includes the power levels.
    wait_for("rooms synced", s.room_list().await?.updates(), |state| {
        [(&space, "Lab"), (&room, "meetings")]
            .iter()
            .all(|(id, name)| state.rooms.iter().any(|r| &r.id == *id && r.name == *name))
    })
    .await?;

    assert!(s.can_change_topic(&room).await?);

    let service = CallService::jitsi("Lab Jitsi", "https://video.lab.example")?;
    s.set_space_call_service(&space, Some(service.clone()))
        .await?;
    assert_eq!(s.space_call_service(&space).await?, Some(service));

    // In a moderated room, members cannot change room state.
    let bob_list = bob.session.room_list().await?;
    wait_for("bob invited", bob_list.updates(), |s| {
        s.rooms.iter().any(|r| r.id == room)
    })
    .await?;
    bob.session
        .join(<&mtrx_core::RoomOrAliasId>::from(&*room), &[])
        .await?;
    wait_for("bob joined", bob_list.updates(), |s| {
        s.rooms
            .iter()
            .any(|r| r.id == room && !r.invited && r.name == "meetings")
    })
    .await?;
    // Power levels arrive with the joined room's state, after the join.
    let can_set = timeout("power levels synced", async {
        loop {
            match bob.session.can_change_topic(&room).await {
                Ok(can_set) => return can_set,
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    })
    .await?;
    assert!(!can_set);
    Ok(())
}

/// Join `room` as `user` and wait until its state (with the power levels) synced.
async fn join_synced(user: &Device, room: &mtrx_core::OwnedRoomId, name: &str) -> Result<()> {
    let list = user.session.room_list().await?;
    wait_for("invited", list.updates(), |s| {
        s.rooms.iter().any(|r| &r.id == room)
    })
    .await?;
    user.session
        .join(<&mtrx_core::RoomOrAliasId>::from(&**room), &[])
        .await?;
    wait_for("joined", list.updates(), |s| {
        s.rooms
            .iter()
            .any(|r| &r.id == room && !r.invited && r.name == name)
    })
    .await?;
    timeout("power levels synced", async {
        while user.session.permissions(room).await.is_err() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn everyone_can_edit() -> Result<()> {
    let (_, alice) = new_user("perm").await?;
    let (_, bob) = new_user("perm").await?;
    let (_, carol) = new_user("perm").await?;
    let s = &alice.session;
    let new_room = |name: &str, space, permissions| NewRoom {
        name: Some(name.into()),
        space,
        invite: vec![bob.session.user_id()],
        permissions,
        ..Default::default()
    };

    // New rooms are open: bob may rename and invite, but not change permissions.
    let open = s
        .create_room(new_room("open", false, Permissions::Everyone))
        .await?;
    join_synced(&bob, &open, "open").await?;
    assert_eq!(bob.session.permissions(&open).await?, Permissions::Everyone);
    assert!(bob.session.can_change_topic(&open).await?);
    assert!(!bob.session.can_change_permissions(&open).await?);
    bob.session.invite(&open, &carol.session.user_id()).await?;

    // A moderated space with a moderated room, opened by alice in one go.
    let space = s
        .create_room(new_room("team", true, Permissions::Moderators))
        .await?;
    let room = s
        .create_room(new_room("closed", false, Permissions::Moderators))
        .await?;
    s.add_to_space(&space, &room).await?;
    join_synced(&bob, &room, "closed").await?;
    assert_eq!(
        bob.session.permissions(&room).await?,
        Permissions::Moderators
    );
    assert!(!bob.session.can_change_topic(&room).await?);
    wait_for("space synced", s.room_list().await?.updates(), |state| {
        state
            .rooms
            .iter()
            .any(|r| r.id == room && r.parents.contains(&space))
    })
    .await?;

    let update = s.let_everyone_edit_space(&space).await?;
    let mut changed = update.changed.clone();
    changed.sort();
    let mut expected = vec![space.clone(), room.clone()];
    expected.sort();
    assert_eq!(changed, expected);
    assert!(update.not_allowed.is_empty());

    timeout("bob sees the change", async {
        while bob.session.permissions(&room).await.ok() != Some(Permissions::Everyone) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await?;
    assert!(bob.session.can_change_topic(&room).await?);
    assert!(!bob.session.can_change_permissions(&room).await?);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn room_management() -> Result<()> {
    let (_, alice) = new_user("manage").await?;
    let (bob_name, bob) = new_user("manage").await?;
    let s = &alice.session;
    let space = s
        .create_room(NewRoom {
            name: Some("Lab".into()),
            space: true,
            ..Default::default()
        })
        .await?;
    let room = s
        .create_room(NewRoom {
            name: Some("papers".into()),
            parent: Some(space.clone()),
            ..Default::default()
        })
        .await?;
    let list = s.room_list().await?;
    wait_for("room in space", list.updates(), |state| {
        state
            .rooms
            .iter()
            .any(|r| r.id == room && r.parents.contains(&space))
    })
    .await?;

    s.set_room_name(&room, "reading list").await?;
    s.set_room_topic(&room, "Papers to read").await?;
    wait_for("renamed", list.updates(), |state| {
        state.rooms.iter().any(|r| {
            r.id == room && r.name == "reading list" && r.topic.as_deref() == Some("Papers to read")
        })
    })
    .await?;

    let dm = s.direct_chat(&bob.session.user_id()).await?;
    wait_for("dm listed", list.updates(), |state| {
        state
            .rooms
            .iter()
            .any(|r| r.id == dm && r.kind == RoomKind::Direct)
    })
    .await?;
    assert_eq!(s.direct_chat(&bob.session.user_id()).await?, dm, "reused");

    // The directory lists people sharing a room with us, once both joined;
    // it updates in the background.
    bob.session
        .join(<&mtrx_core::RoomOrAliasId>::from(&*dm), &[])
        .await?;
    timeout("bob in the user directory", async {
        loop {
            let found = s.search_users(&bob_name, 10).await?;
            if found.iter().any(|u| u.id == bob.session.user_id()) {
                return anyhow::Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await??;

    s.leave(&room).await?;
    wait_for("left", list.updates(), |state| {
        !state.rooms.iter().any(|r| r.id == room)
    })
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn co_owners() -> Result<()> {
    let (_, alice) = new_user("owners").await?;
    let (_, bob) = new_user("owners").await?;
    let s = &alice.session;
    assert!(s.offers_owners().await);
    let bob_id = bob.session.user_id();
    let room = s
        .create_room(NewRoom {
            name: Some("shared".into()),
            co_owners: vec![bob_id.clone()],
            ..Default::default()
        })
        .await?;
    join_synced(&bob, &room, "shared").await?;
    // Both owners: above every level, neither changed by the other.
    let members = timeout("bob joined", async {
        loop {
            let members = s.members(&room).await?;
            if members.iter().any(|m| m.id == bob_id && !m.invited) {
                return anyhow::Ok(members);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await??;
    assert!(
        members
            .iter()
            .all(|m| m.level.is_none() && m.manage.up_to.is_none())
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn room_settings_and_space_links() -> Result<()> {
    let (_, alice) = new_user("settings").await?;
    let s = &alice.session;
    let space = |name: &str| NewRoom {
        name: Some(name.into()),
        space: true,
        ..Default::default()
    };
    let first = s.create_room(space("First")).await?;
    let second = s.create_room(space("Second")).await?;
    let room = s
        .create_room(NewRoom {
            name: Some("moving".into()),
            parent: Some(first.clone()),
            ..Default::default()
        })
        .await?;
    // The settings once sync brings back what `done` waits for.
    let settled = |what: &'static str, done: fn(&mtrx_core::RoomSettings) -> bool| {
        let room = room.clone();
        timeout(what, async move {
            loop {
                if let Ok(settings) = s.room_settings(&room).await
                    && done(&settings)
                {
                    return anyhow::Ok(settings);
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
    };
    let settings = settled("in the first space", |s| s.spaces.len() == 1).await??;
    assert_eq!(settings.version.as_deref(), Some("12"));
    assert_eq!(settings.spaces[0].space, first);
    assert!(settings.spaces[0].can_change && !settings.spaces[0].suggested);
    assert_eq!(settings.listed, Some(false));
    assert_eq!(settings.permissions, Permissions::Everyone);

    // Each change shows in the settings read straight after.
    s.set_suggested(&first, &room, true).await?;
    assert!(s.room_settings(&room).await?.spaces[0].suggested);
    // Moved: added to the second, out of the first.
    s.add_to_space(&second, &room).await?;
    s.remove_from_space(&first, &room).await?;
    let moved = s.room_settings(&room).await?;
    assert_eq!(
        (
            moved.spaces[0].space.clone(),
            moved.spaces.len(),
            moved.can_add_to
        ),
        (second, 1, vec![first])
    );

    let name = unique_name("moving");
    s.set_address(&room, Some(&name)).await?;
    let addressed = s.room_settings(&room).await?;
    assert_eq!(addressed.address.context("address")?.alias(), name);
    s.set_listed(&room, true).await?;
    s.set_history(&room, History::Joined).await?;
    s.set_permissions(&room, Permissions::Moderators).await?;
    let changed = s.room_settings(&room).await?;
    assert_eq!(
        (changed.listed, changed.history, changed.permissions),
        (Some(true), History::Joined, Permissions::Moderators)
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn browse_and_join_space_rooms() -> Result<()> {
    let (_, alice) = new_user("browse").await?;
    let (_, bob) = new_user("browse").await?;
    let s = &alice.session;
    let space = s
        .create_room(NewRoom {
            name: Some("Institute".into()),
            space: true,
            ..Default::default()
        })
        .await?;
    let open = s
        .create_room(NewRoom {
            name: Some("coffee".into()),
            parent: Some(space.clone()),
            ..Default::default()
        })
        .await?;
    let list = s.room_list().await?;
    wait_for("room in space", list.updates(), |state| {
        state
            .rooms
            .iter()
            .any(|r| r.id == open && r.parents.contains(&space))
    })
    .await?;
    s.set_room_access(&open, Access::SpaceMembers).await?;
    let asking = s
        .create_room(NewRoom {
            name: Some("thesis".into()),
            parent: Some(space.clone()),
            ..Default::default()
        })
        .await?;
    s.set_room_access(&asking, Access::Knock).await?;
    s.invite(&space, &bob.session.user_id()).await?;
    join_synced(&bob, &space, "Institute").await?;

    let b = &bob.session;
    let children = b.space_children(&space).await?;
    let coffee = children
        .iter()
        .find(|r| r.id == open)
        .context("coffee listed")?;
    assert_eq!(coffee.access, JoinAccess::SpaceMembers);
    assert_eq!(coffee.membership, None);
    assert_eq!(coffee.role, None);
    let thesis = children
        .iter()
        .find(|r| r.id == asking)
        .context("thesis listed")?;
    assert_eq!(thesis.access, JoinAccess::Knock);
    b.knock(&asking.clone().into(), thesis.via.clone()).await?;
    assert!(!coffee.via.is_empty());
    let preview = b.preview(&open.clone().into(), coffee.via.clone()).await?;
    assert_eq!(preview.name, "coffee");
    assert_eq!(preview.membership, None);

    b.join(<&mtrx_core::RoomOrAliasId>::from(&*open), &coffee.via)
        .await?;
    timeout("membership synced", async {
        loop {
            let children = b.space_children(&space).await?;
            let membership = |id| {
                children
                    .iter()
                    .find(|r| &r.id == id)
                    .and_then(|r| r.membership)
            };
            if membership(&open) == Some(Membership::Joined)
                && membership(&asking) == Some(Membership::Asked)
            {
                return anyhow::Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await??;

    // The admin may manage bob, who may manage no one.
    let bob_id = b.user_id();
    let found = timeout("bob in coffee", async {
        loop {
            let members = s.members(&open).await?;
            if let Some(bob) = members.into_iter().find(|m| m.id == bob_id && !m.invited) {
                return anyhow::Ok(bob);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await??;
    assert!(found.manage.remove && found.manage.ban);
    // Rooms are created at version 12: their creator is above all levels,
    // gives any and keeps theirs.
    assert_eq!(found.manage.up_to, Some(9007199254740991));
    let us = s.members(&open).await?;
    let us = us.iter().find(|m| m.id == s.user_id()).unwrap();
    assert_eq!((us.level, us.manage.up_to), (None, None));
    assert!(
        b.members(&open)
            .await?
            .iter()
            .all(|m| m.manage == Default::default())
    );
    s.set_level(&open, &bob_id, Role::Moderator.level(0))
        .await?;
    s.remove_member(&open, &bob_id, Some("tidying up"), false)
        .await?;
    // Our own server's directory: nothing published, but it answers.
    b.public_rooms(None, "", DirectoryKind::All, None).await?;
    Ok(())
}

/// The message of `entries` whose body is `body`.
fn find_message<'a>(entries: &'a [TimelineEntry], body: &str) -> Option<&'a mtrx_core::Event> {
    entries.iter().find_map(|e| match e {
        TimelineEntry::Event(ev) if matches!(&ev.content, EventContent::Message(m) if m.body == body) => {
            Some(ev)
        }
        _ => None,
    })
}

fn message(event: &mtrx_core::Event) -> &mtrx_core::Message {
    match &event.content {
        EventContent::Message(m) => m,
        _ => panic!("not a message"),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn replies_reactions_edits_typing_receipts() -> Result<()> {
    let (_, alice) = new_user("talk").await?;
    let (_, bob) = new_user("talk").await?;
    let room = alice
        .session
        .create_room(NewRoom {
            name: Some("talk".into()),
            encrypted: true,
            invite: vec![bob.session.user_id()],
            ..Default::default()
        })
        .await?;
    join_synced(&bob, &room, "talk").await?;
    let alice_timeline = alice.session.timeline(&room).await?;
    wait_for("bob joined", alice_timeline.updates(), |entries| {
        entries
            .iter()
            .any(|e| matches!(e, TimelineEntry::Event(ev) if ev.sender == bob.session.user_id()))
    })
    .await?;
    let bob_timeline = bob.session.timeline(&room).await?;

    let question = timeout(
        "send",
        bob_timeline
            .send_markdown("Which dataset?", None)
            .await?
            .sent(),
    )
    .await??;
    wait_for("question arrives", alice_timeline.updates(), |e| {
        has_message(e, "Which dataset?")
    })
    .await?;

    // Reply.
    timeout(
        "reply",
        alice_timeline
            .send_markdown("The **new** one", Some(&question))
            .await?
            .sent(),
    )
    .await??;
    let entries = wait_for("bob sees the reply", bob_timeline.updates(), |e| {
        find_message(e, "The **new** one").is_some_and(|ev| {
            message(ev)
                .in_reply_to
                .as_ref()
                .is_some_and(|r| r.preview.is_some())
        })
    })
    .await?;
    let reply = message(find_message(&entries, "The **new** one").unwrap())
        .in_reply_to
        .clone()
        .unwrap();
    assert_eq!(reply.event_id, question);
    assert_eq!(reply.preview.as_deref(), Some("Which dataset?"));
    assert_eq!(reply.sender, Some(bob.session.user_id()));
    assert_eq!(reply.kind, Some(mtrx_core::MessageKind::Text));

    // Reaction, seen as ours by alice and as someone else's by bob.
    let entries = alice_timeline.current();
    let question_key = find_message(&entries, "Which dataset?")
        .unwrap()
        .key
        .clone();
    alice_timeline.toggle_reaction(&question_key, "👍").await?;
    // With who reacted, named.
    let alice_id = alice.session.user_id();
    wait_for("bob sees the reaction", bob_timeline.updates(), |e| {
        find_message(e, "Which dataset?").is_some_and(|ev| {
            message(ev).reactions
                == [mtrx_core::Reaction {
                    key: "👍".into(),
                    count: 1,
                    own: false,
                    by: vec![mtrx_core::Reader {
                        id: alice_id.clone(),
                        name: Some(alice_id.localpart().to_owned()),
                        avatar: None,
                    }],
                }]
        })
    })
    .await?;
    alice_timeline.toggle_reaction(&question_key, "👍").await?;
    wait_for("reaction removed", bob_timeline.updates(), |e| {
        find_message(e, "Which dataset?").is_some_and(|ev| message(ev).reactions.is_empty())
    })
    .await?;

    // Edit and delete our own message.
    let entries = alice_timeline.current();
    let own = find_message(&entries, "The **new** one").unwrap();
    assert!(own.editable);
    assert!(!find_message(&entries, "Which dataset?").unwrap().editable);
    let own_key = own.key.clone();
    alice_timeline
        .edit_markdown(&own_key, "The *newest* one")
        .await?;
    wait_for("bob sees the edit", bob_timeline.updates(), |e| {
        find_message(e, "The *newest* one").is_some_and(|ev| message(ev).edited)
    })
    .await?;
    let entries = alice_timeline.current();
    let own_key = find_message(&entries, "The *newest* one")
        .unwrap()
        .key
        .clone();
    alice_timeline.redact(&own_key).await?;
    wait_for("bob sees it deleted", bob_timeline.updates(), |e| {
        !has_message(e, "The *newest* one")
            && e.iter().any(|x| matches!(x, TimelineEntry::Event(ev) if matches!(ev.content, EventContent::Redacted)))
    })
    .await?;

    // Typing.
    let typing = bob_timeline.typing_updates();
    alice_timeline.typing(true).await?;
    wait_for("bob sees alice typing", typing, |names| names.len() == 1).await?;

    // Read receipts: bob read up to the latest event, shown with his name.
    bob_timeline.mark_read().await?;
    let bob_id = bob.session.user_id();
    wait_for("alice sees bob's receipt", alice_timeline.updates(), |e| {
        e.iter().any(|x| {
            matches!(x, TimelineEntry::Event(ev)
                if ev.read_by.iter().any(|r| r.id == bob_id && r.name.as_deref() == Some(bob_id.localpart())))
        })
    })
    .await?;
    Ok(())
}

/// The original of a reply far back in the room is fetched; one that
/// cannot be fetched shows as such.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn replies_to_unloaded_messages() -> Result<()> {
    let (name, alice) = new_user("quote").await?;
    let room = alice
        .session
        .create_room(NewRoom {
            name: Some("quote".into()),
            ..Default::default()
        })
        .await?;
    let timeline = alice.session.timeline(&room).await?;
    let send = |text: String, reply_to: Option<mtrx_core::OwnedEventId>| {
        let timeline = &timeline;
        async move {
            let pending = timeline.send_markdown(&text, reply_to.as_deref()).await?;
            anyhow::Ok(timeout("send", pending.sent()).await??)
        }
    };
    let original = send("Far back".into(), None).await?;
    for i in 0..60 {
        send(format!("Filler {i}"), None).await?;
    }
    send("Answer".into(), Some(original.clone())).await?;
    let missing: mtrx_core::OwnedEventId =
        format!("$missing:{}", alice.session.user_id().server_name())
            .try_into()
            .unwrap();
    send("Answer to nothing".into(), Some(missing)).await?;

    // A new device loads only the latest messages.
    let second = login(&name).await?;
    let list = second.session.room_list().await?;
    wait_for("room synced", list.updates(), |s| {
        s.rooms.iter().any(|r| r.id == room)
    })
    .await?;
    let timeline = second.session.timeline(&room).await?;
    let reply = |entries: &[TimelineEntry], body| {
        find_message(entries, body).and_then(|ev| message(ev).in_reply_to.clone())
    };
    // Back to the answer, not as far as the original.
    timeline.paginate_back(5).await?;
    let entries = wait_for("both quotes settled", timeline.updates(), |e| {
        reply(e, "Answer").is_some_and(|r| r.preview.is_some())
            && reply(e, "Answer to nothing").is_some_and(|r| r.failed)
    })
    .await?;
    assert!(!has_message(&entries, "Far back"));
    assert_eq!(
        reply(&entries, "Answer").unwrap().preview.as_deref(),
        Some("Far back")
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn members_and_notifications() -> Result<()> {
    let (_, alice) = new_user("notify").await?;
    let (_, bob) = new_user("notify").await?;
    let (_, carol) = new_user("notify").await?;
    let room = alice
        .session
        .create_room(NewRoom {
            name: Some("lab".into()),
            encrypted: true,
            invite: vec![bob.session.user_id(), carol.session.user_id()],
            ..Default::default()
        })
        .await?;
    join_synced(&bob, &room, "lab").await?;

    let members = alice.session.members(&room).await?;
    let role = |user: &Device| {
        members
            .iter()
            .find(|m| m.id == user.session.user_id())
            .map(|m| (m.role, m.invited))
    };
    assert_eq!(role(&alice), Some((mtrx_core::Role::Admin, false)));
    assert_eq!(role(&bob), Some((mtrx_core::Role::Member, false)));
    assert_eq!(role(&carol), Some((mtrx_core::Role::Member, true)));
    assert_eq!(
        members.last().map(|m| m.id.clone()),
        Some(carol.session.user_id())
    );

    // Bob's message notifies alice (decrypted), not bob himself.
    let alice_timeline = alice.session.timeline(&room).await?;
    wait_for("bob joined", alice_timeline.updates(), |entries| {
        entries
            .iter()
            .any(|e| matches!(e, TimelineEntry::Event(ev) if ev.sender == bob.session.user_id()))
    })
    .await?;
    let notifications = alice.session.notifications();
    let bob_timeline = bob.session.timeline(&room).await?;
    timeout(
        "send",
        bob_timeline
            .send_markdown("Results are in", None)
            .await?
            .sent(),
    )
    .await??;
    let notification = wait_for("notified", notifications, |n| n.room_id == room).await?;
    assert_eq!(notification.body, "Results are in");
    assert_eq!(notification.room_name, "lab");
    assert!(!notification.highlight);

    // A mention (`m.mentions`) highlights it; a mention of someone else
    // does not, even with alice's name in the text.
    let alice_id = alice.session.user_id();
    for (text, highlight) in [
        (format!("Thanks {alice_id}!"), true),
        (
            format!(
                "{} has it, says {}",
                carol.session.user_id(),
                alice_id.localpart()
            ),
            false,
        ),
    ] {
        let notifications = alice.session.notifications();
        timeout(
            "send",
            bob_timeline.send_markdown(&text, None).await?.sent(),
        )
        .await??;
        let notification = wait_for("notified", notifications, |n| n.body == text).await?;
        assert_eq!(notification.highlight, highlight, "{text}");
    }

    // Bob reads up to carol's join: alice sees him on the message before
    // it, not on the join, which shows without readers.
    join_synced(&carol, &room, "lab").await?;
    let carol_id = carol.session.user_id();
    let is_carols_join = |ev: &mtrx_core::Event| {
        matches!(
            &ev.content,
            EventContent::Membership {
                user,
                change: MembershipChange::InvitationAccepted,
                ..
            } if *user == carol_id
        )
    };
    wait_for("bob sees carol join", bob_timeline.updates(), |e| {
        e.iter()
            .any(|x| matches!(x, TimelineEntry::Event(ev) if is_carols_join(ev)))
    })
    .await?;
    bob_timeline.mark_read().await?;
    let bob_id = bob.session.user_id();
    let last = format!(
        "{} has it, says {}",
        carol.session.user_id(),
        alice_id.localpart()
    );
    wait_for(
        "bob's receipt on his message",
        alice_timeline.updates(),
        |e| {
            let joined = e
                .iter()
                .any(|x| matches!(x, TimelineEntry::Event(ev) if is_carols_join(ev)));
            let on_message = e.iter().any(|x| {
                matches!(x, TimelineEntry::Event(ev)
                if matches!(&ev.content, EventContent::Message(m) if m.body == last)
                    && ev.read_by.iter().any(|r| r.id == bob_id))
            });
            joined && on_message
        },
    )
    .await?;
    let entries = alice_timeline.current();
    assert!(
        entries.iter().all(|x| match x {
            TimelineEntry::Event(ev) => !ev.is_state() || ev.read_by.is_empty(),
            _ => true,
        }),
        "no readers on state events"
    );
    Ok(())
}

async fn send_text(
    timeline: &mtrx_core::RoomTimeline,
    text: &str,
) -> Result<mtrx_core::OwnedEventId> {
    Ok(timeout("send", timeline.send_markdown(text, None).await?.sent()).await??)
}

/// Collect notifications until one came with each of `bodies`, and those
/// still on their way then (such as a second copy).
async fn collect_notifications(
    what: &str,
    notifications: impl Stream<Item = mtrx_core::Notification>,
    bodies: &[String],
) -> Result<Vec<mtrx_core::Notification>> {
    let mut notifications = std::pin::pin!(notifications);
    let mut got: Vec<mtrx_core::Notification> = Vec::new();
    timeout(what, async {
        while !bodies.iter().all(|b| got.iter().any(|n| &n.body == b)) {
            match notifications.next().await {
                Some(n) => got.push(n),
                None => break,
            }
        }
    })
    .await?;
    let _ = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(n) = notifications.next().await {
            got.push(n);
        }
    })
    .await;
    Ok(got)
}

/// Bob is notified of messages in rooms whose messages he never loaded (he
/// opens no timeline), encrypted or not: live, after being offline while a
/// room got several messages, and for rooms beyond the room list's first
/// rooms. Once each, as the rooms' notify modes say, and not for his own.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn notifications_for_rooms_never_opened() -> Result<()> {
    let (_, alice) = new_user("unopened").await?;
    let (_, bob) = new_user("unopened").await?;
    alice.session.set_display_name("Alice Smith").await?;
    let bob_id = bob.session.user_id();
    let joined = |entries: &Vec<TimelineEntry>| {
        entries
            .iter()
            .any(|e| matches!(e, TimelineEntry::Event(ev) if ev.sender == bob_id))
    };

    // Mentions only: in the encrypted room a mention is told apart from
    // other messages only once decrypted.
    let mut rooms = Vec::new();
    for (name, encrypted) in [("secret", true), ("plain", false)] {
        let room = alice
            .session
            .create_room(NewRoom {
                name: Some(name.into()),
                encrypted,
                invite: vec![bob_id.clone()],
                ..Default::default()
            })
            .await?;
        join_synced(&bob, &room, name).await?;
        bob.session
            .set_notify_mode(&room, NotifyMode::Mentions)
            .await?;
        let timeline = alice.session.timeline(&room).await?;
        wait_for("bob joined", timeline.updates(), joined).await?;
        rooms.push((name, room, timeline));
    }
    let direct = alice.session.direct_chat(&bob_id).await?;
    join_synced(&bob, &direct, "Alice Smith").await?;
    let direct_timeline = alice.session.timeline(&direct).await?;
    wait_for("bob joined", direct_timeline.updates(), joined).await?;

    // Live.
    for (name, room, timeline) in &rooms {
        let notifications = bob.session.notifications();
        let text = format!("{name}: over to you {bob_id}");
        send_text(timeline, &text).await?;
        let n = wait_for("notified of a mention", notifications, |n| n.body == text).await?;
        assert_eq!(&n.room_id, room);
        assert_eq!(n.room_name, *name);
        assert_eq!(n.sender_name, "Alice Smith");
        assert!(n.highlight);
    }
    let notifications = bob.session.notifications();
    send_text(&direct_timeline, "Lunch?").await?;
    let n = wait_for("notified of a direct message", notifications, |n| {
        n.body == "Lunch?"
    })
    .await?;
    assert_eq!(n.room_id, direct);
    assert_eq!(n.sender_name, "Alice Smith");
    assert!(n.direct);

    // More rooms than the room list syncs first (20).
    let mut others = Vec::new();
    for i in 0..22 {
        let room = alice
            .session
            .create_room(NewRoom {
                name: Some(format!("other {i}")),
                invite: vec![bob_id.clone()],
                ..Default::default()
            })
            .await?;
        bob.session
            .join(<&mtrx_core::RoomOrAliasId>::from(&*room), &[])
            .await?;
        others.push(room);
    }
    let bob_list = bob.session.room_list().await?;
    wait_for("bob in all rooms", bob_list.updates(), |s| {
        others
            .iter()
            .all(|room| s.rooms.iter().any(|r| &r.id == room && !r.invited))
    })
    .await?;

    // Offline while each room gets a mention and then other messages, and
    // the other rooms get newer ones.
    bob.session.stop_sync().await;
    let mentions: Vec<String> = rooms
        .iter()
        .map(|(name, ..)| format!("{name}: back to you {bob_id}"))
        .collect();
    for ((name, _, timeline), mention) in rooms.iter().zip(&mentions) {
        send_text(timeline, mention).await?;
        send_text(timeline, &format!("{name}: something else")).await?;
        send_text(timeline, &format!("{name}: and more")).await?;
    }
    for room in &others {
        send_text(&alice.session.timeline(room).await?, "news").await?;
    }
    let notifications = bob.session.notifications();
    bob.session.start_sync().await;
    let got =
        collect_notifications("notified after being offline", notifications, &mentions).await?;
    for ((name, room, _), mention) in rooms.iter().zip(&mentions) {
        let of_room: Vec<_> = got.iter().filter(|n| &n.room_id == room).collect();
        assert_eq!(of_room.len(), 1, "{name}: the mention, once: {of_room:?}");
        assert_eq!(&of_room[0].body, mention);
        assert_eq!(of_room[0].room_name, *name);
        assert_eq!(of_room[0].sender_name, "Alice Smith");
    }

    // Not for his own messages.
    let (_, plain, alice_plain) = &rooms[1];
    let notifications = bob.session.notifications();
    send_text(
        &bob.session.timeline(plain).await?,
        &format!("note to self {bob_id}"),
    )
    .await?;
    send_text(alice_plain, &format!("last one {bob_id}")).await?;
    let n = wait_for("notified", notifications, |n| &n.room_id == plain).await?;
    assert!(n.body.starts_with("last one"), "{}", n.body);
    Ok(())
}

/// The file of the message of `entries` named `filename`.
fn find_media<'a>(entries: &'a [TimelineEntry], filename: &str) -> Option<&'a mtrx_core::Media> {
    entries.iter().find_map(|e| match e {
        TimelineEntry::Event(ev) => match &ev.content {
            EventContent::Message(m) => m.media.as_ref().filter(|m| m.filename == filename),
            _ => None,
        },
        _ => None,
    })
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn images_and_files() -> Result<()> {
    let (_, alice) = new_user("media").await?;
    let (_, bob) = new_user("media").await?;
    let room = alice
        .session
        .create_room(NewRoom {
            name: Some("media".into()),
            encrypted: true,
            invite: vec![bob.session.user_id()],
            ..Default::default()
        })
        .await?;
    join_synced(&bob, &room, "media").await?;
    let alice_timeline = alice.session.timeline(&room).await?;
    wait_for("bob joined", alice_timeline.updates(), |entries| {
        entries
            .iter()
            .any(|e| matches!(e, TimelineEntry::Event(ev) if ev.sender == bob.session.user_id()))
    })
    .await?;

    let small = test_png(48);
    let big = test_png(1200);
    let notes = b"basal melt: 0.1 m/yr\n".to_vec();
    let dir = TempDir::new()?;
    let notes_path = dir.path().join("notes.txt");
    std::fs::write(&notes_path, &notes)?;
    // The text, then each file as a message of its own, in order.
    let png = |name: &str, bytes: &Vec<u8>| Attachment {
        filename: name.into(),
        mimetype: "image/png".into(),
        bytes: bytes.clone(),
    };
    // Each upload reports its progress, and is gone once sent.
    let uploads = alice_timeline.uploads().await?;
    let pending = alice_timeline
        .send(
            "Results:",
            vec![
                png("small.png", &small),
                png("big.png", &big),
                Attachment::read(&notes_path).await?,
            ],
            None,
        )
        .await?;
    timeout("all sent", pending.sent()).await??;
    let mut uploads = std::pin::pin!(uploads);
    let mut seen = false;
    timeout("uploads reported", async {
        while let Some(now) = uploads.next().await {
            seen |= now.iter().any(|u| u.total > 0);
            if seen && now.is_empty() {
                break;
            }
        }
    })
    .await?;

    let bob_timeline = bob.session.timeline(&room).await?;
    let entries = wait_for("bob sees the files", bob_timeline.updates(), |e| {
        ["small.png", "big.png", "notes.txt"]
            .iter()
            .all(|name| find_media(e, name).is_some())
    })
    .await?;
    let order: Vec<&str> = entries
        .iter()
        .filter_map(|e| match e {
            TimelineEntry::Event(ev) => match &ev.content {
                EventContent::Message(m) => Some(m.body.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(order, ["Results:", "small.png", "big.png", "notes.txt"]);
    let image = find_media(&entries, "small.png").unwrap();
    assert_eq!(image.kind, mtrx_core::MediaKind::Image);
    assert_eq!((image.width, image.height), (Some(48), Some(48)));
    assert_eq!(image.size, Some(small.len() as u64));
    assert_eq!(image.mimetype.as_deref(), Some("image/png"));
    let file = find_media(&entries, "notes.txt").unwrap();
    assert_eq!(file.kind, mtrx_core::MediaKind::File);
    assert_eq!(file.mimetype.as_deref(), Some("text/plain"));
    assert_eq!((file.width, file.height), (None, None));

    let s = &bob.session;
    assert_eq!(s.media_content(&image.source).await?, small);
    assert_eq!(s.media_preview(&image.source).await?, small);
    assert_eq!(s.media_content(&file.source).await?, notes);
    // The big image comes with a thumbnail.
    let big_media = find_media(&entries, "big.png").unwrap();
    assert_eq!(s.media_content(&big_media.source).await?, big);
    let preview = image::load_from_memory(&s.media_preview(&big_media.source).await?)?;
    assert_eq!((preview.width(), preview.height()), (600, 600));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn room_categories_and_tags() -> Result<()> {
    let (name, first) = new_user("categories").await?;
    let s = &first.session;
    let room = s
        .create_room(NewRoom {
            name: Some("ice".into()),
            ..Default::default()
        })
        .await?;
    let second = login(&name).await?;

    // A change shows at once, and reaches the other device.
    let mut id = String::new();
    s.change_room_categories(|c| {
        let layout = c.layout_mut(None);
        id = layout.add_category("Projects");
        layout.place(&room, Some(&id));
    })
    .await?;
    assert_eq!(
        s.room_categories()
            .home
            .category_of(&room)
            .map(|c| c.name.clone()),
        Some("Projects".into())
    );
    wait_for(
        "the other device sees the category",
        second.session.room_categories_updates(),
        |c| {
            c.home
                .category(&id)
                .is_some_and(|c| c.rooms == [room.clone()])
        },
    )
    .await?;

    // So do room list colours.
    s.change_room_list_colours(|c| {
        c.home = Some("Teal".into());
        c.spaces.insert(room.clone(), "Rose".into());
    })
    .await?;
    assert_eq!(s.room_list_colours().home.as_deref(), Some("Teal"));
    wait_for(
        "the other device sees the colours",
        second.session.room_list_colours_updates(),
        |c| {
            c.home.as_deref() == Some("Teal")
                && c.spaces.get(&room).map(String::as_str) == Some("Rose")
        },
    )
    .await?;

    // Tags show in the room list.
    let list = s.room_list().await?;
    list.wait_loaded().await;
    s.set_room_tag(&room, mtrx_core::RoomTag::Favourite).await?;
    let tag =
        |state: &mtrx_core::RoomListState| state.rooms.iter().find(|r| r.id == room).map(|r| r.tag);
    wait_for("favourite", list.updates(), |state| {
        tag(state) == Some(mtrx_core::RoomTag::Favourite)
    })
    .await?;
    s.set_room_tag(&room, mtrx_core::RoomTag::LowPriority)
        .await?;
    wait_for("low priority", list.updates(), |state| {
        tag(state) == Some(mtrx_core::RoomTag::LowPriority)
    })
    .await?;
    s.set_room_tag(&room, mtrx_core::RoomTag::None).await?;
    wait_for("untagged", list.updates(), |state| {
        tag(state) == Some(mtrx_core::RoomTag::None)
    })
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn timeline_shows_its_start() -> Result<()> {
    let (_, alice) = new_user("start").await?;
    let room = alice
        .session
        .create_room(NewRoom {
            name: Some("short".into()),
            ..Default::default()
        })
        .await?;
    let timeline = alice.session.timeline(&room).await?;
    timeout("paginate", async {
        while !timeline.paginate_back(20).await? {}
        anyhow::Ok(())
    })
    .await??;
    wait_for("start entry", timeline.updates(), |entries| {
        matches!(entries.first(), Some(TimelineEntry::Start))
    })
    .await?;
    Ok(())
}

/// A message far back is found by loading older events; one not in the
/// room is not, once its start is loaded.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn seeking_a_message_far_back() -> Result<()> {
    let (name, alice) = new_user("seek").await?;
    let room = alice
        .session
        .create_room(NewRoom {
            name: Some("seek".into()),
            ..Default::default()
        })
        .await?;
    let timeline = alice.session.timeline(&room).await?;
    let send = |text: String| {
        let timeline = &timeline;
        async move {
            let pending = timeline.send_markdown(&text, None).await?;
            anyhow::Ok(timeout("send", pending.sent()).await??)
        }
    };
    let far_back = send("Far back".into()).await?;
    for i in 0..80 {
        send(format!("Filler {i}")).await?;
    }

    // A new device loads only the latest messages.
    let second = login(&name).await?;
    let list = second.session.room_list().await?;
    wait_for("room synced", list.updates(), |s| {
        s.rooms.iter().any(|r| r.id == room)
    })
    .await?;
    let timeline = second.session.timeline(&room).await?;
    assert!(!has_message(&timeline.current(), "Far back"));
    let id = timeout("seek", timeline.seek(&far_back)).await??;
    let found = find_message(&timeline.current(), "Far back").map(|e| e.id.clone());
    assert_eq!(found.as_ref(), Some(&id));
    // There now: no loading.
    assert_eq!(timeout("item", timeline.item_of(&far_back)).await?, id);

    let missing: mtrx_core::OwnedEventId =
        format!("$missing:{}", second.session.user_id().server_name())
            .try_into()
            .unwrap();
    let not_found = timeout("seek missing", timeline.seek(&missing)).await?;
    assert!(
        matches!(not_found, Err(mtrx_core::Error::EventNotFound)),
        "{not_found:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn contacts_and_user_info() -> Result<()> {
    let (_, alice) = new_user("people").await?;
    let (_, bob) = new_user("people").await?;
    let (_, carol) = new_user("people").await?;
    let s = &alice.session;
    let space = s
        .create_room(NewRoom {
            name: Some("lab".into()),
            space: true,
            invite: vec![bob.session.user_id()],
            ..Default::default()
        })
        .await?;
    let room = s
        .create_room(NewRoom {
            name: Some("ice".into()),
            parent: Some(space.clone()),
            invite: vec![bob.session.user_id(), carol.session.user_id()],
            ..Default::default()
        })
        .await?;
    join_synced(&bob, &space, "lab").await?;
    join_synced(&bob, &room, "ice").await?;
    join_synced(&carol, &room, "ice").await?;

    let (bob_id, carol_id) = (bob.session.user_id(), carol.session.user_id());
    let contacts = timeout("contacts", async {
        loop {
            let contacts = s.contacts().await?;
            let ids: Vec<_> = contacts.iter().map(|c| c.user.id.clone()).collect();
            if ids.contains(&bob_id) && ids.contains(&carol_id) {
                return anyhow::Ok(contacts);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await??;
    // Bob shares the space too, so he comes first.
    assert_eq!(contacts[0].user.id, bob_id);
    assert_eq!(contacts[0].spaces, std::slice::from_ref(&space));
    assert!(!contacts.iter().any(|c| c.user.id == s.user_id()));

    let info = s.user_info(&bob_id).await?;
    assert_eq!(info.user.name.as_deref(), Some(bob_id.localpart()));
    assert!(info.shared.contains(&space) && info.shared.contains(&room));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn notes_to_self() -> Result<()> {
    let (_, alice) = new_user("notes").await?;
    let s = &alice.session;
    assert_eq!(s.notes_room(), None);
    let notes = s.direct_chat(&s.user_id()).await?;
    let list = s.room_list().await?;
    let state = wait_for("notes listed as a direct chat", list.updates(), |state| {
        state.rooms.iter().any(|r| {
            r.id == notes && r.kind == RoomKind::Direct && r.direct_user == Some(s.user_id())
        })
    })
    .await?;
    assert_eq!(
        state
            .rooms
            .iter()
            .find(|r| r.id == notes)
            .map(|r| r.name.as_str()),
        Some("Notes")
    );
    // Found again rather than created twice.
    assert_eq!(s.direct_chat(&s.user_id()).await?, notes);
    assert_eq!(s.notes_room(), Some(notes.clone()));

    let timeline = s.timeline(&notes).await?;
    timeout(
        "send",
        timeline.send_markdown("buy milk", None).await?.sent(),
    )
    .await??;
    wait_for("note", timeline.updates(), |e| has_message(e, "buy milk")).await?;
    timeout(
        "second note",
        timeline
            .send_markdown("- check *ice* data", None)
            .await?
            .sent(),
    )
    .await??;
    let entries = wait_for("both notes", timeline.updates(), |e| {
        has_message(e, "- check *ice* data")
    })
    .await?;
    let exported = mtrx_core::notes_markdown(&entries, None);
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    assert!(exported.starts_with(&format!("## {today} ")), "{exported}");
    assert!(exported.contains("\n\nbuy milk\n\n## "), "{exported}");
    assert!(exported.ends_with("\n\n- check *ice* data\n"), "{exported}");
    let tomorrow = chrono::Local::now().date_naive().succ_opt().unwrap();
    assert_eq!(mtrx_core::notes_markdown(&entries, Some(tomorrow)), "");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn room_settings() -> Result<()> {
    let (_, alice) = new_user("settings").await?;
    let s = &alice.session;
    assert_eq!(s.connection(), Connection::Online);
    let room = s
        .create_room(NewRoom {
            name: Some("settings".into()),
            ..Default::default()
        })
        .await?;
    let list = s.room_list().await?;
    // Its state (name, permissions) comes with sync.
    wait_for("room synced", list.updates(), |state| {
        state
            .rooms
            .iter()
            .any(|r| r.id == room && r.name == "settings")
    })
    .await?;
    let settings = s.room_settings(&room).await?;
    assert!(settings.can_rename && settings.can_change_topic && settings.can_change_picture);
    assert!(settings.can_invite);
    assert_eq!(settings.notify, NotifyMode::All);

    // Each change returns once sync confirms it: well before its 10 s limit.
    for mode in [NotifyMode::Mute, NotifyMode::Mentions, NotifyMode::All] {
        let started = std::time::Instant::now();
        s.set_notify_mode(&room, mode).await?;
        assert!(started.elapsed() < Duration::from_secs(5), "not confirmed");
        assert_eq!(s.room_settings(&room).await?.notify, mode);
    }

    // Who can join: anyone, the space's members, invited people.
    assert!(settings.can_change_access);
    assert_eq!(settings.access, Access::Invite);
    s.set_room_access(&room, Access::Public).await?;
    wait_for("public", list.updates(), |state| {
        state.rooms.iter().any(|r| r.id == room && r.public)
    })
    .await?;
    let space = s
        .create_room(NewRoom {
            name: Some("settings space".into()),
            space: true,
            ..Default::default()
        })
        .await?;
    s.add_to_space(&space, &room).await?;
    wait_for("in the space", list.updates(), |state| {
        state
            .rooms
            .iter()
            .any(|r| r.id == room && r.parents.contains(&space))
    })
    .await?;
    s.set_room_access(&room, Access::SpaceMembers).await?;
    wait_for("space members", list.updates(), |state| {
        state.rooms.iter().any(|r| r.id == room && !r.public)
    })
    .await?;
    assert_eq!(s.room_settings(&room).await?.access, Access::SpaceMembers);
    s.set_room_access(&room, Access::Invite).await?;
    // Not told apart in the room list: check after each update.
    let mut updates = std::pin::pin!(list.updates());
    timeout("invite only", async {
        while s.room_settings(&room).await?.access != Access::Invite {
            updates.next().await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn signed_out_elsewhere() -> Result<()> {
    let name = unique_name("ended");
    register(&name).await?;
    let dir = TempDir::new()?;
    let profile = || Profile::new("p", dir.path(), SecretStore::File);
    let session = Session::login(profile()?, &homeserver(), &name, PASSWORD).await?;
    session.start_sync().await;
    // Restoring in a second process works alongside (tokens are shared).
    drop(Session::restore(profile()?).await?);

    test_support::log_out_everywhere(&name)
        .await
        .map_err(anyhow::Error::msg)?;
    wait_for("signed out", session.connection_updates(), |c| {
        *c == Connection::SignedOut
    })
    .await?;
    assert_eq!(session.connection(), Connection::SignedOut);

    session.forget().await?;
    assert!(matches!(
        Session::restore(profile()?).await,
        Err(mtrx_core::Error::NotLoggedIn(_))
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn failed_sends_are_retried_or_discarded() -> Result<()> {
    let (_, alice) = new_user("alice").await?;
    let room = alice
        .session
        .create_room(NewRoom {
            name: Some("unsent".into()),
            ..Default::default()
        })
        .await?;
    let timeline = alice.session.timeline(&room).await?;
    // The server refuses events over 64 KiB, whatever we try.
    let too_long = "x".repeat(70_000);
    let failed = |entries: &Vec<TimelineEntry>| {
        find_message(entries, &too_long)
            .is_some_and(|ev| matches!(ev.send_state, mtrx_core::SendState::Failed(_)))
    };
    let pending = timeline.send_markdown(&too_long, None).await?;
    assert!(timeout("refused", pending.sent()).await?.is_err());
    let entries = wait_for("shown as failed", timeline.updates(), failed).await?;
    let key = find_message(&entries, &too_long).unwrap().key.clone();

    // Sent again: sending, then failed again.
    let mut updates = std::pin::pin!(timeline.updates());
    timeline.retry(&key).await?;
    wait_for("sending again", &mut updates, |entries| {
        find_message(entries, &too_long)
            .is_some_and(|ev| ev.send_state == mtrx_core::SendState::Sending)
    })
    .await?;
    wait_for("failed again", &mut updates, failed).await?;

    // What is queued after it waits for it; once it is dropped, it is sent.
    let later = timeline.send_markdown("after the long one", None).await?;
    timeline.discard(&key).await?;
    timeout("sent after discarding", later.sent()).await??;
    let entries = timeline.current();
    assert!(find_message(&entries, &too_long).is_none());
    // A sent message is no longer a local echo.
    let sent = find_message(&entries, "after the long one").unwrap();
    assert!(timeline.retry(&sent.key).await.is_err());
    assert!(timeline.discard(&sent.key).await.is_err());
    Ok(())
}

/// Long enough for a watcher's next read of a user (once a minute).
const PRESENCE_TIMEOUT: Duration = Duration::from_secs(100);

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn presence_between_users() -> Result<()> {
    let (_, alice) = new_user("presence").await?;
    let (_, bob) = new_user("presence").await?;
    let (_, carol) = new_user("presence").await?;
    let alice_id = alice.session.user_id();
    // Synapse shows presence to those who share a room.
    let room = alice
        .session
        .create_room(NewRoom {
            name: Some("here".into()),
            invite: vec![bob.session.user_id()],
            ..Default::default()
        })
        .await?;
    join_synced(&bob, &room, "here").await?;

    alice
        .session
        .set_presence(PresenceState::Online, Some("In a meeting"))
        .await?;
    let seen = bob.session.presence(&alice_id).await?;
    assert_eq!(seen.state, PresenceState::Online);
    assert_eq!(seen.status_message.as_deref(), Some("In a meeting"));
    assert!(seen.currently_active);
    let last_active = seen.last_active.context("last active")?;
    let ago = SystemTime::now()
        .duration_since(last_active)
        .unwrap_or_default();
    assert!(ago < TIMEOUT, "{ago:?}");

    let watcher = bob.session.watch_presence([alice_id.clone()]);
    let mut updates = std::pin::pin!(watcher.updates());
    let (user, first) = timeout("the first presence", updates.next())
        .await?
        .context("ended")??;
    assert_eq!(
        (user, first.state),
        (alice_id.clone(), PresenceState::Online)
    );

    // Read again a minute later: the server kept Alice's presence (it
    // drops one not refreshed for 30 s), so the change is to unavailable,
    // not offline.
    alice
        .session
        .set_presence(PresenceState::Unavailable, None)
        .await?;
    let changed = tokio::time::timeout(PRESENCE_TIMEOUT, async {
        while let Some(update) = updates.next().await {
            let (_, presence) = update?;
            if presence.state != PresenceState::Online {
                return Ok(presence);
            }
        }
        bail!("the watcher ended")
    })
    .await
    .context("timed out: the change to unavailable")??;
    assert_eq!(changed.state, PresenceState::Unavailable);
    assert_eq!(changed.status_message, None);

    alice
        .session
        .set_presence(PresenceState::Offline, None)
        .await?;
    let seen = bob.session.presence(&alice_id).await?;
    assert_eq!(seen.state, PresenceState::Offline);

    // Ours from the app's activity: online in the foreground, offline in
    // the background.
    let own = alice.session.own_presence();
    own.activity(Activity::Foreground).await?;
    let seen = bob.session.presence(&alice_id).await?;
    assert_eq!(seen.state, PresenceState::Online);
    own.activity(Activity::Background).await?;
    let seen = bob.session.presence(&alice_id).await?;
    assert_eq!(seen.state, PresenceState::Offline);

    // Synapse refuses the presence of someone Bob shares no room with,
    // while it answers his own: that user's error, not a server without
    // presence.
    match bob.session.presence(&carol.session.user_id()).await {
        Err(mtrx_core::Error::PresenceUnavailable(answer)) => {
            bail!("a stranger's presence read as unavailable: {answer}")
        }
        Err(e) => assert!(e.to_string().contains("M_FORBIDDEN"), "{e}"),
        Ok(presence) => bail!("a stranger's presence: {presence:?}"),
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn someone_elses_public_room_is_public() -> Result<()> {
    let (_, bob) = new_user("host").await?;
    let (alice_name, alice) = new_user("guest").await?;
    let room = bob
        .session
        .create_room(NewRoom {
            name: Some("open to all".into()),
            ..Default::default()
        })
        .await?;
    bob.session.set_room_access(&room, Access::Public).await?;
    let public = async |what: &str, device: &Device| -> Result<()> {
        let list = device.session.room_list().await?;
        wait_for(what, list.updates(), |state| {
            state.rooms.iter().any(|r| r.id == room && r.public)
        })
        .await?;
        Ok(())
    };
    public("public for its creator", &bob).await?;
    alice
        .session
        .join(<&mtrx_core::RoomOrAliasId>::from(&*room), &[])
        .await?;
    public("public once joined", &alice).await?;
    // And on a device that first sees it already joined.
    let other = login(&alice_name).await?;
    public("public on another device", &other).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn drafts_are_kept_per_room_across_restarts() -> Result<()> {
    let (_, alice) = new_user("drafts").await?;
    let s = &alice.session;
    let room = s.create_room(NewRoom::default()).await?;
    let other = s.create_room(NewRoom::default()).await?;
    let mut with_drafts = Box::pin(s.rooms_with_drafts_updates());
    assert!(with_drafts.next().await.unwrap().is_empty());
    assert_eq!(s.draft(&room).await?, None);

    let reply_to = answered_event();
    let draft = Draft {
        text: "half a thought".into(),
        reply_to: Some(reply_to),
    };
    s.save_draft(&room, &draft).await?;
    assert_eq!(s.draft(&room).await?, Some(draft.clone()));
    assert_eq!(s.draft(&other).await?, None, "per room");
    assert_eq!(
        with_drafts.next().await.unwrap(),
        std::collections::BTreeSet::from([room.clone()])
    );

    // Kept on the device: back after a restart, and listed.
    let Device { session, _dir: dir } = alice;
    drop(session);
    let s = Session::restore(Profile::new("test", dir.path(), SecretStore::File)?).await?;
    assert_eq!(s.draft(&room).await?, Some(draft));
    let mut with_drafts = Box::pin(s.rooms_with_drafts_updates());
    assert_eq!(
        with_drafts.next().await.unwrap(),
        std::collections::BTreeSet::from([room.clone()])
    );

    // Sending (an empty draft) clears it.
    s.save_draft(&room, &Draft::default()).await?;
    assert_eq!(s.draft(&room).await?, None);
    assert!(with_drafts.next().await.unwrap().is_empty());

    // Kept by a keeper, as the composer changes.
    let keeper = s.draft_keeper(&other);
    assert_eq!(keeper.restore().await, None);
    let elsewhere = Draft {
        text: "elsewhere".into(),
        reply_to: None,
    };
    keeper.changed(elsewhere.clone());
    keeper.flush().await;
    assert_eq!(s.draft(&other).await?, Some(elsewhere.clone()));
    assert_eq!(
        with_drafts.next().await.unwrap(),
        std::collections::BTreeSet::from([other.clone()])
    );
    assert_eq!(s.draft_keeper(&other).restore().await, Some(elsewhere));
    Ok(())
}

/// The actions the row of the message `event` offers; none without a row.
fn actions(rows: &[mtrx_core::Row], event: &mtrx_core::OwnedEventId) -> Vec<MessageAction> {
    (rows.iter())
        .find_map(|row| match row {
            mtrx_core::Row::Message(m) if m.event.event_id.as_ref() == Some(event) => {
                Some(m.actions.clone())
            }
            _ => None,
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn changes_show_before_sync_brings_them() -> Result<()> {
    let (_, alice) = new_user("echoes").await?;
    let s = &alice.session;
    let list = s.room_list().await?;
    let mut lists = std::pin::pin!(s.room_lists(&list));
    // A room made is coming until listed.
    let room = s
        .create_room(NewRoom {
            name: Some("echoes".into()),
            ..Default::default()
        })
        .await?;
    wait_for("listed, no longer coming", &mut lists, |l| {
        l.room(&room).is_some() && l.joining.is_empty()
    })
    .await?;
    let timeline = s.timeline(&room).await?;
    let plan = timeout(
        "send",
        timeline.send_markdown("The plan", None).await?.sent(),
    )
    .await??;
    wait_for("Pin offered", timeline.rows(), |rows| {
        actions(rows, &plan).contains(&MessageAction::Pin)
    })
    .await?;

    // Without sync, the room list and the rows show our changes at once.
    s.stop_sync().await;
    s.set_room_topic(&room, "About.").await?;
    s.set_room_tag(&room, mtrx_core::RoomTag::Favourite).await?;
    s.set_pinned(&room, &plan, true).await?;
    wait_for("the changes listed", &mut lists, |l| {
        l.room(&room).is_some_and(|r| {
            r.topic.as_deref() == Some("About.")
                && r.tag == mtrx_core::RoomTag::Favourite
                && r.pinned == [plan.clone()]
        })
    })
    .await?;
    wait_for("Unpin offered", timeline.rows(), |rows| {
        actions(rows, &plan).contains(&MessageAction::Unpin)
    })
    .await?;
    // So does our reaction, added and taken away: matrix-sdk-ui's own.
    let key = find_message(&timeline.current(), "The plan")
        .unwrap()
        .key
        .clone();
    let reacted = |entries: &Vec<TimelineEntry>| {
        find_message(entries, "The plan")
            .is_some_and(|ev| (message(ev).reactions.iter()).any(|r| r.own && r.key == "👍"))
    };
    timeline.toggle_reaction(&key, "👍").await?;
    wait_for("our reaction", timeline.updates(), reacted).await?;
    timeline.toggle_reaction(&key, "👍").await?;
    wait_for("taken away", timeline.updates(), |e| !reacted(e)).await?;

    // Synced, as shown.
    s.start_sync().await;
    wait_for("synced", list.updates(), |state| {
        (state.rooms.iter()).any(|r| {
            r.id == room && r.topic.as_deref() == Some("About.") && r.pinned == [plan.clone()]
        })
    })
    .await?;
    Ok(())
}

fn answered_event() -> mtrx_core::OwnedEventId {
    "$answered:example.org".try_into().unwrap()
}
