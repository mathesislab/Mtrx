//! `mtrx`: command-line Matrix client built on `mtrx-core`.

// matrix-sdk futures are deeply nested.
#![recursion_limit = "256"]

mod google;
mod presence;
mod rooms;
mod timeline;

use std::{
    collections::HashMap,
    io::{BufRead, Read},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use futures_util::StreamExt;
use mtrx_core::{
    EventContent, Live, NewRoom, OwnedRoomId, OwnedUserId, Permissions, Profile, RecoveryStep,
    RoomKind, RoomListState, RoomTimeline, SecretStore, Session, TimelineEntry, Verification,
    VerificationState, describe_send_time, parse_send_time,
};

/// How long one-shot commands wait for the first sync.
const SYNC_TIMEOUT: Duration = Duration::from_secs(60);
/// How long `read` waits for late room keys.
const DECRYPTION_GRACE: Duration = Duration::from_secs(5);

#[derive(Parser)]
#[command(name = "mtrx", version, about = "Matrix client")]
struct Cli {
    /// Account profile to use.
    #[arg(long, global = true, env = "MTRX_PROFILE", default_value = "default")]
    profile: String,
    /// Root directory for profiles (default: platform data directory).
    #[arg(long, global = true, env = "MTRX_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Where to keep access tokens and store passphrases.
    #[arg(
        long,
        global = true,
        env = "MTRX_SECRET_STORE",
        default_value = "keyring"
    )]
    secret_store: SecretStoreArg,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum SecretStoreArg {
    /// OS credential store.
    Keyring,
    /// Plain-text files in the data directory (development only).
    File,
}

#[derive(Subcommand)]
enum Command {
    /// Log in with a password.
    Login {
        /// Server name (matrix.org) or URL (http://localhost:8008).
        #[arg(long)]
        homeserver: String,
        /// User name or full user ID (password login).
        #[arg(long, required_unless_present = "browser")]
        user: Option<String>,
        /// Read the password from the first line of stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,
        /// Sign in through the server's web page instead (e.g. matrix.org).
        #[arg(long, conflicts_with_all = ["user", "password_stdin"])]
        browser: bool,
    },
    /// Log out and delete local data for the profile.
    Logout,
    /// Show the logged-in account.
    Whoami,
    /// List rooms, grouped by space.
    Rooms {
        /// Show room IDs.
        #[arg(long)]
        ids: bool,
    },
    /// Create a room. Every member may invite and edit it unless --moderated.
    CreateRoom {
        name: String,
        /// Create an unencrypted room.
        #[arg(long)]
        unencrypted: bool,
        /// Only moderators may edit the room (the usual Matrix default).
        #[arg(long)]
        moderated: bool,
        /// Users to invite.
        #[arg(long)]
        invite: Vec<OwnedUserId>,
    },
    /// Create a space. Every member may edit it unless --moderated.
    CreateSpace {
        name: String,
        /// Only moderators may edit the space.
        #[arg(long)]
        moderated: bool,
    },
    /// Let every member invite and edit a room; for a space, also every
    /// room in it. Removing people and changing permissions stay with admins.
    LetEveryoneEdit { room: String },
    /// Join a room by ID or alias, or accept an invite.
    Join { room: String },
    /// Leave a room.
    Leave { room: String },
    /// Invite a user to a room.
    Invite { room: String, user: OwnedUserId },
    /// Add a room to a space.
    SpaceAdd { space: String, room: String },
    /// Print recent messages of a room.
    Read {
        room: String,
        /// Number of events to show.
        #[arg(short = 'n', default_value_t = 20)]
        count: usize,
    },
    /// Send a Markdown message (from the argument, or stdin if omitted).
    Send { room: String, text: Option<String> },
    /// Schedule a Markdown message (from the argument, or stdin if omitted).
    /// Scheduled messages are sent while the app or `mtrx watch` runs.
    Schedule {
        room: String,
        /// E.g. "in 30m", "17:30", "tomorrow 9:00", "fri", "2026-11-02 10:00".
        when: String,
        text: Option<String>,
    },
    /// List scheduled messages.
    Scheduled,
    /// Cancel a scheduled message.
    Unschedule { id: String },
    /// Send a scheduled message now, e.g. an overdue one.
    SendNow { id: String },
    /// Follow a room's messages, or unread counts of all rooms, until Ctrl-C.
    /// Sends scheduled messages when due.
    Watch { room: Option<String> },
    /// Add a note to self (Markdown, from the argument or stdin if omitted).
    Note { text: Option<String> },
    /// Print your notes to self (the Notes chat) as Markdown, oldest first,
    /// each under a heading with its time.
    Notes {
        /// Only notes from this day on (YYYY-MM-DD).
        #[arg(long)]
        since: Option<chrono::NaiveDate>,
        /// Write to this file instead of printing.
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// Show our devices, verification and recovery status.
    Devices,
    /// Manage recovery (encrypted key backup).
    #[command(subcommand)]
    Recovery(RecoveryCommand),
    /// Verify this device with another of our devices by comparing emojis.
    Verify {
        /// Wait for the other device to send the request instead.
        #[arg(long)]
        wait: bool,
    },
    /// Google account for Meet links (app-wide, not per profile).
    #[command(subcommand)]
    Google(google::GoogleCommand),
    /// Show someone's presence (state, last active, status message), or
    /// set ours with --set.
    #[command(
        override_usage = "mtrx presence USER_ID\n       mtrx presence --set <STATE> [--status TEXT]"
    )]
    Presence {
        /// Whose, e.g. @alice:matrix.org.
        #[arg(value_name = "USER_ID", required_unless_present = "set")]
        user: Option<OwnedUserId>,
        /// Set our presence instead.
        #[arg(long, value_name = "STATE", conflicts_with = "user")]
        set: Option<presence::StateArg>,
        /// Status message to set with --set (none clears it).
        #[arg(long, value_name = "TEXT", requires = "set")]
        status: Option<String>,
    },
}

