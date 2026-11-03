//! Presence: whether people are online, idle or away, when they were last
//! active, and their status message.
//!
//! Sync does not carry presence: simplified sliding sync (MSC4186) has no
//! presence extension, and the SDK's sliding sync responses never include
//! any. So others' presence is read from the presence endpoint, and
//! [`PresenceWatcher`] polls it. Ours is set there too, and kept: a server
//! forgets a presence that no client refreshes (Synapse after 30 s, and
//! its sliding sync does not count as a refresh), so the session sends it
//! again every [`KEEP_ALIVE`] until it is set offline or the session ends.
//! [`OwnPresence`] sets ours from what the app is doing.

use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    sync::{Arc, Mutex, Weak},
    time::{Duration, SystemTime},
};

use futures_util::{Stream, future::BoxFuture};
use matrix_sdk::{
    Client, HttpError,
    ruma::{
        OwnedUserId, UserId,
        api::{client::presence::get_presence, error::ErrorKind},
        presence::PresenceState as RumaPresenceState,
    },
};
use tokio::{
    sync::{Notify, broadcast},
    time::Instant,
};
use tracing::{debug, warn};

use crate::{AbortOnDrop, Error, Result, Session};

/// How often our presence is sent again while it is not offline. Synapse
/// sets a device offline 30 s after its last refresh.
const KEEP_ALIVE: Duration = Duration::from_secs(20);
/// Without input for this long in the foreground, we are idle.
pub const IDLE_AFTER: Duration = Duration::from_secs(5 * 60);
/// How often [`PresenceWatcher`] reads each user's presence.
const POLL_EVERY: Duration = Duration::from_secs(60);
/// The shortest time between two reads of [`PresenceWatcher`]: with many
/// users, a round takes longer than [`POLL_EVERY`].
const MIN_GAP: Duration = Duration::from_millis(500);
/// Last activity that moved by less than this is not a change: the server
/// says how long ago, so each read differs by the request's time.
const LAST_ACTIVE_RESOLUTION: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresenceState {
    Online,
    /// Idle, or away.
    Unavailable,
    Offline,
}

/// Someone's presence, as their server knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presence {
    pub state: PresenceState,
    /// When they last did something, if the server says.
    pub last_active: Option<SystemTime>,
    /// Online and active within about the last minute.
    pub currently_active: bool,
    pub status_message: Option<String>,
}

impl Presence {
    fn from_response(response: get_presence::v3::Response, now: SystemTime) -> Self {
        Self {
            state: response.presence.into(),
            last_active: response
                .last_active_ago
                .and_then(|ago| now.checked_sub(ago)),
            currently_active: response.currently_active.unwrap_or(false),
            status_message: response.status_msg.filter(|m| !m.trim().is_empty()),
        }
    }

    /// Whether `self` says something `old` does not: last activity counts
    /// once it moved by [`LAST_ACTIVE_RESOLUTION`].
    fn differs_from(&self, old: &Self) -> bool {
        let moved = match (self.last_active, old.last_active) {
            (Some(new), Some(old)) => {
                let by = new.duration_since(old).unwrap_or_else(|e| e.duration());
                by >= LAST_ACTIVE_RESOLUTION
            }
            (new, old) => new.is_some() != old.is_some(),
        };
        self.state != old.state
            || self.currently_active != old.currently_active
            || self.status_message != old.status_message
            || moved
    }
}

impl From<RumaPresenceState> for PresenceState {
    fn from(state: RumaPresenceState) -> Self {
        match state {
            RumaPresenceState::Online => Self::Online,
            RumaPresenceState::Unavailable => Self::Unavailable,
            // Unknown states (a server's "busy", say) say nothing we show.
            _ => Self::Offline,
        }
    }
}

impl From<PresenceState> for RumaPresenceState {
    fn from(state: PresenceState) -> Self {
        match state {
            PresenceState::Online => Self::Online,
            PresenceState::Unavailable => Self::Unavailable,
            PresenceState::Offline => Self::Offline,
        }
    }
}

/// The dot on someone's picture: green online, amber idle (unavailable).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresenceDot {
    Online,
    Idle,
}

impl PresenceDot {
    /// What the dot says, for accessibility.
    pub fn label(self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::Idle => "Idle",
        }
    }
}

impl Presence {
    /// The dot on their picture; none offline.
    pub fn dot(&self) -> Option<PresenceDot> {
        match self.state {
            PresenceState::Online => Some(PresenceDot::Online),
            PresenceState::Unavailable => Some(PresenceDot::Idle),
            PresenceState::Offline => None,
        }
    }

    /// The line under their name: their status message, else "Active
    /// now", "Idle" or when they were last active ("Last active 2 hr
    /// ago"); nothing if offline with nothing more known.
    pub fn line(&self, now: SystemTime) -> Option<String> {
        let status = self.status_message.as_deref();
        if let Some(message) = status.filter(|m| !m.trim().is_empty()) {
            return Some(message.into());
        }
        match self.state {
            PresenceState::Online => Some("Active now".into()),
            PresenceState::Unavailable => Some("Idle".into()),
            PresenceState::Offline => self
                .last_active
                .map(|at| format!("Last active {}", describe_last_active(at, now))),
        }
    }
}

/// How long ago `at` was, as iOS says it in English: "just now" under a
/// minute, then "5 min ago", "2 hr ago", "1 day ago", "3 days ago", "2 wk
/// ago", "4 mo ago" (months of 30 days), "1 yr ago".
pub fn describe_last_active(at: SystemTime, now: SystemTime) -> String {
    let minutes = now.duration_since(at).unwrap_or_default().as_secs() / 60;
    let (hours, days) = (minutes / 60, minutes / 1440);
    match days {
        0 if minutes == 0 => "just now".into(),
        0 if hours == 0 => format!("{minutes} min ago"),
        0 => format!("{hours} hr ago"),
        1 => "1 day ago".into(),
        2..7 => format!("{days} days ago"),
        7..30 => format!("{} wk ago", days / 7),
        30..365 => format!("{} mo ago", (days / 30).min(11)),
        _ => format!("{} yr ago", days / 365),
    }
}

