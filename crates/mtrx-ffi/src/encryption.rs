//! End-to-end encryption: recovery with the recovery key, and emoji
//! verification with another of our devices.

use std::sync::Arc;

use futures_util::StreamExt;
use mtrx_core as core;

use crate::{
    MtrxError, Session, TaskHandle,
    runtime::{InRuntime, follow, forward, run},
};

/// Whether this device can read encrypted messages, from
/// `Session.subscribeEncryptionState`, now and after every change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EncryptionState {
    /// Signed by our cross-signing identity: other devices and users trust
    /// it, and it gets the keys of new messages.
    pub device_verified: bool,
    pub recovery: RecoveryStatus,
    /// Message keys are being fetched from the key backup (after the
    /// recovery key or a verification, and when a room's timeline opens):
    /// undecryptable messages may decrypt meanwhile.
    #[uniffi(default)]
    pub keys_arriving: bool,
}

impl From<core::EncryptionState> for EncryptionState {
    fn from(state: core::EncryptionState) -> Self {
        Self {
            device_verified: state.device_verified,
            recovery: state.recovery.into(),
            keys_arriving: state.keys_arriving,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RecoveryStatus {
    /// Not known: the server could not be asked.
    Unknown,
    /// Recovery is set up and this device has all its secrets.
    Enabled,
    /// The account has no recovery.
    Disabled,
    /// The account has recovery but this device lacks its secrets: enter the
    /// recovery key, or verify with another device.
    Incomplete,
}

impl From<core::RecoveryStatus> for RecoveryStatus {
    fn from(status: core::RecoveryStatus) -> Self {
        match status {
            core::RecoveryStatus::Unknown => Self::Unknown,
            core::RecoveryStatus::Enabled => Self::Enabled,
            core::RecoveryStatus::Disabled => Self::Disabled,
            core::RecoveryStatus::Incomplete => Self::Incomplete,
        }
    }
}

/// A step of `Session.recover`, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RecoveryStep {
    CheckingKey,
    /// Importing the cross-signing identity (this device becomes verified)
    /// and turning on the key backup.
    ImportingIdentity,
    /// Fetching message keys from the backup, room by room (the most
    /// recently active encrypted rooms first).
    DownloadingKeys {
        done: u32,
        total: u32,
    },
}

impl From<core::RecoveryStep> for RecoveryStep {
    fn from(step: core::RecoveryStep) -> Self {
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        match step {
            core::RecoveryStep::CheckingKey => Self::CheckingKey,
            core::RecoveryStep::ImportingIdentity => Self::ImportingIdentity,
            core::RecoveryStep::DownloadingKeys { done, total } => Self::DownloadingKeys {
                done: count(done),
                total: count(total),
            },
        }
    }
}

/// One of our devices (sessions).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DeviceInfo {
    pub id: String,
    pub name: Option<String>,
    /// Signed by our cross-signing identity.
    pub verified: bool,
    /// This device.
    pub current: bool,
}

impl From<core::DeviceInfo> for DeviceInfo {
    fn from(device: core::DeviceInfo) -> Self {
        Self {
            id: device.id.to_string(),
            name: device.name,
            verified: device.verified,
            current: device.current,
        }
    }
}

/// Where a [`Verification`] is.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum VerificationState {
    /// Waiting for the other device (to accept, or to start comparing).
    Waiting,
    /// Show these, in this order, and ask whether the other device shows
    /// the same; answer with `confirm()` or `mismatch()`.
    Emojis { emojis: Vec<VerificationEmoji> },
    /// We confirmed; waiting for the other device to confirm.
    Confirmed,
    /// Both confirmed; the devices exchange keys. Keep the app open until
    /// `Done`.
    SharingKeys,
    /// Verified.
    Done,
    /// Ended without verifying (by either device, a mismatch, or a timeout).
    Cancelled { reason: String },
}

