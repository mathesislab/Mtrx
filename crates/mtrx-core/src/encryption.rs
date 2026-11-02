//! End-to-end encryption: recovery (key backup + secret storage) and
//! interactive emoji verification between our own devices.

use std::{collections::HashSet, sync::Arc};

use futures_util::{Stream, StreamExt, stream};
use matrix_sdk::{
    Client,
    encryption::{
        CrossSigningResetAuthType,
        backups::BackupState,
        recovery::RecoveryError,
        secret_storage::SecretStorageError,
        verification::{
            SasState, SasVerification, Verification as SdkVerification, VerificationRequest,
            VerificationRequestState,
        },
    },
    ruma::{
        DeviceId, api::client::uiaa,
        events::key::verification::request::ToDeviceKeyVerificationRequestEvent,
    },
};
use matrix_sdk_crypto::secret_storage::DecodeError;
use tokio::sync::{Mutex, broadcast, watch};
use tokio_stream::wrappers::{BroadcastStream, WatchStream};
use tracing::warn;

use crate::{AbortOnDrop, Error, Live, OwnedDeviceId, OwnedRoomId, OwnedUserId, Result, Session};

/// Whether this device can read encrypted messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncryptionState {
    /// Signed by our cross-signing identity.
    pub device_verified: bool,
    pub recovery: RecoveryStatus,
    /// Message keys are being fetched from the key backup: the backup is
    /// being turned on (after the recovery key or a verification), or the
    /// keys of rooms are being downloaded ([`Session::recover`], or a room's
    /// timeline opening). Undecryptable messages may decrypt meanwhile.
    pub keys_arriving: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryStatus {
    /// Not known: the server could not be asked.
    Unknown,
    /// Recovery is set up and this device has all secrets.
    Enabled,
    /// No recovery set up for the account.
    Disabled,
    /// Recovery exists but this device lacks the secrets; enter the recovery
    /// key or verify with another device.
    Incomplete,
}