/// The ways a server refuses presence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    /// `M_FORBIDDEN`: presence is off (conduwuit), or we may not see this
    /// user's (Synapse, for someone we share no room with).
    Forbidden,
    /// `M_UNRECOGNIZED`: the server has no presence endpoint.
    Unrecognized,
}

fn refusal(kind: Option<&ErrorKind>) -> Option<Refusal> {
    match kind {
        Some(ErrorKind::Forbidden) => Some(Refusal::Forbidden),
        Some(ErrorKind::Unrecognized) => Some(Refusal::Unrecognized),
        _ => None,
    }
}

/// Read `user`'s presence with `read` (the endpoint). A refusal means the
/// server does not share presence ([`Error::PresenceUnavailable`]) when it
/// also refuses ours; else only this user's is hidden from us, which is
/// the server's error as it is.
async fn fetch<F>(
    read: impl Fn(OwnedUserId) -> F,
    own: &UserId,
    user: &UserId,
    now: impl Fn() -> SystemTime,
) -> Result<Presence>
where
    F: Future<Output = Result<get_presence::v3::Response, HttpError>>,
{
    let error = match read(user.to_owned()).await {
        Ok(response) => return Ok(Presence::from_response(response, now())),
        Err(error) => error,
    };
    match refusal(error.client_api_error_kind()) {
        None => Err(error.into()),
        Some(Refusal::Unrecognized) => Err(Error::PresenceUnavailable(error.to_string())),
        Some(Refusal::Forbidden) if user == own => {
            Err(Error::PresenceUnavailable(error.to_string()))
        }
        Some(Refusal::Forbidden) => match read(own.to_owned()).await {
            Err(own_error) if refusal(own_error.client_api_error_kind()).is_some() => {
                Err(Error::PresenceUnavailable(own_error.to_string()))
            }
            _ => Err(error.into()),
        },
    }
}

/// Set our presence; [`Error::PresenceUnavailable`] if the server refuses.
async fn send(client: &Client, state: PresenceState, status_message: Option<String>) -> Result<()> {
    // Through the client, so that sync requests ask for the same state.
    match client
        .set_presence(state.into(), status_message, true)
        .await
    {
        Ok(()) => Ok(()),
        Err(error) if refusal(error.client_api_error_kind()).is_some() => {
            Err(Error::PresenceUnavailable(error.to_string()))
        }
        Err(error) => Err(error.into()),
    }
}

/// Call `send` every [`KEEP_ALIVE`], until the server does not share
/// presence.
async fn keep_alive<F: Future<Output = Result<()>>>(mut send: impl FnMut() -> F) {
    loop {
        tokio::time::sleep(KEEP_ALIVE).await;
        match send().await {
            Ok(()) => {}
            Err(Error::PresenceUnavailable(answer)) => {
                warn!("the server stopped sharing presence: {answer}");
                return;
            }
            Err(e) => debug!("could not refresh our presence: {e}"),
        }
    }
}

/// Sets our presence on the server, and keeps it there.
#[derive(Clone)]
pub(crate) struct Keeper {
    client: Client,
    /// Refreshes our presence while it is not offline.
    kept: Arc<tokio::sync::Mutex<Option<AbortOnDrop>>>,
}

impl Keeper {
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client,
            kept: Default::default(),
        }
    }

    /// Set our presence, and keep it until it is set again, set offline or
    /// the keeper is dropped. A send that fails (the server unreachable,
    /// say) is tried again with the refreshes, unless the server refuses
    /// ([`Error::PresenceUnavailable`]).
    async fn set(&self, state: PresenceState, status_message: Option<String>) -> Result<()> {
        let mut kept = self.kept.lock().await;
        *kept = None;
        let sent = send(&self.client, state, status_message.clone()).await;
        let refused = matches!(sent, Err(Error::PresenceUnavailable(_)));
        if state != PresenceState::Offline && !refused {
            let client = self.client.clone();
            *kept = Some(AbortOnDrop(tokio::spawn(keep_alive(move || {
                let (client, status_message) = (client.clone(), status_message.clone());
                async move { send(&client, state, status_message).await }
            }))));
        }
        sent
    }
}

/// Our presence from the app's activity, set through `keeper`.
pub(crate) fn own_presence(keeper: Keeper) -> OwnPresence {
    OwnPresence::new(Box::new(move |state| {
        let keeper = keeper.clone();
        Box::pin(async move { keeper.set(state, None).await })
    }))
}

impl Session {
    /// `user`'s presence. [`Error::PresenceUnavailable`]: the server does
    /// not share presence. Note that Synapse with presence off answers
    /// "offline", with nothing else, for everyone.
    pub async fn presence(&self, user: &UserId) -> Result<Presence> {
        self.server_presence().fetch(user).await
    }

    /// Set our presence, and keep it until it is set again, set offline or
    /// the session ends; a failed send is tried again every [`KEEP_ALIVE`].
    /// `status_message` `None` clears it. The apps set ours through
    /// [`Session::own_presence`] instead.
    pub async fn set_presence(
        &self,
        state: PresenceState,
        status_message: Option<&str>,
    ) -> Result<()> {
        let status_message = status_message.map(ToOwned::to_owned);
        self.presence_keeper.set(state, status_message).await
    }