/// One of the seven emoji to compare.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct VerificationEmoji {
    /// The emoji itself, e.g. "🐶".
    pub symbol: String,
    /// Its English name, e.g. "Dog"; show it under the symbol.
    pub description: String,
}

impl From<core::VerificationState> for VerificationState {
    fn from(state: core::VerificationState) -> Self {
        match state {
            core::VerificationState::Waiting => Self::Waiting,
            core::VerificationState::Emojis(emojis) => Self::Emojis {
                emojis: emojis
                    .into_iter()
                    .map(|e| VerificationEmoji {
                        symbol: e.symbol,
                        description: e.description,
                    })
                    .collect(),
            },
            core::VerificationState::Confirmed => Self::Confirmed,
            core::VerificationState::SharingKeys => Self::SharingKeys,
            core::VerificationState::Done => Self::Done,
            core::VerificationState::Cancelled(reason) => Self::Cancelled { reason },
        }
    }
}

/// Receives the encryption state, now and after every change.
#[uniffi::export(with_foreign)]
pub trait EncryptionStateListener: Send + Sync {
    fn on_update(&self, state: EncryptionState);
}

/// Receives the steps of `Session.recover` as they start.
#[uniffi::export(with_foreign)]
pub trait RecoveryListener: Send + Sync {
    fn on_update(&self, step: RecoveryStep);
}

/// Receives a verification's state, now and after every change.
#[uniffi::export(with_foreign)]
pub trait VerificationListener: Send + Sync {
    fn on_update(&self, state: VerificationState);
}

/// Receives verification requests from other devices.
#[uniffi::export(with_foreign)]
pub trait VerificationRequestListener: Send + Sync {
    fn on_request(&self, verification: Arc<Verification>);
}

/// An emoji verification with another device: one we asked for
/// (`Session.requestSelfVerification`) or one we received
/// (`VerificationRequestListener`, then `accept()`). The protocol runs in
/// the background while the object lives; dropping it stops following it
/// (the other device then times out).
#[derive(uniffi::Object)]
pub struct Verification {
    inner: InRuntime<Arc<core::Verification>>,
}

impl Verification {
    fn new(verification: core::Verification) -> Arc<Self> {
        Arc::new(Self {
            inner: InRuntime::new(Arc::new(verification)),
        })
    }

    fn core(&self) -> Arc<core::Verification> {
        Arc::clone(&self.inner)
    }
}

#[uniffi::export]
impl Verification {
    /// The user whose device this is: our own ID for a self-verification.
    pub fn other_user(&self) -> String {
        self.inner.other_user().to_string()
    }

    /// Accept a request we received.
    pub async fn accept(&self) -> Result<(), MtrxError> {
        let verification = self.core();
        run(async move { Ok(verification.accept().await?) }).await
    }

    pub fn state(&self) -> VerificationState {
        self.inner.current().into()
    }

    /// Call `listener` with the state, now and on every change. Ends after
    /// `Done` or `Cancelled`.
    pub fn subscribe(&self, listener: Arc<dyn VerificationListener>) -> Arc<TaskHandle> {
        let states = self.inner.states();
        follow(states, VerificationState::from, move |state| {
            listener.on_update(state)
        })
    }

    /// The emoji match (in `Emojis`).
    pub async fn confirm(&self) -> Result<(), MtrxError> {
        let verification = self.core();
        run(async move { Ok(verification.confirm().await?) }).await
    }

    /// The emoji differ (in `Emojis`): cancels, and tells the other device.
    pub async fn mismatch(&self) -> Result<(), MtrxError> {
        let verification = self.core();
        run(async move { Ok(verification.mismatch().await?) }).await
    }

    /// Stop, at any step before `Done` (also to decline a request).
    pub async fn cancel(&self) -> Result<(), MtrxError> {
        let verification = self.core();
        run(async move { Ok(verification.cancel().await?) }).await
    }
}