/// Progress of a [`Verification`].
#[derive(Clone, Debug, PartialEq)]
pub enum VerificationState {
    /// Waiting for the other device.
    Waiting,
    /// Compare these with the other device, then call [`Verification::confirm`]
    /// or [`Verification::mismatch`].
    Emojis(Vec<VerificationEmoji>),
    /// We confirmed; waiting for the other device to confirm.
    Confirmed,
    /// Emojis matched; our devices are exchanging cross-signing keys. Both
    /// devices must stay online until [`VerificationState::Done`].
    SharingKeys,
    Done,
    Cancelled(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationEmoji {
    pub symbol: String,
    pub description: String,
}

/// An interactive emoji (SAS) verification with one of our other devices.
///
/// The protocol steps are driven in the background; frontends watch
/// [`Verification::states`] and answer with [`Verification::confirm`] or
/// [`Verification::mismatch`].
pub struct Verification {
    request: VerificationRequest,
    sas: Arc<Mutex<Option<SasVerification>>>,
    state: watch::Receiver<VerificationState>,
    /// Drives the protocol.
    _task: AbortOnDrop,
}

impl Verification {
    fn new(client: Client, request: VerificationRequest) -> Self {
        let (tx, state) = watch::channel(VerificationState::Waiting);
        let sas = Arc::new(Mutex::new(None));
        let task = tokio::spawn(drive(client, request.clone(), sas.clone(), tx));
        Self {
            request,
            sas,
            state,
            _task: AbortOnDrop(task),
        }
    }

    pub fn other_user(&self) -> OwnedUserId {
        self.request.other_user_id().to_owned()
    }

    /// Accept an incoming request.
    pub async fn accept(&self) -> Result<()> {
        Ok(self.request.accept().await?)
    }

    pub fn current(&self) -> VerificationState {
        self.state.borrow().clone()
    }

    /// The state, kept up to date while the verification lives, until
    /// [`VerificationState::Done`] or [`VerificationState::Cancelled`].
    pub fn states(&self) -> Live<VerificationState> {
        Live::from_watch(self.state.clone())
    }

    /// The emojis match.
    pub async fn confirm(&self) -> Result<()> {
        Ok(self.active_sas().await?.confirm().await?)
    }

    /// The emojis differ: cancel and mark the attempt as suspicious.
    pub async fn mismatch(&self) -> Result<()> {
        Ok(self.active_sas().await?.mismatch().await?)
    }

    pub async fn cancel(&self) -> Result<()> {
        match self.sas.lock().await.as_ref() {
            Some(sas) => sas.cancel().await?,
            None => self.request.cancel().await?,
        }
        Ok(())
    }

    async fn active_sas(&self) -> Result<SasVerification> {
        self.sas
            .lock()
            .await
            .clone()
            .ok_or_else(|| Error::Verification("no emoji comparison in progress".into()))
    }
}

/// Run the request until it becomes an emoji verification, then run that.
async fn drive(
    client: Client,
    request: VerificationRequest,
    slot: Arc<Mutex<Option<SasVerification>>>,
    tx: watch::Sender<VerificationState>,
) {
    let mut changes = request.changes();
    let mut state = request.state();
    loop {
        match state {
            VerificationRequestState::Ready { .. } if request.we_started() => {
                match request.start_sas().await {
                    Ok(Some(sas)) => return drive_sas(client, sas, slot, tx).await,
                    Ok(None) => {
                        tx.send_replace(VerificationState::Cancelled(
                            "other device does not support emoji verification".into(),
                        ));
                        return;
                    }
                    Err(e) => {
                        tx.send_replace(VerificationState::Cancelled(e.to_string()));
                        return;
                    }
                }
            }
            VerificationRequestState::Transitioned {
                verification: SdkVerification::SasV1(sas),
            } => {
                return drive_sas(client, sas, slot, tx).await;
            }
            VerificationRequestState::Transitioned { .. } => {
                tx.send_replace(VerificationState::Cancelled("unsupported method".into()));
                return;
            }
            VerificationRequestState::Done => {
                tx.send_replace(VerificationState::Done);
                return;
            }
            VerificationRequestState::Cancelled(info) => {
                tx.send_replace(VerificationState::Cancelled(info.reason().to_owned()));
                return;
            }
            _ => {}
        }
        match changes.next().await {
            Some(next) => state = next,
            None => return,
        }
    }
}

async fn drive_sas(
    client: Client,
    sas: SasVerification,
    slot: Arc<Mutex<Option<SasVerification>>>,
    tx: watch::Sender<VerificationState>,
) {
    *slot.lock().await = Some(sas.clone());
    let mut changes = sas.changes();
    let mut state = sas.state();
    loop {
        match state {
            // The other device started: accept to move on to the key exchange.
            SasState::Started { .. } => {
                if let Err(e) = sas.accept().await {
                    tx.send_replace(VerificationState::Cancelled(e.to_string()));
                    return;
                }
            }
            SasState::KeysExchanged {
                emojis: Some(emojis),
                ..
            } => {
                let emojis = emojis
                    .emojis
                    .iter()
                    .map(|e| VerificationEmoji {
                        symbol: e.symbol.to_owned(),
                        description: e.description.to_owned(),
                    })
                    .collect();
                tx.send_replace(VerificationState::Emojis(emojis));
            }
            SasState::KeysExchanged { emojis: None, .. } => {
                let _ = sas.cancel().await;
                tx.send_replace(VerificationState::Cancelled(
                    "other device does not support emoji verification".into(),
                ));
                return;
            }
            SasState::Confirmed => {
                tx.send_replace(VerificationState::Confirmed);
            }
            SasState::Done { .. } => {
                if sas.other_user_id() == client.user_id().expect("logged in") {
                    tx.send_replace(VerificationState::SharingKeys);
                    let other = sas.other_device().device_id().to_owned();
                    if let Err(e) = wait_cross_signed(&client, &other).await {
                        tx.send_replace(VerificationState::Cancelled(e.to_string()));
                        return;
                    }
                }
                tx.send_replace(VerificationState::Done);
                return;
            }
            SasState::Cancelled(info) => {
                tx.send_replace(VerificationState::Cancelled(info.reason().to_owned()));
                return;
            }
            _ => {}
        }
        match changes.next().await {
            Some(next) => state = next,
            None => return,
        }
    }
}

/// After self-verification the new device fetches the cross-signing keys from
/// the other one and signs itself. Wait until both devices are cross-signed.
async fn wait_cross_signed(client: &Client, other: &DeviceId) -> Result<()> {
    let encryption = client.encryption();
    let user = client.user_id().expect("logged in");
    // Subscribe before checking so no update is missed.
    let mut updates = std::pin::pin!(encryption.devices_stream().await?);
    loop {
        let signed = |device: Option<matrix_sdk::encryption::identities::Device>| {
            device.is_some_and(|d| d.is_cross_signed_by_owner())
        };
        if signed(encryption.get_own_device().await?)
            && signed(encryption.get_device(user, other).await?)
        {
            return Ok(());
        }
        if updates.next().await.is_none() {
            return Err(Error::Verification("device updates ended".into()));
        }
    }
}

/// Forward incoming verification requests to [`Session::verification_requests`].
pub(crate) fn register_handlers(client: &Client) -> broadcast::Sender<VerificationRequest> {
    let (tx, _) = broadcast::channel(8);
    let sender = tx.clone();
    client.add_event_handler(
        move |event: ToDeviceKeyVerificationRequestEvent, client: Client| {
            let sender = sender.clone();
            async move {
                let request = client
                    .encryption()
                    .get_verification_request(&event.sender, &event.content.transaction_id)
                    .await;
                match request {
                    Some(request) => {
                        let _ = sender.send(request);
                    }
                    None => warn!(
                        "verification request {} not found",
                        event.content.transaction_id
                    ),
                }
            }
        },
    );
    tx
}

impl Session {
    /// Whether this device is verified, the account's recovery, and whether
    /// message keys are arriving.
    pub async fn encryption_state(&self) -> Result<EncryptionState> {
        encryption_state(&self.client, &self.room_keys).await
    }

    /// The encryption state, once read, kept up to date while held: as this
    /// device's verification, the recovery, the key backup and the key
    /// downloads change. A reading that fails is tried again at the next
    /// change.
    pub fn encryption_states(&self) -> impl Future<Output = Live<EncryptionState>> + Send + use<> {
        let encryption = self.client.encryption();
        // Followed before the first reading, so no change is missed.
        let mut changes = stream::select_all([
            encryption.verification_state().map(|_| ()).boxed(),
            encryption.recovery().state_stream().map(|_| ()).boxed(),
            encryption.backups().state_stream().map(|_| ()).boxed(),
            WatchStream::new(self.room_keys.arriving.subscribe())
                .map(|_| ())
                .boxed(),
        ]);
        let (client, room_keys) = (self.client.clone(), self.room_keys.clone());
        async move {
            let first = loop {
                if let Some(state) = read_encryption_state(&client, &room_keys).await {
                    break state;
                }
                if changes.next().await.is_none() {
                    // Nothing to read it again on.
                    std::future::pending::<()>().await;
                }
            };
            Live::spawn(first, |states| async move {
                while changes.next().await.is_some() {
                    if let Some(state) = read_encryption_state(&client, &room_keys).await {
                        states.send_if_modified(|current| {
                            let changed = *current != state;
                            *current = state;
                            changed
                        });
                    }
                }
            })
        }
    }

    /// The account's recovery, checked now as the SDK checks it: whether it
    /// is set up, and whether this device holds the secrets it keeps (the
    /// cross-signing keys, and the key backup's unless the backup was turned
    /// off). Not the SDK's record of it, which its checks running at once
    /// can leave stale: one begun before [`Session::recover`] enabled the
    /// backup (on a keys query, as recovery signs this device) can write
    /// last.
    pub async fn recovery_status(&self) -> RecoveryStatus {
        recovery_status(&self.client).await
    }

    /// Set up recovery: back up room keys and store our secrets on the server,
    /// protected by the returned recovery key. Show it to the user once.
    pub async fn enable_recovery(&self) -> Result<String> {
        Ok(self
            .client
            .encryption()
            .recovery()
            .enable()
            .wait_for_backups_to_upload()
            .await?)
    }

    /// Start over when neither another device nor the recovery key can unlock
    /// encryption: creates a new cross-signing identity and recovery, and
    /// returns the new recovery key. Messages only the old keys could decrypt
    /// stay unreadable, and other users see that our identity changed.
    pub async fn reset_encryption(&self, password: &str) -> Result<String> {
        let recovery = self.client.encryption().recovery();
        if let Some(handle) = recovery.reset_identity().await? {
            match handle.auth_type() {
                CrossSigningResetAuthType::Uiaa(uiaa) => {
                    let mut auth =
                        uiaa::Password::new((&self.user_id()).into(), password.to_owned());
                    auth.session = uiaa.session.clone();
                    handle.reset(Some(uiaa::AuthData::Password(auth))).await?;
                }
                CrossSigningResetAuthType::OAuth(_) => {
                    return Err(Error::Verification(
                        "identity reset via OAuth is not supported yet".into(),
                    ));
                }
            }
        }
        self.enable_recovery().await
    }

    /// Unlock recovery on this device: imports the cross-signing keys (this
    /// device becomes verified) and the key backup (old messages decrypt),
    /// then fetches message keys of the most recently active encrypted rooms.
    /// `progress` is called at each step.
    pub async fn recover(&self, recovery_key: &str, progress: impl Fn(RecoveryStep)) -> Result<()> {
        let encryption = self.client.encryption();
        progress(RecoveryStep::CheckingKey);
        // Fails fast on a wrong key; `recover` below opens the store again.
        encryption
            .secret_storage()
            .open_secret_store(recovery_key)
            .await
            .map_err(recovery_key_error)?;
        progress(RecoveryStep::ImportingIdentity);
        // Keys arrive from here on: the backup is turned on, then rooms'
        // keys are downloaded.
        let _arriving = self.room_keys.arriving();
        encryption.recovery().recover(recovery_key).await?;

        let rooms = self.recent_encrypted_rooms(PREFETCH_ROOMS);
        for (done, room) in rooms.iter().enumerate() {
            progress(RecoveryStep::DownloadingKeys {
                done,
                total: rooms.len(),
            });
            self.room_keys.download(&self.client, room).await;
        }
        Ok(())
    }

    fn recent_encrypted_rooms(&self, limit: usize) -> Vec<OwnedRoomId> {
        let mut rooms: Vec<_> = self
            .client
            .joined_rooms()
            .into_iter()
            .filter(|r| r.encryption_state().is_encrypted())
            .collect();
        rooms.sort_by_key(|r| std::cmp::Reverse(r.latest_event_timestamp()));
        rooms
            .into_iter()
            .take(limit)
            .map(|r| r.room_id().to_owned())
            .collect()
    }

    /// Whether this device is verified (signed by our cross-signing identity).
    pub async fn is_device_verified(&self) -> Result<bool> {
        is_device_verified(&self.client).await
    }

    /// Ask our other devices to verify this one.
    pub async fn request_self_verification(&self) -> Result<Verification> {
        let identity = self
            .client
            .encryption()
            .get_user_identity(&self.user_id())
            .await?
            .ok_or_else(|| Error::Verification("account has no cross-signing identity".into()))?;
        let request = identity
            .request_verification()
            .await
            .map_err(|e| Error::Verification(e.to_string()))?;
        Ok(Verification::new(self.client.clone(), request))
    }

    /// Verification requests from our other devices. Sync must be running.
    pub fn verification_requests(&self) -> impl Stream<Item = Verification> + use<> {
        let client = self.client.clone();
        BroadcastStream::new(self.verification_tx.subscribe()).filter_map(move |r| {
            let client = client.clone();
            async move { r.ok().map(|request| Verification::new(client, request)) }
        })
    }

    pub async fn devices(&self) -> Result<Vec<DeviceInfo>> {
        let devices = self
            .client
            .encryption()
            .get_user_devices(&self.user_id())
            .await?;
        Ok(devices
            .devices()
            .map(|d| DeviceInfo {
                id: d.device_id().to_owned(),
                name: d.display_name().map(ToOwned::to_owned),
                verified: d.is_cross_signed_by_owner(),
                current: d.device_id() == self.device_id(),
            })
            .collect())
    }
}

async fn encryption_state(client: &Client, room_keys: &RoomKeys) -> Result<EncryptionState> {
    let backup = client.encryption().backups().state();
    Ok(EncryptionState {
        device_verified: is_device_verified(client).await?,
        recovery: recovery_status(client).await,
        keys_arriving: keys_arriving(backup, room_keys.downloading()),
    })
}

/// [`encryption_state`], or none if it could not be read.
async fn read_encryption_state(client: &Client, room_keys: &RoomKeys) -> Option<EncryptionState> {
    (encryption_state(client, room_keys).await)
        .inspect_err(|e| warn!("could not read the encryption state: {e}"))
        .ok()
}

/// While the backup is turned on (`Enabling`; `Downloading` only with the
/// SDK's one-shot download, which we do not use) or rooms' keys download.
fn keys_arriving(backup: BackupState, downloading: bool) -> bool {
    downloading || matches!(backup, BackupState::Enabling | BackupState::Downloading)
}

async fn is_device_verified(client: &Client) -> Result<bool> {
    let device = client
        .encryption()
        .get_own_device()
        .await?
        .ok_or_else(|| Error::Verification("own device not found".into()))?;
    Ok(device.is_cross_signed_by_owner())
}

async fn recovery_status(client: &Client) -> RecoveryStatus {
    match check_recovery(client).await {
        Ok(status) => status,
        Err(e) => {
            warn!("checking recovery failed: {e}");
            RecoveryStatus::Unknown
        }
    }
}

async fn check_recovery(client: &Client) -> Result<RecoveryStatus> {
    let encryption = client.encryption();
    if !encryption.secret_storage().is_enabled().await? {
        return Ok(RecoveryStatus::Disabled);
    }
    let cross_signing = encryption
        .cross_signing_status()
        .await
        .is_some_and(|status| status.is_complete());
    let backup = encryption.backups().are_enabled().await || backup_turned_off(client).await?;
    Ok(if cross_signing && backup {
        RecoveryStatus::Enabled
    } else {
        RecoveryStatus::Incomplete
    })
}

/// Whether the key backup was turned off for the account: MSC4287's
/// `m.key_backup`, else its unstable form.
async fn backup_turned_off(client: &Client) -> Result<bool> {
    let account = client.account();
    if let Some(content) = account.fetch_account_data("m.key_backup".into()).await? {
        return Ok(content.get_field::<bool>("enabled").ok().flatten() == Some(false));
    }
    Ok(account
        .fetch_account_data("m.org.matrix.custom.backup_disabled".into())
        .await?
        .and_then(|content| content.get_field::<bool>("disabled").ok().flatten())
        .unwrap_or(false))
}

/// A key that is not a recovery key (not Base58, the wrong length, prefix or
/// parity) or not the backup's (the MAC check fails) is the user's mistake;
/// other failures are not.
fn recovery_key_error(error: SecretStorageError) -> Error {
    match error {
        SecretStorageError::SecretStorageKey(
            DecodeError::Base58(_)
            | DecodeError::KeyLength(..)
            | DecodeError::Prefix(..)
            | DecodeError::Parity(..)
            | DecodeError::Mac(_),
        ) => Error::InvalidRecoveryKey,
        error => RecoveryError::from(error).into(),
    }
}

/// Encrypted rooms whose message keys [`Session::recover`] fetches right away.
const PREFETCH_ROOMS: usize = 20;

/// Progress of [`Session::recover`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryStep {
    CheckingKey,
    /// Importing the cross-signing identity (verifies this device) and
    /// enabling the key backup.
    ImportingIdentity,
    /// Fetching message keys from the backup, room by room.
    DownloadingKeys {
        done: usize,
        total: usize,
    },
}

/// Fetches a room's message keys from the key backup in one request, once per
/// session. Without this the SDK fetches keys one by one as decryption fails,
/// which is slow for rooms with long encrypted history.
pub(crate) struct RoomKeys {
    done: std::sync::Mutex<HashSet<OwnedRoomId>>,
    /// The number of [`Arriving`] guards alive.
    arriving: watch::Sender<usize>,
}

impl Default for RoomKeys {
    fn default() -> Self {
        Self {
            done: Default::default(),
            arriving: watch::channel(0).0,
        }
    }
}

/// Keys are arriving while one lives.
pub(crate) struct Arriving(watch::Sender<usize>);

impl Drop for Arriving {
    fn drop(&mut self) {
        self.0.send_modify(|n| *n -= 1);
    }
}

impl RoomKeys {
    /// Marks keys as arriving until dropped.
    pub(crate) fn arriving(&self) -> Arriving {
        self.arriving.send_modify(|n| *n += 1);
        Arriving(self.arriving.clone())
    }

