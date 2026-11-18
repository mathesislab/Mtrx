//! Personal settings stored as account data, so they sync to the user's
//! devices and nobody else sees them: room categories, room list colours,
//! browsed directory servers ([`Synced`]); and per room, rooms' disappearing
//! modes ([`RoomSynced`]).
//!
//! Changes show at once: [`Synced::change`] publishes the new value before
//! saving it on the server, and values synced from other devices replace it
//! unless a change of ours is still being saved.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use matrix_sdk::{
    Client, Room,
    event_handler::EventHandlerHandle,
    ruma::{
        OwnedRoomId, RoomId,
        events::{
            GlobalAccountDataEvent, GlobalAccountDataEventContent, RoomAccountDataEvent,
            RoomAccountDataEventContent, StaticEventContent,
        },
    },
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::{Mutex, watch};
use tracing::warn;

use crate::{Live, Result};

/// A value kept as account data, as event content of its own type.
pub(crate) trait AccountData: Clone + Default + PartialEq + Send + Sync + 'static {
    type Content: GlobalAccountDataEventContent
        + StaticEventContent<IsPrefix = matrix_sdk::ruma::events::False>
        + Serialize
        + DeserializeOwned
        + Clone
        + Send
        + Sync
        + 'static;

    fn from_content(content: Self::Content) -> Self;
    fn into_content(self) -> Self::Content;
}

/// The session's copy of an [`AccountData`] value.
pub(crate) struct Synced<T: AccountData> {
    client: Client,
    value: Arc<watch::Sender<T>>,
    /// Changes not yet saved; synced values are stale meanwhile.
    unsaved: Arc<AtomicUsize>,
    /// Saves one at a time, so the last change is saved last.
    save: Mutex<()>,
    handler: EventHandlerHandle,
}

impl<T: AccountData> Synced<T> {
    pub(crate) async fn load(client: Client) -> Self {
        let value = Arc::new(watch::Sender::new(stored::<T>(&client).await));
        let unsaved = Arc::new(AtomicUsize::new(0));
        let handler = client.add_event_handler({
            let (value, unsaved) = (value.clone(), unsaved.clone());
            move |event: GlobalAccountDataEvent<T::Content>| async move {
                if unsaved.load(Ordering::SeqCst) == 0 {
                    let new = T::from_content(event.content);
                    value.send_if_modified(|current| {
                        let changed = *current != new;
                        *current = new;
                        changed
                    });
                }
            }
        });
        Self {
            client,
            value,
            unsaved,
            save: Mutex::new(()),
            handler,
        }
    }

    pub(crate) fn get(&self) -> T {
        self.value.borrow().clone()
    }

    /// The current value, then every change, from this device or others.
    pub(crate) fn updates(&self) -> Live<T> {
        Live::from_watch(self.subscribe())
    }

    /// The current value, then every change; for following it in a task.
    pub(crate) fn subscribe(&self) -> watch::Receiver<T> {
        self.value.subscribe()
    }

    /// Change the value: published at once, then saved on the server; if
    /// saving fails, the server's value is restored.
    pub(crate) async fn change(&self, change: impl FnOnce(&mut T)) -> Result<()> {
        self.unsaved.fetch_add(1, Ordering::SeqCst);
        self.value.send_modify(change);
        let saved = {
            let _guard = self.save.lock().await;
            let content = self.value.borrow().clone().into_content();
            self.client.account().set_account_data(content).await
        };
        self.unsaved.fetch_sub(1, Ordering::SeqCst);
        if let Err(e) = saved {
            self.value.send_replace(stored::<T>(&self.client).await);
            return Err(e.into());
        }
        Ok(())
    }
}

impl<T: AccountData> Drop for Synced<T> {
    fn drop(&mut self) {
        self.client.remove_event_handler(self.handler.clone());
    }
}

/// The value as stored, or the default if it can't be read.
pub(crate) async fn stored<T: AccountData>(client: &Client) -> T {
    let raw = match client.account().account_data::<T::Content>().await {
        Ok(raw) => raw,
        Err(e) => {
            warn!("could not read {}: {e}", T::Content::TYPE);
            return T::default();
        }
    };
    match raw.map(|raw| raw.deserialize()).transpose() {
        Ok(content) => content.map(T::from_content).unwrap_or_default(),
        Err(e) => {
            warn!("could not read {}: {e}", T::Content::TYPE);
            T::default()
        }
    }
}