#[derive(Subcommand)]
enum RecoveryCommand {
    /// Set up recovery and print the recovery key.
    Enable,
    /// Create a new encryption identity and recovery key, when no other device
    /// or recovery key is available. Old encrypted messages may become unreadable.
    Reset {
        /// Read the account password from the first line of stdin.
        #[arg(long)]
        password_stdin: bool,
    },
    /// Unlock recovery on this device with the recovery key.
    Recover {
        /// Read the key from the first line of stdin instead of prompting.
        #[arg(long)]
        key_stdin: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,matrix_sdk=error,matrix_sdk::http_client=off,matrix_sdk::latest_events=off".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let secrets = match cli.secret_store {
        SecretStoreArg::Keyring => SecretStore::Keyring,
        SecretStoreArg::File => SecretStore::File,
    };
    let data_root = match cli.data_dir {
        Some(dir) => dir,
        None => Profile::default_data_root()?,
    };
    let profile = Profile::new(&cli.profile, &data_root, secrets.clone())?;

    match cli.command {
        Command::Login {
            homeserver,
            user,
            password_stdin,
            browser,
        } => {
            let session = if browser {
                Session::login_oauth(profile, &homeserver, |url| {
                    println!("Opening the sign-in page in your browser:\n{url}");
                    let _ = open::that_detached(url.as_str());
                })
                .await
            } else {
                let user = user.expect("clap requires --user without --browser");
                let password = read_secret(password_stdin, "Password: ")?;
                Session::login(profile, &homeserver, &user, &password).await
            }
            .context("login failed")?;
            println!(
                "Logged in as {} (device {})",
                session.user_id(),
                session.device_id()
            );
        }
        Command::Logout => {
            Session::restore(profile).await?.logout().await?;
            println!("Logged out");
        }
        Command::Whoami => {
            let session = Session::restore(profile).await?;
            println!("user:       {}", session.user_id());
            println!("device:     {}", session.device_id());
            println!("homeserver: {}", session.homeserver());
            println!("profile:    {}", session.profile().dir().display());
        }
        Command::Rooms { ids } => {
            let (session, list) = synced(profile).await?;
            rooms::print_tree(&list.get().rooms, ids);
            session.stop_sync().await;
        }
        Command::CreateRoom {
            name,
            unencrypted,
            moderated,
            invite,
        } => {
            let session = Session::restore(profile).await?;
            let new = NewRoom {
                name: Some(name),
                encrypted: !unencrypted,
                invite,
                permissions: permissions(moderated),
                ..Default::default()
            };
            println!("{}", session.create_room(new).await?);
        }
        Command::CreateSpace { name, moderated } => {
            let session = Session::restore(profile).await?;
            let new = NewRoom {
                name: Some(name),
                space: true,
                permissions: permissions(moderated),
                ..Default::default()
            };
            println!("{}", session.create_room(new).await?);
        }
        Command::Join { room } => {
            let session = Session::restore(profile).await?;
            let room = room
                .as_str()
                .try_into()
                .context("expected a room ID or alias")?;
            println!("{}", session.join(room, &[]).await?);
        }
        Command::LetEveryoneEdit { room } => {
            let (session, list) = synced(profile).await?;
            let id = session.find_room(&room).await?;
            let is_space = list
                .get()
                .rooms
                .iter()
                .any(|r| r.id == id && r.kind == RoomKind::Space);
            let room = session.room(&id)?;
            if is_space {
                let update = room.let_everyone_edit_all().await?;
                println!(
                    "Opened {} room(s); {} already open; {} not allowed (not an admin there).",
                    update.changed.len(),
                    update.unchanged.len(),
                    update.not_allowed.len()
                );
            } else {
                room.set_permissions(Permissions::Everyone).await?;
            }
            session.stop_sync().await;
        }
        Command::Leave { room } => {
            let (session, _list) = synced(profile).await?;
            session
                .room(&session.find_room(&room).await?)?
                .leave()
                .await?;
            session.stop_sync().await;
        }
        Command::Invite { room, user } => {
            let (session, _list) = synced(profile).await?;
            let room = session.find_room(&room).await?;
            session.room(&room)?.invite(&user).await?;
            session.stop_sync().await;
        }
        Command::SpaceAdd { space, room } => {
            let (session, _list) = synced(profile).await?;
            let space: OwnedRoomId = session.find_room(&space).await?;
            let room = session.find_room(&room).await?;
            session.room(&space)?.add_child(&room).await?;
            session.stop_sync().await;
        }
        Command::Read { room, count } => {
            let (session, _list) = synced(profile).await?;
            let timeline = session.timeline(&session.find_room(&room).await?).await?;
            load_events(&timeline, count).await?;
            settle_decryption(&timeline).await;
            let entries = timeline.current();
            let skip = entries.len().saturating_sub(count);
            for entry in &entries[skip..] {
                println!("{}", timeline::entry_line(entry));
            }
            session.stop_sync().await;
        }
        Command::Note { text } => {
            let text = read_text(text)?;
            let (session, _list) = synced(profile).await?;
            let notes = session.direct_chat(&session.user_id()).await?;
            let timeline = session.timeline(&notes).await?;
            let pending = timeline.send_markdown(text.trim_end(), None).await?;
            let sent = tokio::time::timeout(SYNC_TIMEOUT, pending.sent()).await;
            session.stop_sync().await;
            sent.context("timed out sending")??;
        }
        Command::Notes { since, output } => {
            let (session, _list) = synced(profile).await?;
            let room = session
                .notes_room()
                .context("No notes yet: start them in Mtrx with “Notes to self”")?;
            let timeline = session.timeline(&room).await?;
            while !timeline.paginate_back(100).await? {}
            settle_decryption(&timeline).await;
            let notes = mtrx_core::notes_markdown(&timeline.current(), since);
            session.stop_sync().await;
            match output {
                Some(path) => std::fs::write(&path, notes)
                    .with_context(|| format!("writing {}", path.display()))?,
                None => print!("{notes}"),
            }
        }
        Command::Send { room, text } => {
            let text = read_text(text)?;
            let (session, _list) = synced(profile).await?;
            let timeline = session.timeline(&session.find_room(&room).await?).await?;
            let pending = timeline.send_markdown(&text, None).await?;
            let sent = tokio::time::timeout(SYNC_TIMEOUT, pending.sent()).await;
            session.stop_sync().await;
            sent.context("timed out sending")??;
        }
        Command::Schedule { room, when, text } => {
            let now = SystemTime::now();
            let send_at = parse_send_time(&when, now)
                .with_context(|| format!("cannot read the time `{when}`"))?;
            let text = read_text(text)?;
            let (session, _list) = synced(profile).await?;
            let room = session.find_room(&room).await?;
            let message = session
                .room(&room)?
                .schedule(text.trim_end(), Vec::new(), send_at, None)
                .await?;
            session.stop_sync().await;
            println!(
                "Scheduled for {} (id {}). It is sent while Mtrx or `mtrx watch` runs.",
                describe_send_time(send_at, now),
                message.id
            );
        }
        Command::Scheduled => {
            let (session, list) = synced(profile).await?;
            let rooms = list.get().rooms;
            let now = SystemTime::now();
            for message in session.scheduled() {
                let room = rooms
                    .iter()
                    .find(|r| r.id == message.room_id)
                    .map_or(message.room_id.as_str(), |r| r.name.as_str());
                let files: Vec<&str> = message.files.iter().map(|f| f.filename.as_str()).collect();
                println!(
                    "{}  {:<20} {}{}: {}{}",
                    message.id,
                    describe_send_time(message.send_at, now),
                    room,
                    if message.overdue { " [overdue]" } else { "" },
                    message.text.lines().next().unwrap_or_default(),
                    if files.is_empty() {
                        String::new()
                    } else {
                        format!(" [+ {}]", files.join(", "))
                    },
                );
            }
            session.stop_sync().await;
        }
        Command::Unschedule { id } => {
            let session = Session::restore(profile).await?;
            session.unschedule(&id).await?;
        }
        Command::SendNow { id } => {
            let (session, _list) = synced(profile).await?;
            let pending = session.send_scheduled_now(&id).await?;
            let sent = tokio::time::timeout(SYNC_TIMEOUT, pending.sent()).await;
            session.stop_sync().await;
            sent.context("timed out sending")??;
        }
        Command::Watch { room: Some(room) } => {
            let (session, _list) = synced(profile).await?;
            start_scheduler(&session);
            let timeline = session.timeline(&session.find_room(&room).await?).await?;
            load_events(&timeline, 10).await?;
            watch_timeline(&timeline).await;
            session.stop_sync().await;
        }
        Command::Watch { room: None } => {
            let (session, list) = synced(profile).await?;
            start_scheduler(&session);
            watch_rooms(&list).await;
            session.stop_sync().await;
        }
        Command::Devices => {
            let (session, _list) = synced(profile).await?;
            println!("recovery: {:?}", session.recovery_status().await);
            for device in session.devices().await? {
                println!(
                    "{} {}  {}{}",
                    if device.verified {
                        "[verified]  "
                    } else {
                        "[unverified]"
                    },
                    device.id,
                    device.name.as_deref().unwrap_or(""),
                    if device.current {
                        "  (this device)"
                    } else {
                        ""
                    },
                );
            }
            session.stop_sync().await;
        }
        Command::Recovery(RecoveryCommand::Enable) => {
            let (session, _list) = synced(profile).await?;
            let key = session.enable_recovery().await?;
            println!("Recovery key (store it safely, it is shown only once):\n{key}");
            session.stop_sync().await;
        }
        Command::Recovery(RecoveryCommand::Reset { password_stdin }) => {
            let password = read_secret(password_stdin, "Account password: ")?;
            let (session, _list) = synced(profile).await?;
            let key = session.reset_encryption(&password).await?;
            println!("New recovery key (store it safely, it is shown only once):\n{key}");
            session.stop_sync().await;
        }
        Command::Recovery(RecoveryCommand::Recover { key_stdin }) => {
            let key = read_secret(key_stdin, "Recovery key: ")?;
            let (session, _list) = synced(profile).await?;
            session
                .recover(&key, |step| match step {
                    RecoveryStep::CheckingKey => println!("Checking the recovery key…"),
                    RecoveryStep::ImportingIdentity => {
                        println!("Verifying this device and connecting to the key backup…")
                    }
                    RecoveryStep::DownloadingKeys { done, total } => {
                        println!("Downloading message keys for recent rooms: {done} of {total}…")
                    }
                })
                .await?;
            println!(
                "Recovered; this device verified: {}",
                session.is_device_verified().await?
            );
            session.stop_sync().await;
        }
        Command::Verify { wait } => {
            let (session, _list) = synced(profile).await?;
            let verification = if wait {
                println!("Waiting for a verification request (Ctrl-C to stop)...");
                let mut requests = std::pin::pin!(session.verification_requests());
                let verification = tokio::select! {
                    v = requests.next() => v.context("verification requests ended")?,
                    _ = tokio::signal::ctrl_c() => return Ok(()),
                };
                verification.accept().await?;
                verification
            } else {
                println!("Request sent; accept it on your other device.");
                session.request_self_verification().await?
            };
            tokio::time::timeout(SYNC_TIMEOUT, run_verification(&verification))
                .await
                .context("timed out verifying")??;
            session.stop_sync().await;
        }
        Command::Google(command) => {
            google::run(command, &data_root, secrets).await?;
        }
        Command::Presence { user, set, status } => {
            let session = Session::restore(profile).await?;
            match (user, set) {
                (_, Some(state)) => presence::set(&session, state, status.as_deref()).await?,
                (Some(user), None) => presence::show(&session, &user).await?,
                (None, None) => unreachable!("clap requires a user without --set"),
            }
        }
    }
    Ok(())
}