    /// Our presence, from the app's activity.
    pub fn own_presence(&self) -> &OwnPresence {
        &self.own_presence
    }

    /// Follow the presence of `users`; see [`PresenceWatcher`].
    pub fn watch_presence(&self, users: impl IntoIterator<Item = OwnedUserId>) -> PresenceWatcher {
        PresenceWatcher::spawn(self.server_presence(), users)
    }

    fn server_presence(&self) -> ServerPresence {
        ServerPresence {
            client: self.client.clone(),
            own: self.user_id(),
        }
    }
}

/// What the app is doing, which sets our presence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    /// The app comes to the foreground; also at start and sign-in.
    Foreground,
    /// The app goes to the background, or its window closes.
    Background,
    /// A touch, key press or click.
    Input,
    /// Signing out: offline, as in the background (until the next
    /// foreground, should signing out fail).
    SignOut,
}

/// How [`OwnPresence`] sets our presence: the server, or a fake.
type Sender = Box<dyn Fn(PresenceState) -> BoxFuture<'static, Result<()>> + Send + Sync>;

/// Our presence as the app's activity says, while "Share my presence" is
/// on: online in the foreground, idle after [`IDLE_AFTER`] there without
/// input, and offline in the background, when signing out, or not
/// sharing. Each change is set once, in order, and kept (see
/// [`Session::set_presence`]).
pub struct OwnPresence {
    own: Arc<Own>,
}

struct Own {
    send: Sender,
    state: Mutex<OwnState>,
    /// Sets run one at a time, in order.
    queue: tokio::sync::Mutex<()>,
}

struct OwnState {
    /// "Share my presence", a device setting each app keeps; on until the
    /// app says otherwise.
    sharing: bool,
    foreground: bool,
    last_input: Instant,
    /// What we set last, if anything.
    set: Option<PresenceState>,
    /// The server does not share presence (its answer).
    unavailable: Option<String>,
    /// Sets us idle; runs while we are online.
    idle_timer: Option<AbortOnDrop>,
}

impl OwnState {
    /// Our presence as of `now`.
    fn wanted(&self, now: Instant) -> PresenceState {
        if !self.sharing || !self.foreground {
            PresenceState::Offline
        } else if now >= self.last_input + IDLE_AFTER {
            PresenceState::Unavailable
        } else {
            PresenceState::Online
        }
    }
}

impl OwnPresence {
    pub(crate) fn new(send: Sender) -> Self {
        Self {
            own: Arc::new(Own {
                send,
                state: Mutex::new(OwnState {
                    sharing: true,
                    foreground: false,
                    last_input: Instant::now(),
                    set: None,
                    unavailable: None,
                    idle_timer: None,
                }),
                queue: Default::default(),
            }),
        }
    }

    /// The app's `activity`, as of this call; the future sets our presence
    /// if it changes, after the sets before. [`Error::PresenceUnavailable`]:
    /// the server does not share presence; other failures are tried again
    /// with the refreshes. Foreground: online if sharing, else offline (wait
    /// for it before sync starts, so that sync says the same); input: online
    /// again if idle; background and signing out: offline.
    pub fn activity(&self, activity: Activity) -> impl Future<Output = Result<()>> + use<> {
        let changes = {
            let mut own = self.own.state.lock().unwrap();
            match activity {
                Activity::Foreground => {
                    own.foreground = true;
                    own.last_input = Instant::now();
                    true
                }
                Activity::Background | Activity::SignOut => {
                    own.foreground = false;
                    true
                }
                Activity::Input => {
                    own.last_input = Instant::now();
                    own.set == Some(PresenceState::Unavailable)
                }
            }
        };
        let own = self.own.clone();
        async move { if changes { own.update().await } else { Ok(()) } }
    }

    /// Share our presence or stop, as of this call (on at first): pass the
    /// app's setting before the first [`Activity::Foreground`]. The future
    /// sets us offline, or online if in the foreground.
    pub fn set_sharing(&self, on: bool) -> impl Future<Output = Result<()>> + use<> {
        let changes = {
            let mut own = self.own.state.lock().unwrap();
            let changes = own.sharing != on;
            own.sharing = on;
            if on {
                own.last_input = Instant::now();
            }
            changes
        };
        let own = self.own.clone();
        async move { if changes { own.update().await } else { Ok(()) } }
    }
}

impl Own {
    /// Set our presence as it should be now, if that changed; and keep the
    /// idle timer running while we are online. Boxed, as the timer calls it
    /// in turn.
    fn update(self: Arc<Self>) -> BoxFuture<'static, Result<()>> {
        Box::pin(async move {
            let _turn = self.queue.lock().await;
            let state = {
                let mut own = self.state.lock().unwrap();
                if let Some(answer) = &own.unavailable {
                    return Err(Error::PresenceUnavailable(answer.clone()));
                }
                let wanted = own.wanted(Instant::now());
                if wanted != PresenceState::Online {
                    own.idle_timer = None;
                } else if own.idle_timer.as_ref().is_none_or(|t| t.0.is_finished()) {
                    let timer = tokio::spawn(idle_timer(Arc::downgrade(&self)));
                    own.idle_timer = Some(AbortOnDrop(timer));
                }
                if own.set == Some(wanted) {
                    return Ok(());
                }
                own.set = Some(wanted);
                wanted
            };
            let sent = (self.send)(state).await;
            if let Err(Error::PresenceUnavailable(answer)) = &sent {
                let mut own = self.state.lock().unwrap();
                own.unavailable = Some(answer.clone());
                own.idle_timer = None;
            }
            sent
        })
    }
}