    fn downloading(&self) -> bool {
        *self.arriving.borrow() > 0
    }

    /// Without a backup on this device (before the recovery key or a
    /// verification) there is nothing to fetch: the room is tried again the
    /// next time.
    pub(crate) async fn download(&self, client: &Client, room: &OwnedRoomId) {
        let backups = client.encryption().backups();
        if self.done.lock().unwrap().contains(room) || !backups.are_enabled().await {
            return;
        }
        let _arriving = self.arriving();
        match backups.download_room_keys_for_room(room).await {
            Ok(()) => {
                self.done.lock().unwrap().insert(room.clone());
            }
            Err(e) => warn!(%room, "downloading room keys from backup failed: {e}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: OwnedDeviceId,
    pub name: Option<String>,
    pub verified: bool,
    pub current: bool,
}

#[cfg(test)]
mod tests {
    use matrix_sdk_crypto::secret_storage::SecretStorageKey;

    use super::*;

    /// Why `key` does not open the secret store of `backup`.
    fn decode_error(key: &str, backup: &SecretStorageKey) -> DecodeError {
        SecretStorageKey::from_account_data(key, backup.event_content().clone())
            .expect_err("the key is refused")
    }

    #[test]
    fn mistyped_and_other_keys_are_invalid() {
        let backup = SecretStorageKey::new();
        let right = backup.to_base58();
        let other = SecretStorageKey::new().to_base58();
        let short = right[..right.len() - 5].to_owned();
        for key in ["not a recovery key", "0OIl", &short, &other] {
            let error = SecretStorageError::SecretStorageKey(decode_error(key, &backup));
            assert!(
                matches!(recovery_key_error(error), Error::InvalidRecoveryKey),
                "{key}"
            );
        }
        assert!(
            SecretStorageKey::from_account_data(&right, backup.event_content().clone()).is_ok()
        );
        assert_eq!(
            Error::InvalidRecoveryKey.to_string(),
            "The recovery key is not valid"
        );
    }

    #[test]
    fn keys_arrive_while_the_backup_turns_on_or_rooms_download() {
        assert!(keys_arriving(BackupState::Enabling, false));
        assert!(keys_arriving(BackupState::Downloading, false));
        assert!(keys_arriving(BackupState::Enabled, true));
        for state in [
            BackupState::Unknown,
            BackupState::Creating,
            BackupState::Resuming,
            BackupState::Enabled,
            BackupState::Disabling,
        ] {
            assert!(!keys_arriving(state, false), "{state:?}");
        }
    }

    #[test]
    fn keys_arrive_until_the_last_guard_drops() {
        let keys = RoomKeys::default();
        let mut changes = keys.arriving.subscribe();
        assert!(!keys.downloading());
        let recovering = keys.arriving();
        let room = keys.arriving();
        assert!(keys.downloading());
        drop(room);
        assert!(keys.downloading());
        changes.mark_unchanged();
        drop(recovering);
        assert!(!keys.downloading());
        assert!(changes.has_changed().unwrap());
    }

    #[test]
    fn other_secret_storage_failures_are_not_the_key() {
        let missing = SecretStorageError::MissingKeyInfo { key_id: None };
        assert!(matches!(
            recovery_key_error(missing),
            Error::Recovery(RecoveryError::SecretStorage(
                SecretStorageError::MissingKeyInfo { .. }
            ))
        ));
    }
}