#[uniffi::export]
impl Session {
    /// Call `listener` with the encryption state, now and on every change
    /// (a verification, a recovery, keys arriving and done).
    pub fn subscribe_encryption_state(
        &self,
        listener: Arc<dyn EncryptionStateListener>,
    ) -> Arc<TaskHandle> {
        let session = self.core();
        TaskHandle::spawn(async move {
            let states = session.encryption_states().await;
            forward(states, EncryptionState::from, |state| {
                listener.on_update(state)
            })
            .await;
        })
    }

    /// Unlock encryption with the recovery key (as the user typed it; spaces
    /// are fine): this device becomes verified, and message keys are
    /// fetched from the backup, so encrypted messages decrypt, also in open
    /// timelines. `listener` gets each step. A wrong key fails at
    /// `CheckingKey` with `MtrxError.Other`. Sync must be running.
    pub async fn recover(
        &self,
        recovery_key: String,
        listener: Arc<dyn RecoveryListener>,
    ) -> Result<(), MtrxError> {
        let session = self.core();
        run(async move {
            Ok(session
                .recover(&recovery_key, |step| listener.on_update(step.into()))
                .await?)
        })
        .await
    }

    /// Our devices, this one included.
    pub async fn devices(&self) -> Result<Vec<DeviceInfo>, MtrxError> {
        let session = self.core();
        run(async move {
            Ok(session
                .devices()
                .await?
                .into_iter()
                .map(DeviceInfo::from)
                .collect())
        })
        .await
    }

    /// Ask our other devices to verify this one; one of them accepts and
    /// both then compare emoji. Sync must be running.
    pub async fn request_self_verification(&self) -> Result<Arc<Verification>, MtrxError> {
        let session = self.core();
        run(async move {
            Ok(Verification::new(
                session.request_self_verification().await?,
            ))
        })
        .await
    }

    /// Call `listener` with each verification request another device sends
    /// us while sync runs; `accept()` or `cancel()` it.
    pub fn subscribe_verification_requests(
        &self,
        listener: Arc<dyn VerificationRequestListener>,
    ) -> Arc<TaskHandle> {
        let mut requests = Box::pin(self.core().verification_requests());
        TaskHandle::spawn(async move {
            while let Some(verification) = requests.next().await {
                listener.on_request(Verification::new(verification));
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_states_carry_the_emoji_in_order() {
        let emoji = |symbol: &str, description: &str| core::VerificationEmoji {
            symbol: symbol.into(),
            description: description.into(),
        };
        let state = VerificationState::from(core::VerificationState::Emojis(vec![
            emoji("🐶", "Dog"),
            emoji("🔑", "Key"),
        ]));
        assert_eq!(
            state,
            VerificationState::Emojis {
                emojis: vec![
                    VerificationEmoji {
                        symbol: "🐶".into(),
                        description: "Dog".into()
                    },
                    VerificationEmoji {
                        symbol: "🔑".into(),
                        description: "Key".into()
                    },
                ]
            }
        );
        assert_eq!(
            VerificationState::from(core::VerificationState::Cancelled("timeout".into())),
            VerificationState::Cancelled {
                reason: "timeout".into()
            }
        );
    }

    #[test]
    fn recovery_steps_count_rooms() {
        assert_eq!(
            RecoveryStep::from(core::RecoveryStep::DownloadingKeys { done: 2, total: 20 }),
            RecoveryStep::DownloadingKeys { done: 2, total: 20 }
        );
        assert_eq!(
            RecoveryStatus::from(core::RecoveryStatus::Incomplete),
            RecoveryStatus::Incomplete
        );
    }

    #[test]
    fn encryption_states_say_when_keys_arrive() {
        assert_eq!(
            EncryptionState::from(core::EncryptionState {
                device_verified: true,
                recovery: core::RecoveryStatus::Enabled,
                keys_arriving: true,
            }),
            EncryptionState {
                device_verified: true,
                recovery: RecoveryStatus::Enabled,
                keys_arriving: true,
            }
        );
    }
}