/// Prompt without echo, or read the first line of stdin (for scripts).
fn read_secret(from_stdin: bool, prompt: &str) -> Result<String> {
    if from_stdin {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        Ok(line.trim_end_matches(['\r', '\n']).to_owned())
    } else {
        Ok(rpassword::prompt_password(prompt)?)
    }
}

/// Show emojis, ask the user to compare, until done or cancelled.
async fn run_verification(verification: &Verification) -> Result<()> {
    let mut states = verification.states().updates();
    while let Some(state) = states.next().await {
        match state {
            VerificationState::Waiting | VerificationState::Confirmed => {}
            VerificationState::SharingKeys => println!("Emojis match; exchanging keys..."),
            VerificationState::Emojis(emojis) => {
                for emoji in &emojis {
                    print!("{} {}   ", emoji.symbol, emoji.description);
                }
                println!();
                let answer = tokio::task::spawn_blocking(|| {
                    print!("Do the emojis match on both devices? [y/N] ");
                    std::io::Write::flush(&mut std::io::stdout())?;
                    let mut line = String::new();
                    std::io::stdin().lock().read_line(&mut line)?;
                    Ok::<_, std::io::Error>(line.trim().eq_ignore_ascii_case("y"))
                })
                .await??;
                if answer {
                    verification.confirm().await?;
                } else {
                    verification.mismatch().await?;
                }
            }
            VerificationState::Done => {
                println!("Verified.");
                return Ok(());
            }
            VerificationState::Cancelled(reason) => {
                anyhow::bail!("verification cancelled: {reason}")
            }
        }
    }
    Ok(())
}