/// A value kept as room account data, as event content of its own type; a
/// room may have none.
pub(crate) trait RoomAccountData: 'static {
    type Value: Clone + PartialEq + Send + Sync + 'static;
    type Content: RoomAccountDataEventContent
        + StaticEventContent<IsPrefix = matrix_sdk::ruma::events::False>
        + Serialize
        + DeserializeOwned
        + Clone
        + Send
        + Sync
        + 'static;

    /// `None`: the content says there is none (account data can't be
    /// deleted).
    fn from_content(content: Self::Content) -> Option<Self::Value>;
    fn into_content(value: Option<Self::Value>) -> Self::Content;
}

/// The session's copies of a [`RoomAccountData`] value, by room: a
/// [`Synced`] per room. Loaded from the joined rooms, then followed.
pub(crate) struct RoomSynced<T: RoomAccountData> {
    copies: Arc<Copies<T::Value>>,
    /// Saves one at a time, so the last change is saved last.
    save: Mutex<()>,
    client: Client,
    handler: EventHandlerHandle,
}

impl<T: RoomAccountData> RoomSynced<T> {
    pub(crate) async fn load(client: Client) -> Self {
        let mut values = BTreeMap::new();
        for room in client.joined_rooms() {
            if let Some(value) = stored_in::<T>(&room).await {
                values.insert(room.room_id().to_owned(), value);
            }
        }
        let copies = Arc::new(Copies::new(values));
        let handler = client.add_event_handler({
            let copies = copies.clone();
            move |event: RoomAccountDataEvent<T::Content>, room: Room| async move {
                copies.synced(room.room_id(), T::from_content(event.content));
            }
        });
        Self {
            copies,
            save: Mutex::new(()),
            client,
            handler,
        }
    }

    pub(crate) fn get(&self, room: &RoomId) -> Option<T::Value> {
        self.copies.get(room)
    }

    /// The rooms' values, then every change; for following one room's.
    pub(crate) fn subscribe(&self) -> watch::Receiver<BTreeMap<OwnedRoomId, T::Value>> {
        self.copies.values.subscribe()
    }

    /// The rooms' values, then every change, from this device or others.
    pub(crate) fn updates(&self) -> Live<BTreeMap<OwnedRoomId, T::Value>> {
        Live::from_watch(self.subscribe())
    }

    /// Change `room`'s value: published at once, then saved on the server;
    /// if saving fails, the server's value is restored. A change that
    /// leaves the value as it was is not saved.
    pub(crate) async fn change(
        &self,
        room: &Room,
        change: impl FnOnce(&mut Option<T::Value>),
    ) -> Result<()> {
        let id = room.room_id();
        if !self.copies.change(id, change) {
            return Ok(());
        }
        let saved = {
            let _guard = self.save.lock().await;
            let content = T::into_content(self.get(id));
            room.set_account_data(content).await
        };
        let restored = match &saved {
            Ok(_) => None,
            Err(_) => Some(stored_in::<T>(room).await),
        };
        self.copies.saved(id, restored);
        saved?;
        Ok(())
    }

    /// Set `room`'s value, or none; see [`Self::change`].
    pub(crate) async fn set(&self, room: &Room, value: Option<T::Value>) -> Result<()> {
        self.change(room, |current| *current = value).await
    }
}

impl<T: RoomAccountData> Drop for RoomSynced<T> {
    fn drop(&mut self) {
        self.client.remove_event_handler(self.handler.clone());
    }
}

/// [`RoomSynced`]'s values by room, and the rooms with changes of ours not
/// yet saved, whose synced values are stale meanwhile.
struct Copies<V> {
    values: watch::Sender<BTreeMap<OwnedRoomId, V>>,
    /// How many changes are unsaved, by room.
    unsaved: std::sync::Mutex<HashMap<OwnedRoomId, usize>>,
}

impl<V: Clone + PartialEq> Copies<V> {
    fn new(values: BTreeMap<OwnedRoomId, V>) -> Self {
        Self {
            values: watch::Sender::new(values),
            unsaved: Default::default(),
        }
    }

    fn get(&self, room: &RoomId) -> Option<V> {
        self.values.borrow().get(room).cloned()
    }