/// Wait for [`IDLE_AFTER`] without input, then set us idle.
async fn idle_timer(own: Weak<Own>) {
    loop {
        let Some(own) = own.upgrade() else { return };
        let idle_at = own.state.lock().unwrap().last_input + IDLE_AFTER;
        if Instant::now() >= idle_at {
            // Apart from this task, which the update stops.
            tokio::spawn(async move {
                if let Err(e) = own.update().await {
                    debug!("could not set us idle: {e}");
                }
            });
            return;
        }
        drop(own);
        tokio::time::sleep_until(idle_at).await;
    }
}

/// Where [`PresenceWatcher`] reads presence: the server, or a fake.
pub(crate) trait PresenceSource: Send + Sync + 'static {
    fn fetch(&self, user: &UserId) -> impl Future<Output = Result<Presence>> + Send;
}

struct ServerPresence {
    client: Client,
    own: OwnedUserId,
}

impl PresenceSource for ServerPresence {
    async fn fetch(&self, user: &UserId) -> Result<Presence> {
        fetch(
            |user| {
                let client = self.client.clone();
                async move { client.send(get_presence::v3::Request::new(user)).await }
            },
            &self.own,
            user,
            SystemTime::now,
        )
        .await
    }
}

#[derive(Clone, Debug)]
enum Update {
    Presence(OwnedUserId, Presence),
    /// The server does not share presence (its answer): the last update.
    Unavailable(String),
}

#[derive(Default)]
struct Known {
    presences: HashMap<OwnedUserId, Presence>,
    unavailable: Option<String>,
}

type Item = Result<(OwnedUserId, Presence)>;

impl Known {
    /// What a new reader gets first.
    fn snapshot(&self) -> VecDeque<Item> {
        let presences = self.presences.iter();
        let mut items: VecDeque<Item> =
            presences.map(|(u, p)| Ok((u.clone(), p.clone()))).collect();
        if let Some(answer) = &self.unavailable {
            items.push_back(Err(Error::PresenceUnavailable(answer.clone())));
        }
        items
    }
}

struct Shared {
    updates: broadcast::Sender<Update>,
    known: Arc<Mutex<Known>>,
    /// Woken when a reader subscribes.
    subscribed: Notify,
}

impl Shared {
    fn record(&self, user: OwnedUserId, presence: Presence) {
        let mut known = self.known.lock().unwrap();
        let old = known.presences.get(&user);
        if old.is_none_or(|old| presence.differs_from(old)) {
            known.presences.insert(user.clone(), presence.clone());
            let _ = self.updates.send(Update::Presence(user, presence));
        }
    }

    fn unavailable(&self, answer: String) {
        let mut known = self.known.lock().unwrap();
        known.unavailable = Some(answer.clone());
        let _ = self.updates.send(Update::Unavailable(answer));
    }
}

/// Follows the presence of a set of users: reads each about once a minute,
/// spread over the minute, while someone reads [`PresenceWatcher::updates`].
/// Stops when dropped.
pub struct PresenceWatcher {
    shared: Arc<Shared>,
    _task: AbortOnDrop,
}

impl PresenceWatcher {
    pub(crate) fn spawn(
        source: impl PresenceSource,
        users: impl IntoIterator<Item = OwnedUserId>,
    ) -> Self {
        let mut unique = Vec::new();
        for user in users {
            if !unique.contains(&user) {
                unique.push(user);
            }
        }
        let (updates, _) = broadcast::channel(unique.len() * 2 + 16);
        let shared = Arc::new(Shared {
            updates,
            known: Default::default(),
            subscribed: Notify::new(),
        });
        let task = tokio::spawn(poll(source, shared.clone(), unique));
        Self {
            shared,
            _task: AbortOnDrop(task),
        }
    }

    /// The presences known so far, then each change, as `(user, presence)`;
    /// a user's first is the first change. Ends with
    /// [`Error::PresenceUnavailable`] if the server does not share presence,
    /// and when the watcher is dropped. Users whose presence could not be
    /// read (the server hides it, or is unreachable) are left out until it
    /// can.
    pub fn updates(&self) -> impl Stream<Item = Item> + use<> {
        let (pending, updates) = {
            let known = self.shared.known.lock().unwrap();
            (known.snapshot(), self.shared.updates.subscribe())
        };
        self.shared.subscribed.notify_one();
        let reader = Reader {
            known: self.shared.known.clone(),
            ended: pending.iter().any(Result::is_err),
            pending,
            updates,
        };
        futures_util::stream::unfold(reader, Reader::next)
    }
}

struct Reader {
    known: Arc<Mutex<Known>>,
    pending: VecDeque<Item>,
    updates: broadcast::Receiver<Update>,
    ended: bool,
}