/// Paginate until at least `count` events are loaded or the room start is reached.
async fn load_events(timeline: &RoomTimeline, count: usize) -> Result<()> {
    let events = |t: &RoomTimeline| {
        t.current()
            .iter()
            .filter(|e| matches!(e, TimelineEntry::Event(_)))
            .count()
    };
    while events(timeline) < count {
        if timeline.paginate_back(count.max(20) as u16).await? {
            break;
        }
    }
    Ok(())
}

/// Keys for undecryptable events may still arrive (key backup, other
/// devices); give them a moment before printing.
async fn settle_decryption(timeline: &RoomTimeline) {
    let undecryptable = |entries: &Vec<TimelineEntry>| {
        entries.iter().any(|e| {
            matches!(e, TimelineEntry::Event(ev) if ev.content == EventContent::UnableToDecrypt)
        })
    };
    let mut updates = timeline.updates();
    let wait = async {
        while let Some(entries) = updates.next().await {
            if !undecryptable(&entries) {
                return;
            }
        }
    };
    let _ = tokio::time::timeout(DECRYPTION_GRACE, wait).await;
}

/// Print the timeline, then new and changed events, until Ctrl-C.
async fn watch_timeline(timeline: &RoomTimeline) {
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut updates = timeline.updates();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return,
            entries = updates.next() => {
                let Some(entries) = entries else { return };
                for entry in entries {
                    let TimelineEntry::Event(event) = &entry else { continue };
                    let line = timeline::event_line(event);
                    match seen.insert(event.id.clone(), line.clone()) {
                        None => println!("{line}"),
                        Some(old) if old != line => println!("~ {line}"),
                        Some(_) => {}
                    }
                }
            }
        }
    }
}