    /// `room`'s value as synced: shown unless a change of ours is unsaved.
    fn synced(&self, room: &RoomId, value: Option<V>) {
        let unsaved = self.unsaved.lock().unwrap();
        if !unsaved.contains_key(room) {
            self.put(room, value);
        }
    }

    /// A change of ours, shown at once; unless it left the value as it
    /// was (false), unsaved until [`Self::saved`].
    fn change(&self, room: &RoomId, change: impl FnOnce(&mut Option<V>)) -> bool {
        let mut unsaved = self.unsaved.lock().unwrap();
        let changed = self.values.send_if_modified(|values| {
            let mut value = values.get(room).cloned();
            let before = value.clone();
            change(&mut value);
            let changed = value != before;
            if changed {
                set_in(values, room, value);
            }
            changed
        });
        if changed {
            *unsaved.entry(room.to_owned()).or_default() += 1;
        }
        changed
    }

    /// A change of ours saved, or not: then the server's value, `restored`,
    /// shows again.
    fn saved(&self, room: &RoomId, restored: Option<Option<V>>) {
        let mut unsaved = self.unsaved.lock().unwrap();
        if let Some(count) = unsaved.get_mut(room) {
            *count -= 1;
            if *count == 0 {
                unsaved.remove(room);
            }
        }
        if let Some(value) = restored {
            self.put(room, value);
        }
    }

    /// Publish `room`'s `value`, if it changed.
    fn put(&self, room: &RoomId, value: Option<V>) {
        self.values.send_if_modified(|values| {
            let changed = values.get(room) != value.as_ref();
            if changed {
                set_in(values, room, value);
            }
            changed
        });
    }
}

fn set_in<V>(values: &mut BTreeMap<OwnedRoomId, V>, room: &RoomId, value: Option<V>) {
    match value {
        Some(value) => {
            values.insert(room.to_owned(), value);
        }
        None => {
            values.remove(room);
        }
    }
}

/// `room`'s value as stored.
async fn stored_in<T: RoomAccountData>(room: &Room) -> Option<T::Value> {
    let (kind, id) = (T::Content::TYPE, room.room_id());
    let raw = match room.account_data_static::<T::Content>().await {
        Ok(raw) => raw?,
        Err(e) => {
            warn!("could not read {kind} of {id}: {e}");
            return None;
        }
    };
    match raw.deserialize() {
        Ok(event) => T::from_content(event.content),
        Err(e) => {
            warn!("could not read {kind} of {id}: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::{owned_room_id, room_id};

    use super::*;

    fn copies() -> Copies<u32> {
        Copies::new(BTreeMap::from([(owned_room_id!("!a:x"), 1)]))
    }

    #[test]
    fn a_change_shows_at_once() {
        let copies = copies();
        let a = room_id!("!a:x");
        assert!(copies.change(a, |v| *v = Some(2)));
        assert_eq!(copies.get(a), Some(2));
        copies.saved(a, None);
        assert!(copies.change(a, |v| *v = None));
        assert_eq!(copies.get(a), None);
        copies.saved(a, None);
        // Nothing changed, so nothing to save.
        assert!(!copies.change(a, |v| *v = None));
        assert!(copies.unsaved.lock().unwrap().is_empty());
    }

    #[test]
    fn synced_values_wait_while_a_change_is_unsaved() {
        let copies = copies();
        let (a, b) = (room_id!("!a:x"), room_id!("!b:x"));
        assert!(copies.change(a, |v| *v = Some(2)));
        // The echo of an older change, while ours is saved.
        copies.synced(a, Some(1));
        assert_eq!(copies.get(a), Some(2));
        // Another room's are taken.
        copies.synced(b, Some(5));
        assert_eq!(copies.get(b), Some(5));
        copies.saved(a, None);
        copies.synced(a, Some(3));
        assert_eq!(copies.get(a), Some(3));
    }

    #[test]
    fn a_change_not_saved_is_rolled_back() {
        let copies = copies();
        let a = room_id!("!a:x");
        assert!(copies.change(a, |v| *v = Some(2)));
        copies.saved(a, Some(Some(1)));
        assert_eq!(copies.get(a), Some(1));
        // Synced values are taken again.
        copies.synced(a, None);
        assert_eq!(copies.get(a), None);
    }
}