impl Reader {
    async fn next(mut self) -> Option<(Item, Self)> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                return Some((item, self));
            }
            if self.ended {
                return None;
            }
            match self.updates.recv().await {
                Ok(Update::Presence(user, presence)) => return Some((Ok((user, presence)), self)),
                Ok(Update::Unavailable(answer)) => {
                    self.ended = true;
                    return Some((Err(Error::PresenceUnavailable(answer)), self));
                }
                // Missed some: start again from what is known.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    self.pending = self.known.lock().unwrap().snapshot();
                    self.ended = self.pending.iter().any(Result::is_err);
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

/// When to read whom next: users never read at once, in order; then the
/// one due first, at least `gap` after the previous read.
fn next_read(
    due: &[Option<Instant>],
    last_read: Option<Instant>,
    gap: Duration,
    now: Instant,
) -> Option<(usize, Instant)> {
    if let Some(never) = due.iter().position(Option::is_none) {
        return Some((never, now));
    }
    let (index, at) = due
        .iter()
        .enumerate()
        .filter_map(|(i, at)| Some((i, (*at)?)))
        .min_by_key(|(_, at)| *at)?;
    let earliest = last_read.map_or(at, |last| last + gap);
    Some((index, at.max(earliest)))
}

async fn poll(source: impl PresenceSource, shared: Arc<Shared>, users: Vec<OwnedUserId>) {
    if users.is_empty() {
        return;
    }
    let gap = (POLL_EVERY / u32::try_from(users.len()).unwrap_or(u32::MAX)).max(MIN_GAP);
    let mut due: Vec<Option<Instant>> = vec![None; users.len()];
    let mut last_read = None;
    loop {
        if shared.updates.receiver_count() == 0 {
            shared.subscribed.notified().await;
            continue;
        }
        let Some((index, at)) = next_read(&due, last_read, gap, Instant::now()) else {
            return;
        };
        tokio::time::sleep_until(at).await;
        if shared.updates.receiver_count() == 0 {
            continue;
        }
        let user = &users[index];
        let result = source.fetch(user).await;
        let now = Instant::now();
        last_read = Some(now);
        due[index] = Some(now + POLL_EVERY);
        match result {
            Ok(presence) => shared.record(user.clone(), presence),
            Err(Error::PresenceUnavailable(answer)) => {
                shared.unavailable(answer);
                return;
            }
            Err(e) => debug!(%user, "could not read presence: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures_util::StreamExt;
    use matrix_sdk::{
        reqwest::StatusCode,
        ruma::api::{
            client::uiaa::UiaaResponse,
            error::{Error as MatrixError, ErrorBody, FromHttpResponseError, StandardErrorBody},
        },
    };

    use super::*;

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn user(name: &str) -> OwnedUserId {
        format!("@{name}:example.org").try_into().unwrap()
    }

    fn online() -> Presence {
        Presence {
            state: PresenceState::Online,
            last_active: Some(at(1000)),
            currently_active: true,
            status_message: None,
        }
    }

    fn server_error(status: StatusCode, kind: ErrorKind, message: &str) -> HttpError {
        let body = ErrorBody::Standard(StandardErrorBody::new(kind, message.into()));
        let error = UiaaResponse::MatrixError(MatrixError::new(status, body));
        HttpError::Api(Box::new(FromHttpResponseError::Server(error)))
    }

    fn forbidden() -> HttpError {
        server_error(
            StatusCode::FORBIDDEN,
            ErrorKind::Forbidden,
            "You are not allowed to see their presence.",
        )
    }

    #[test]
    fn responses_become_presences() {
        let mut response = get_presence::v3::Response::new(RumaPresenceState::Online);
        response.last_active_ago = Some(Duration::from_secs(90));
        response.currently_active = Some(true);
        response.status_msg = Some("In a meeting".into());
        assert_eq!(
            Presence::from_response(response, at(1000)),
            Presence {
                state: PresenceState::Online,
                last_active: Some(at(910)),
                currently_active: true,
                status_message: Some("In a meeting".into()),
            }
        );

        // Synapse with presence off answers this for everyone.
        let bare = get_presence::v3::Response::new(RumaPresenceState::Offline);
        assert_eq!(
            Presence::from_response(bare, at(1000)),
            Presence {
                state: PresenceState::Offline,
                last_active: None,
                currently_active: false,
                status_message: None,
            }
        );

        let mut odd = get_presence::v3::Response::new("busy".into());
        odd.status_msg = Some("  ".into());
        let odd = Presence::from_response(odd, at(1000));
        assert_eq!(odd.state, PresenceState::Offline);
        assert_eq!(odd.status_message, None);
    }

    #[test]
    fn small_moves_of_last_activity_are_no_change() {
        let old = online();
        let jitter = Presence {
            last_active: Some(at(1000 + 59)),
            ..old.clone()
        };
        assert!(!jitter.differs_from(&old));
        assert!(!old.differs_from(&jitter));
        let later = Presence {
            last_active: Some(at(1000 + 60)),
            ..old.clone()
        };
        assert!(later.differs_from(&old));
        let idle = Presence {
            state: PresenceState::Unavailable,
            ..old.clone()
        };
        assert!(idle.differs_from(&old));
        let status = Presence {
            status_message: Some("Lunch".into()),
            ..old.clone()
        };
        assert!(status.differs_from(&old));
        let unknown = Presence {
            last_active: None,
            ..old.clone()
        };
        assert!(unknown.differs_from(&old));
    }

    #[test]
    fn last_activity_reads_as_time_ago() {
        let now = at(100_000_000);
        let ago = |seconds: u64| describe_last_active(at(100_000_000 - seconds), now);
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(5 * 60), "5 min ago");
        assert_eq!(ago(3599), "59 min ago");
        assert_eq!(ago(2 * 3600 + 59), "2 hr ago");
        assert_eq!(ago(23 * 3600 + 3599), "23 hr ago");
        assert_eq!(ago(47 * 3600), "1 day ago");
        assert_eq!(ago(3 * 86400), "3 days ago");
        assert_eq!(ago(13 * 86400), "1 wk ago");
        assert_eq!(ago(29 * 86400), "4 wk ago");
        assert_eq!(ago(45 * 86400), "1 mo ago");
        assert_eq!(ago(364 * 86400), "11 mo ago");
        assert_eq!(ago(800 * 86400), "2 yr ago");
        // A clock behind the server's.
        assert_eq!(describe_last_active(at(100_000_010), now), "just now");
    }

    fn bare(state: PresenceState) -> Presence {
        Presence {
            state,
            last_active: None,
            currently_active: false,
            status_message: None,
        }
    }

    #[test]
    fn dot_per_state() {
        assert_eq!(bare(PresenceState::Online).dot(), Some(PresenceDot::Online));
        assert_eq!(
            bare(PresenceState::Unavailable).dot(),
            Some(PresenceDot::Idle)
        );
        assert_eq!(bare(PresenceState::Offline).dot(), None);
        assert_eq!(PresenceDot::Online.label(), "Online");
        assert_eq!(PresenceDot::Idle.label(), "Idle");
    }

    #[test]
    fn line_per_state_and_age() {
        let now = at(1_000_000);
        let ago = |seconds| Some(now - Duration::from_secs(seconds));
        let line = |presence: Presence| presence.line(now);
        assert_eq!(
            line(bare(PresenceState::Online)).as_deref(),
            Some("Active now")
        );
        assert_eq!(
            line(bare(PresenceState::Unavailable)).as_deref(),
            Some("Idle")
        );
        // Synapse with presence off: offline, nothing else.
        assert_eq!(line(bare(PresenceState::Offline)), None);
        let offline = |last_active| Presence {
            last_active,
            ..bare(PresenceState::Offline)
        };
        assert_eq!(
            line(offline(ago(20))).as_deref(),
            Some("Last active just now")
        );
        assert_eq!(
            line(offline(Some(now + Duration::from_secs(5)))).as_deref(),
            Some("Last active just now")
        );
        assert_eq!(
            line(offline(ago(2 * 3600 + 59))).as_deref(),
            Some("Last active 2 hr ago")
        );
        // A status message comes first, whatever the state; a blank one
        // says nothing.
        for state in [
            PresenceState::Online,
            PresenceState::Unavailable,
            PresenceState::Offline,
        ] {
            let busy = Presence {
                status_message: Some("In a meeting".into()),
                last_active: ago(600),
                ..bare(state)
            };
            assert_eq!(line(busy).as_deref(), Some("In a meeting"));
        }
        let blank = Presence {
            status_message: Some(" ".into()),
            ..bare(PresenceState::Unavailable)
        };
        assert_eq!(line(blank).as_deref(), Some("Idle"));
    }

    /// Our presence set through a fake server, which records what was set
    /// when, and refuses once `refuse` is set.
    #[derive(Clone)]
    struct FakeServer {
        start: Instant,
        sets: Arc<Mutex<Vec<(u64, PresenceState)>>>,
        refuse: Arc<Mutex<bool>>,
    }

    impl FakeServer {
        fn new() -> Self {
            Self {
                start: Instant::now(),
                sets: Default::default(),
                refuse: Default::default(),
            }
        }

        fn own(&self) -> OwnPresence {
            let server = self.clone();
            OwnPresence::new(Box::new(move |state| {
                let server = server.clone();
                Box::pin(async move {
                    if *server.refuse.lock().unwrap() {
                        return Err(Error::PresenceUnavailable("M_FORBIDDEN".into()));
                    }
                    let at = server.start.elapsed().as_secs();
                    server.sets.lock().unwrap().push((at, state));
                    Ok(())
                })
            }))
        }

        /// What was set, at which second.
        fn sets(&self) -> Vec<(u64, PresenceState)> {
            self.sets.lock().unwrap().clone()
        }
    }

    async fn sleep_secs(seconds: u64) {
        tokio::time::sleep(Duration::from_secs(seconds)).await;
        settle().await;
    }

    #[tokio::test(start_paused = true)]
    async fn idle_after_five_minutes_without_input_online_again_with_input() {
        use PresenceState::*;
        let server = FakeServer::new();
        let own = server.own();
        own.activity(Activity::Foreground).await.unwrap();
        // Input while online sets nothing; it starts the 5 minutes anew.
        sleep_secs(60).await;
        own.activity(Activity::Input).await.unwrap();
        sleep_secs(299).await;
        assert_eq!(server.sets(), [(0, Online)]);
        sleep_secs(1).await;
        assert_eq!(server.sets(), [(0, Online), (360, Unavailable)]);
        // Set once, not again while idle.
        sleep_secs(600).await;
        own.activity(Activity::Input).await.unwrap();
        own.activity(Activity::Input).await.unwrap();
        assert_eq!(server.sets()[2..], [(960, Online)]);
        sleep_secs(300).await;
        assert_eq!(server.sets()[3..], [(1260, Unavailable)]);
    }

    #[tokio::test(start_paused = true)]
    async fn offline_in_the_background_and_online_again_in_the_foreground() {
        use PresenceState::*;
        let server = FakeServer::new();
        let own = server.own();
        own.activity(Activity::Foreground).await.unwrap();
        sleep_secs(10).await;
        own.activity(Activity::Background).await.unwrap();
        // Neither input nor time changes it in the background.
        own.activity(Activity::Input).await.unwrap();
        sleep_secs(600).await;
        own.activity(Activity::Foreground).await.unwrap();
        assert_eq!(server.sets(), [(0, Online), (10, Offline), (610, Online)]);

        // What counts is the order of the calls, not of the sets.
        let away = own.activity(Activity::Background);
        let back = own.activity(Activity::Foreground);
        back.await.unwrap();
        away.await.unwrap();
        assert_eq!(server.sets().len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn not_sharing_is_offline_until_shared_again() {
        use PresenceState::*;
        let server = FakeServer::new();
        let own = server.own();
        // Sharing is on at first: saying so sets nothing.
        own.set_sharing(true).await.unwrap();
        assert_eq!(server.sets(), []);
        // Off at start: offline once, before sync starts.
        own.set_sharing(false).await.unwrap();
        own.activity(Activity::Foreground).await.unwrap();
        sleep_secs(600).await;
        own.activity(Activity::Input).await.unwrap();
        assert_eq!(server.sets(), [(0, Offline)]);

        // On in the foreground: online, idle 5 minutes later.
        own.set_sharing(true).await.unwrap();
        sleep_secs(300).await;
        own.set_sharing(false).await.unwrap();
        assert_eq!(
            server.sets(),
            [
                (0, Offline),
                (600, Online),
                (900, Unavailable),
                (900, Offline)
            ]
        );
        // On in the background: still offline.
        own.activity(Activity::Background).await.unwrap();
        own.set_sharing(true).await.unwrap();
        assert_eq!(server.sets().len(), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn signing_out_is_offline_until_the_next_foreground() {
        use PresenceState::*;
        let server = FakeServer::new();
        let own = server.own();
        own.activity(Activity::Foreground).await.unwrap();
        own.activity(Activity::SignOut).await.unwrap();
        own.activity(Activity::Input).await.unwrap();
        sleep_secs(600).await;
        assert_eq!(server.sets(), [(0, Online), (0, Offline)]);
        // Signing out failed: the app is still in the foreground.
        own.activity(Activity::Foreground).await.unwrap();
        assert_eq!(server.sets()[2..], [(600, Online)]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_refusal_ends_setting_ours() {
        let server = FakeServer::new();
        let own = server.own();
        *server.refuse.lock().unwrap() = true;
        let refused = own.activity(Activity::Foreground).await;
        assert!(
            matches!(refused, Err(Error::PresenceUnavailable(_))),
            "{refused:?}"
        );
        // Not tried again: each change says so at once.
        *server.refuse.lock().unwrap() = false;
        let again = own.activity(Activity::Background).await;
        assert!(
            matches!(again, Err(Error::PresenceUnavailable(_))),
            "{again:?}"
        );
        sleep_secs(600).await;
        assert_eq!(server.sets(), []);
    }

    /// `fetch` with a server that answers `own` and others as given.
    async fn fetch_from(
        own_answer: impl Fn() -> Result<get_presence::v3::Response, HttpError>,
        other_answer: impl Fn() -> Result<get_presence::v3::Response, HttpError>,
        user: &UserId,
    ) -> Result<Presence> {
        let own = self::user("me");
        fetch(
            |asked| {
                let answer = if asked == own {
                    own_answer()
                } else {
                    other_answer()
                };
                async move { answer }
            },
            &own,
            user,
            || at(1000),
        )
        .await
    }

    fn offline() -> Result<get_presence::v3::Response, HttpError> {
        Ok(get_presence::v3::Response::new(RumaPresenceState::Offline))
    }

    #[tokio::test]
    async fn refusals_for_everyone_mean_presence_is_unavailable() {
        // Presence off: ours is refused too (conduwuit).
        let error = fetch_from(|| Err(forbidden()), || Err(forbidden()), &user("bob")).await;
        assert!(
            matches!(&error, Err(Error::PresenceUnavailable(answer)) if answer.contains("M_FORBIDDEN")),
            "{error:?}"
        );
        assert_eq!(
            error.unwrap_err().to_string(),
            "This server does not share presence"
        );
        let own = fetch_from(|| Err(forbidden()), offline, &user("me")).await;
        assert!(matches!(own, Err(Error::PresenceUnavailable(_))), "{own:?}");

        // No presence endpoint at all.
        let unrecognized = || {
            Err(server_error(
                StatusCode::NOT_FOUND,
                ErrorKind::Unrecognized,
                "Unrecognized request",
            ))
        };
        let error = fetch_from(offline, unrecognized, &user("bob")).await;
        assert!(
            matches!(error, Err(Error::PresenceUnavailable(_))),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_refusal_for_one_user_is_their_error() {
        // Synapse refuses the presence of someone we share no room with,
        // and answers ours.
        let error = fetch_from(offline, || Err(forbidden()), &user("stranger")).await;
        match error {
            Err(Error::Http(e)) => {
                assert_eq!(e.client_api_error_kind(), Some(&ErrorKind::Forbidden));
            }
            other => panic!("{other:?}"),
        }
        let missing = || {
            Err(server_error(
                StatusCode::NOT_FOUND,
                ErrorKind::NotFound,
                "Presence state for this user was not found",
            ))
        };
        let error = fetch_from(offline, missing, &user("bob")).await;
        assert!(matches!(error, Err(Error::Http(_))), "{error:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn our_presence_is_refreshed_until_the_server_refuses() {
        let start = Instant::now();
        let sends = Arc::new(Mutex::new(Vec::new()));
        let refuse_from = 3;
        keep_alive(|| {
            let sends = sends.clone();
            async move {
                let mut sends = sends.lock().unwrap();
                sends.push(start.elapsed());
                if sends.len() == 1 {
                    return Err(Error::Sync("network".into()));
                }
                if sends.len() >= refuse_from {
                    return Err(Error::PresenceUnavailable("M_FORBIDDEN".into()));
                }
                Ok(())
            }
        })
        .await;
        // A network error does not stop it; the refusal does.
        assert_eq!(
            *sends.lock().unwrap(),
            [KEEP_ALIVE, KEEP_ALIVE * 2, KEEP_ALIVE * 3]
        );
    }

    /// Answers from a script, and when each user was read.
    #[derive(Clone, Default)]
    struct Fake {
        reads: Arc<Mutex<Vec<(OwnedUserId, Duration)>>>,
        answers: Arc<Mutex<HashMap<OwnedUserId, Result<Presence, String>>>>,
        start: Option<Instant>,
        calls: Arc<AtomicUsize>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                start: Some(Instant::now()),
                ..Default::default()
            }
        }

        fn answer(&self, user: &UserId, answer: Result<Presence, String>) {
            self.answers.lock().unwrap().insert(user.to_owned(), answer);
        }

        fn reads(&self) -> Vec<(OwnedUserId, Duration)> {
            self.reads.lock().unwrap().clone()
        }
    }

    impl PresenceSource for Fake {
        async fn fetch(&self, user: &UserId) -> Result<Presence> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let elapsed = self.start.unwrap().elapsed();
            self.reads.lock().unwrap().push((user.to_owned(), elapsed));
            match self.answers.lock().unwrap().get(user).cloned() {
                Some(Ok(presence)) => Ok(presence),
                Some(Err(answer)) if answer == "unavailable" => {
                    Err(Error::PresenceUnavailable(answer))
                }
                Some(Err(answer)) => Err(Error::Sync(answer)),
                None => Ok(online()),
            }
        }
    }

    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn users_are_read_at_once_then_spread_over_the_minute() {
        let fake = Fake::new();
        let users: Vec<_> = ["a", "b", "c", "d"].map(user).into();
        let watcher = PresenceWatcher::spawn(fake.clone(), users.clone());
        let mut updates = Box::pin(watcher.updates());
        for user in &users {
            let (got, _) = updates.next().await.unwrap().unwrap();
            assert_eq!(&got, user);
        }
        tokio::time::sleep(Duration::from_secs(140)).await;
        let reads = fake.reads();
        let secs = |d: Duration| d.as_secs();
        let times: Vec<_> = reads
            .iter()
            .map(|(u, d)| (u.localpart(), secs(*d)))
            .collect();
        assert_eq!(
            times,
            [
                ("a", 0),
                ("b", 0),
                ("c", 0),
                ("d", 0),
                ("a", 60),
                ("b", 75),
                ("c", 90),
                ("d", 105),
                ("a", 120),
                ("b", 135),
            ]
        );
        // Unchanged presences yield nothing more.
        settle().await;
        assert!(futures_util::FutureExt::now_or_never(updates.next()).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn changes_are_yielded_and_late_readers_get_what_is_known() {
        let fake = Fake::new();
        let (a, b) = (user("a"), user("b"));
        let watcher = PresenceWatcher::spawn(fake.clone(), [a.clone(), b.clone(), a.clone()]);
        let mut first = Box::pin(watcher.updates());
        assert_eq!(first.next().await.unwrap().unwrap(), (a.clone(), online()));
        assert_eq!(first.next().await.unwrap().unwrap(), (b.clone(), online()));

        let idle = Presence {
            state: PresenceState::Unavailable,
            currently_active: false,
            ..online()
        };
        fake.answer(&b, Ok(idle.clone()));
        assert_eq!(
            first.next().await.unwrap().unwrap(),
            (b.clone(), idle.clone())
        );
        // a was read again at 60 s, unchanged; b at 90 s.
        assert_eq!(
            Instant::now() - fake.start.unwrap(),
            Duration::from_secs(90)
        );

        let late: Vec<_> = watcher.updates().take(2).collect().await;
        let mut late: Vec<_> = late.into_iter().map(Result::unwrap).collect();
        late.sort_by(|x, y| x.0.cmp(&y.0));
        assert_eq!(late, [(a, online()), (b, idle)]);
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_read_without_readers() {
        let fake = Fake::new();
        let watcher = PresenceWatcher::spawn(fake.clone(), [user("a")]);
        tokio::time::sleep(Duration::from_secs(300)).await;
        assert!(fake.reads().is_empty());

        let mut updates = Box::pin(watcher.updates());
        updates.next().await.unwrap().unwrap();
        assert_eq!(fake.reads().len(), 1);
        drop(updates);
        tokio::time::sleep(Duration::from_secs(300)).await;
        assert_eq!(fake.reads().len(), 1);

        // Back: read at once, as it is overdue.
        let _updates = watcher.updates();
        settle().await;
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert_eq!(fake.reads().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn errors_skip_a_user_and_unavailable_ends_the_watch() {
        let fake = Fake::new();
        let (a, b) = (user("a"), user("b"));
        fake.answer(&a, Err("unreachable".into()));
        let watcher = PresenceWatcher::spawn(fake.clone(), [a.clone(), b.clone()]);
        let mut updates = Box::pin(watcher.updates());
        assert_eq!(
            updates.next().await.unwrap().unwrap(),
            (b.clone(), online())
        );

        fake.answer(&a, Err("unavailable".into()));
        let end = updates.next().await.unwrap();
        assert!(matches!(end, Err(Error::PresenceUnavailable(_))), "{end:?}");
        assert!(updates.next().await.is_none());
        let calls = fake.calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(300)).await;
        assert_eq!(fake.calls.load(Ordering::SeqCst), calls);

        // A later reader learns it too.
        let late: Vec<_> = watcher.updates().collect().await;
        assert!(matches!(
            late.as_slice(),
            [Ok(_), Err(Error::PresenceUnavailable(_))]
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_the_watcher_ends_its_updates() {
        let fake = Fake::new();
        let watcher = PresenceWatcher::spawn(fake.clone(), [user("a")]);
        let mut updates = Box::pin(watcher.updates());
        updates.next().await.unwrap().unwrap();
        drop(watcher);
        assert!(updates.next().await.is_none());
    }

    #[test]
    fn long_lists_keep_a_minimum_gap() {
        let now = Instant::now();
        let due = [Some(now), Some(now)];
        let last = Some(now);
        assert_eq!(
            next_read(&due, last, MIN_GAP, now),
            Some((0, now + MIN_GAP))
        );
        assert_eq!(
            next_read(&[None, Some(now)], last, MIN_GAP, now),
            Some((0, now))
        );
        assert_eq!(next_read(&[], None, MIN_GAP, now), None);
    }
}
