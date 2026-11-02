//! Drafts: what one wrote in a room's composer and did not send, with the
//! message it answers, kept on this device (the SDK's store) until sent or
//! cleared. Matrix has no way to share them between devices.
//!
//! [`DraftKeeper`] keeps a room's draft as the app's composer changes.

use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_util::{Stream, future::BoxFuture};
use matrix_sdk::{
    Client, ComposerDraft, ComposerDraftType, Room,
    ruma::{OwnedEventId, OwnedRoomId, RoomId},
};
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;
use tracing::warn;

use crate::{Error, Result, Session, session::AbortOnDrop};

/// How long writing pauses before the draft is kept.
pub const DRAFT_PAUSE: Duration = Duration::from_millis(1500);

/// A room's unsent message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub text: String,
    /// The message it answers.
    pub reply_to: Option<OwnedEventId>,
}

impl Draft {
    /// Nothing to keep.
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.reply_to.is_none()
    }
}

/// The rooms with a draft, for the room list to mark.
#[derive(Clone)]
pub(crate) struct Drafts {
    rooms: Arc<watch::Sender<BTreeSet<OwnedRoomId>>>,
}

impl Drafts {
    /// The rooms of `client` with a draft in its store.
    pub(crate) async fn load(client: &Client) -> Self {
        let mut rooms = BTreeSet::new();
        for room in client.rooms() {
            match room.load_composer_draft(None).await {
                Ok(Some(_)) => {
                    rooms.insert(room.room_id().to_owned());
                }
                Ok(None) => {}
                Err(e) => warn!(room = %room.room_id(), "could not read the draft: {e}"),
            }
        }
        Self {
            rooms: Arc::new(watch::Sender::new(rooms)),
        }
    }

    fn set(&self, room_id: &RoomId, has_draft: bool) {
        self.rooms.send_if_modified(|rooms| {
            if has_draft {
                rooms.insert(room_id.to_owned())
            } else {
                rooms.remove(room_id)
            }
        });
    }
}

/// The draft left in `room`, if any.
async fn load(room: &Room) -> Result<Option<Draft>> {
    let Some(draft) = room.load_composer_draft(None).await? else {
        return Ok(None);
    };
    let reply_to = match draft.draft_type {
        ComposerDraftType::Reply { event_id } => Some(event_id),
        ComposerDraftType::NewMessage | ComposerDraftType::Edit { .. } => None,
    };
    Ok(Some(Draft {
        text: draft.plain_text,
        reply_to,
    }))
}

/// Keep `draft` for `room`; an empty one clears it.
async fn save(room: &Room, drafts: &Drafts, draft: &Draft) -> Result<()> {
    if draft.is_empty() {
        room.clear_composer_draft(None).await?;
    } else {
        let draft_type = match &draft.reply_to {
            Some(event_id) => ComposerDraftType::Reply {
                event_id: event_id.clone(),
            },
            None => ComposerDraftType::NewMessage,
        };
        room.save_composer_draft(
            ComposerDraft {
                plain_text: draft.text.clone(),
                html_text: None,
                draft_type,
                attachments: Vec::new(),
            },
            None,
        )
        .await?;
    }
    drafts.set(room.room_id(), !draft.is_empty());
    Ok(())
}

impl Session {
    /// The draft left in room `room_id`, if any.
    pub async fn draft(&self, room_id: &RoomId) -> Result<Option<Draft>> {
        load(&self.room(room_id)?).await
    }

    /// Keep `draft` for room `room_id`; an empty one clears it. The apps
    /// keep drafts through [`Session::draft_keeper`].
    pub async fn save_draft(&self, room_id: &RoomId, draft: &Draft) -> Result<()> {
        save(&self.room(room_id)?, &self.drafts, draft).await
    }

    /// The rooms with a draft now, then after every change.
    pub fn rooms_with_drafts_updates(&self) -> impl Stream<Item = BTreeSet<OwnedRoomId>> + use<> {
        WatchStream::new(self.drafts.rooms.subscribe())
    }

    /// Keeps the draft of room `room` as its composer changes.
    pub fn draft_keeper(&self, room: &RoomId) -> DraftKeeper {
        DraftKeeper::new(Box::new(RoomDraft {
            client: self.client.clone(),
            room: room.to_owned(),
            drafts: self.drafts.clone(),
        }))
    }
}

/// Where a [`DraftKeeper`] reads and keeps its room's draft: the store, or
/// a fake.
trait DraftStore: Send + Sync + 'static {
    fn load(&self) -> BoxFuture<'_, Result<Option<Draft>>>;
    fn save<'a>(&'a self, draft: &'a Draft) -> BoxFuture<'a, Result<()>>;
}

/// A room's draft in the SDK's store.
struct RoomDraft {
    client: Client,
    room: OwnedRoomId,
    drafts: Drafts,
}

impl RoomDraft {
    fn room(&self) -> Result<Room> {
        (self.client.get_room(&self.room)).ok_or_else(|| Error::RoomNotFound(self.room.to_string()))
    }
}

