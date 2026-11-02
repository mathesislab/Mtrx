//! `mtrx presence`: someone's presence, or setting ours.

use std::time::SystemTime;

use anyhow::{Result, bail};
use clap::ValueEnum;
use mtrx_core::{Presence, PresenceState, Session, UserId, describe_last_active};

#[derive(Clone, Copy, ValueEnum)]
pub enum StateArg {
    Online,
    Unavailable,
    Offline,
}

impl From<StateArg> for PresenceState {
    fn from(state: StateArg) -> Self {
        match state {
            StateArg::Online => Self::Online,
            StateArg::Unavailable => Self::Unavailable,
            StateArg::Offline => Self::Offline,
        }
    }
}

pub async fn show(session: &Session, user: &UserId) -> Result<()> {
    match session.presence(user).await {
        Ok(presence) => {
            print!("{}", describe(user, &presence, SystemTime::now()));
            Ok(())
        }
        Err(mtrx_core::Error::PresenceUnavailable(answer)) => {
            bail!("This server does not share presence (it answered: {answer})")
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn set(session: &Session, state: StateArg, status: Option<&str>) -> Result<()> {
    match session.set_presence(state.into(), status).await {
        Ok(()) => {}
        Err(mtrx_core::Error::PresenceUnavailable(answer)) => {
            bail!("This server does not share presence (it answered: {answer})")
        }
        Err(e) => return Err(e.into()),
    }
    let state = name(state.into());
    match status {
        Some(status) => println!("Presence set to {state}, status “{status}”."),
        None => println!("Presence set to {state}, no status."),
    }
    if !matches!(state, "offline") {
        println!(
            "Servers drop a presence that no client refreshes (Synapse after about 30 s); \
             the apps refresh it while open."
        );
    }
    Ok(())
}

fn name(state: PresenceState) -> &'static str {
    match state {
        PresenceState::Online => "online",
        PresenceState::Unavailable => "unavailable",
        PresenceState::Offline => "offline",
    }
}

fn describe(user: &UserId, presence: &Presence, now: SystemTime) -> String {
    let state = match (presence.state, presence.currently_active) {
        (PresenceState::Online, true) => "online, active now",
        (state, _) => name(state),
    };
    let last_active = presence
        .last_active
        .map_or("unknown".into(), |at| describe_last_active(at, now));
    let status = presence.status_message.as_deref().unwrap_or("none");
    let mut out = format!(
        "user:         {user}\nstate:        {state}\nlast active:  {last_active}\nstatus:       {status}\n"
    );
    let bare = presence.state == PresenceState::Offline
        && presence.last_active.is_none()
        && presence.status_message.is_none();
    if bare {
        out.push_str(
            "note:         the server knows nothing more about this user. A server that does \
             not share presence (Synapse with presence off) answers this for everyone: \
             look up your own ID while another of your clients is open to tell.\n",
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn presence_reads_plainly() {
        let user = <&UserId>::try_from("@alice:example.org").unwrap();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100_000);
        let online = Presence {
            state: PresenceState::Online,
            last_active: Some(now - Duration::from_secs(2 * 3600)),
            currently_active: false,
            status_message: Some("In a meeting".into()),
        };
        assert_eq!(
            describe(user, &online, now),
            "user:         @alice:example.org\n\
             state:        online\n\
             last active:  2 hr ago\n\
             status:       In a meeting\n"
        );
        let active = Presence {
            currently_active: true,
            ..online
        };
        assert!(describe(user, &active, now).contains("state:        online, active now\n"));

        let bare = Presence {
            state: PresenceState::Offline,
            last_active: None,
            currently_active: false,
            status_message: None,
        };
        let text = describe(user, &bare, now);
        assert!(
            text.contains("state:        offline\nlast active:  unknown\nstatus:       none\n")
        );
        assert!(text.contains("Synapse with presence off"));
    }
}
