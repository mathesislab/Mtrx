//! Personal settings stored as account data, so they sync to the user's
//! devices and nobody else sees them: room categories, room list colours,
//! browsed directory servers.
//!
//! Changes show at once: [`Synced::change`] publishes the new value before
//! saving it on the server, and values synced from other devices replace it
//! unless a change of ours is still being saved.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use matrix_sdk::{
    Client,
    event_handler::EventHandlerHandle,
    ruma::events::{GlobalAccountDataEvent, GlobalAccountDataEventContent, StaticEventContent},
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
        Live::from_watch(self.value.subscribe())
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

async fn stored<T: AccountData>(client: &Client) -> T {
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