/// Send scheduled messages while watching; overdue ones only get a notice.
fn start_scheduler(session: &Session) {
    session.start_scheduler();
    let overdue = session.scheduled().iter().filter(|m| m.overdue).count();
    if overdue > 0 {
        eprintln!(
            "{overdue} scheduled message(s) are overdue and held; see `mtrx scheduled`, \
             then `mtrx send-now ID` or `mtrx unschedule ID`."
        );
    }
}

fn permissions(moderated: bool) -> Permissions {
    if moderated {
        Permissions::Moderators
    } else {
        Permissions::Everyone
    }
}

/// The message text from the argument, or stdin if omitted.
fn read_text(text: Option<String>) -> Result<String> {
    match text {
        Some(text) => Ok(text),
        None => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            Ok(text)
        }
    }
}

/// Print rooms whose unread count changes, until Ctrl-C.
async fn watch_rooms(list: &Live<RoomListState>) {
    let mut unread: HashMap<OwnedRoomId, u64> = HashMap::new();
    let mut updates = list.updates();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return,
            state = updates.next() => {
                let Some(state) = state else { return };
                for room in state.rooms {
                    if unread.insert(room.id.clone(), room.unread_messages) != Some(room.unread_messages) {
                        println!("{}: {} unread", room.name, room.unread_messages);
                    }
                }
            }
        }
    }
}

/// Restore the session and wait until the room list is loaded.
async fn synced(profile: Profile) -> Result<(Arc<Session>, Live<RoomListState>)> {
    let session = Arc::new(Session::restore(profile).await?);
    session.start_sync().await;
    let list = session.room_list().await?;
    tokio::time::timeout(SYNC_TIMEOUT, list.wait_for(|s| s.loaded))
        .await
        .context("timed out waiting for sync")?;
    Ok((session, list))
}