impl DraftStore for RoomDraft {
    fn load(&self) -> BoxFuture<'_, Result<Option<Draft>>> {
        Box::pin(async move { load(&self.room()?).await })
    }

    fn save<'a>(&'a self, draft: &'a Draft) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move { save(&self.room()?, &self.drafts, draft).await })
    }
}

/// Keeps a room's draft on this device as the app's composer changes: the
/// app brings back the one left in the room ([`DraftKeeper::restore`]),
/// says what is written after every change ([`DraftKeeper::changed`]), and
/// saves it at once when the room is left or the app goes to the
/// background ([`DraftKeeper::flush`]). Each draft is saved once, in order;
/// an empty one (sent) clears it. Leaving a room keeps its draft.
///
/// Whether a restored draft replaces what is in the composer is the app's
/// to decide: only if nothing was written meanwhile.
pub struct DraftKeeper {
    keeper: Arc<Keeper>,
}

struct Keeper {
    store: Box<dyn DraftStore>,
    state: Mutex<State>,
    /// Restoring and saves run one at a time, in order.
    queue: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct State {
    restored: bool,
    kept: Kept,
    /// The draft as the app last said it is; none before its first change.
    written: Option<Draft>,
    /// Saves the draft once writing pauses for [`DRAFT_PAUSE`].
    timer: Option<AbortOnDrop>,
}

/// The draft as the store has it, as far as we know.
#[derive(Default)]
enum Kept {
    /// Not read yet: only something written replaces it.
    #[default]
    Unread,
    Known(Draft),
    /// A save failed: the next one replaces it, whatever it is.
    Unknown,
}

impl State {
    /// The draft to save: the one written, if it is to replace the one kept.
    fn to_save(&self) -> Option<Draft> {
        let written = self.written.as_ref()?;
        let replaces = match &self.kept {
            Kept::Unread => !written.is_empty(),
            Kept::Known(kept) => kept != written,
            Kept::Unknown => true,
        };
        replaces.then(|| written.clone())
    }
}

impl DraftKeeper {
    fn new(store: Box<dyn DraftStore>) -> Self {
        Self {
            keeper: Arc::new(Keeper {
                store,
                state: Mutex::default(),
                queue: tokio::sync::Mutex::default(),
            }),
        }
    }

    /// The draft left in the room, read once: `None` if there is none, if
    /// it cannot be read, or on later calls.
    pub async fn restore(&self) -> Option<Draft> {
        let again = std::mem::replace(&mut self.keeper.state.lock().unwrap().restored, true);
        if again {
            return None;
        }
        let _turn = self.keeper.queue.lock().await;
        let left = match self.keeper.store.load().await {
            Ok(left) => left,
            Err(e) => {
                warn!("could not read the draft: {e}");
                None
            }
        };
        // Unless a save came first: then the store has that.
        let mut state = self.keeper.state.lock().unwrap();
        if let Kept::Unread = state.kept {
            state.kept = Kept::Known(left.clone().unwrap_or_default());
        }
        left
    }

    /// What is written now: saved once writing pauses for [`DRAFT_PAUSE`],
    /// unless it is the one kept (before that one was read, unless it is
    /// empty). During an edit, pass the draft from before it (an edit is
    /// not a draft). Needs a tokio runtime.
    pub fn changed(&self, draft: Draft) {
        let mut state = self.keeper.state.lock().unwrap();
        state.written = Some(draft);
        let keeper = self.keeper.clone();
        state.timer = Some(AbortOnDrop(tokio::spawn(async move {
            tokio::time::sleep(DRAFT_PAUSE).await;
            // Apart from the timer, which the next change stops.
            tokio::spawn(keeper.save());
        })));
    }

    /// Save the draft now, after the saves under way: leaving the room, or
    /// the app going to the background.
    pub async fn flush(&self) {
        self.keeper.state.lock().unwrap().timer = None;
        self.keeper.clone().save().await;
    }
}

impl Keeper {
    /// Save the draft as last written, if it is to replace the one kept.
    async fn save(self: Arc<Self>) {
        let _turn = self.queue.lock().await;
        let Some(draft) = self.state.lock().unwrap().to_save() else {
            return;
        };
        let saved = self.store.save(&draft).await;
        self.state.lock().unwrap().kept = match saved {
            Ok(()) => Kept::Known(draft),
            Err(e) => {
                warn!("could not keep the draft: {e}");
                Kept::Unknown
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::Instant;

    use super::*;

    /// A store that records each save and when it was made, and fails
    /// while `fail` is set. Saves take `slow` to finish.
    #[derive(Clone, Default)]
    struct FakeStore {
        left: Arc<Mutex<Option<Draft>>>,
        loads: Arc<Mutex<usize>>,
        saves: Arc<Mutex<Vec<(u64, String)>>>,
        saving: Arc<Mutex<bool>>,
        fail: Arc<Mutex<bool>>,
        slow: Duration,
        start: Option<Instant>,
    }

    impl FakeStore {
        fn new() -> Self {
            Self {
                start: Some(Instant::now()),
                ..Default::default()
            }
        }

        fn keeper(&self) -> DraftKeeper {
            DraftKeeper::new(Box::new(self.clone()))
        }

        /// The text of each save, and at which tenth of a second.
        fn saves(&self) -> Vec<(u64, String)> {
            self.saves.lock().unwrap().clone()
        }
    }

    impl DraftStore for FakeStore {
        fn load(&self) -> BoxFuture<'_, Result<Option<Draft>>> {
            *self.loads.lock().unwrap() += 1;
            Box::pin(async move { Ok(self.left.lock().unwrap().clone()) })
        }

        fn save<'a>(&'a self, draft: &'a Draft) -> BoxFuture<'a, Result<()>> {
            Box::pin(async move {
                let overlapping = std::mem::replace(&mut *self.saving.lock().unwrap(), true);
                assert!(!overlapping, "one save at a time");
                tokio::time::sleep(self.slow).await;
                *self.saving.lock().unwrap() = false;
                let fail = *self.fail.lock().unwrap();
                if fail {
                    return Err(Error::Timeline("store locked".into()));
                }
                let at = self.start.unwrap().elapsed().as_millis() as u64 / 100;
                self.saves.lock().unwrap().push((at, draft.text.clone()));
                *self.left.lock().unwrap() = Some(draft.clone()).filter(|d| !d.is_empty());
                Ok(())
            })
        }
    }

    fn text(text: &str) -> Draft {
        Draft {
            text: text.into(),
            reply_to: None,
        }
    }

    async fn sleep_ms(ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn saved_once_writing_pauses() {
        let store = FakeStore::new();
        let keeper = store.keeper();
        assert_eq!(keeper.restore().await, None);
        keeper.changed(text("H"));
        sleep_ms(1000).await;
        keeper.changed(text("Hi"));
        sleep_ms(1400).await;
        assert_eq!(store.saves(), []);
        sleep_ms(100).await;
        assert_eq!(store.saves(), [(25, "Hi".into())]);
        // Nothing new to keep.
        keeper.changed(text("Hi"));
        sleep_ms(2000).await;
        assert_eq!(store.saves().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn the_draft_left_is_read_once_and_not_saved_again() {
        let store = FakeStore::new();
        *store.left.lock().unwrap() = Some(text("Half written"));
        let keeper = store.keeper();
        assert_eq!(keeper.restore().await, Some(text("Half written")));
        assert_eq!(keeper.restore().await, None);
        assert_eq!(*store.loads.lock().unwrap(), 1);
        // The app put it in the composer.
        keeper.changed(text("Half written"));
        keeper.flush().await;
        assert_eq!(store.saves(), []);
        keeper.changed(text(""));
        keeper.flush().await;
        assert_eq!(store.saves(), [(0, "".into())]);
        assert_eq!(*store.left.lock().unwrap(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn only_something_written_replaces_the_draft_before_it_is_read() {
        let store = FakeStore::new();
        *store.left.lock().unwrap() = Some(text("Half written"));
        let keeper = store.keeper();
        keeper.changed(text(""));
        keeper.flush().await;
        assert_eq!(store.saves(), []);
        keeper.changed(text("Other"));
        keeper.flush().await;
        assert_eq!(store.saves(), [(0, "Other".into())]);
        // What the store has now.
        assert_eq!(keeper.restore().await, Some(text("Other")));
    }

    #[tokio::test(start_paused = true)]
    async fn flushing_saves_at_once_after_the_saves_under_way() {
        let store = FakeStore {
            slow: Duration::from_millis(500),
            ..FakeStore::new()
        };
        let keeper = store.keeper();
        keeper.restore().await;
        keeper.changed(text("One"));
        sleep_ms(1500).await;
        // "One" is being saved: "Two" waits for it.
        keeper.changed(text("Two"));
        keeper.flush().await;
        assert_eq!(store.saves(), [(20, "One".into()), (25, "Two".into())]);
        // The timer was stopped.
        sleep_ms(2000).await;
        assert_eq!(store.saves().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_save_is_tried_again_with_the_next_change() {
        let store = FakeStore::new();
        let keeper = store.keeper();
        keeper.restore().await;
        *store.fail.lock().unwrap() = true;
        keeper.changed(text("Hello"));
        keeper.flush().await;
        *store.fail.lock().unwrap() = false;
        assert_eq!(store.saves(), []);
        keeper.flush().await;
        assert_eq!(store.saves(), [(0, "Hello".into())]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_change_is_saved_after_the_keeper_is_dropped() {
        let store = FakeStore::new();
        let keeper = store.keeper();
        keeper.restore().await;
        keeper.changed(text("Bye"));
        drop(keeper);
        sleep_ms(1500).await;
        assert_eq!(store.saves(), [(15, "Bye".into())]);
    }
}
