//! One secret value, kept in the OS keyring, a plain file, or a store the
//! app supplies (the mobile apps' Keychain and Keystore).

use std::{
    fmt, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{Error, Result};

#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod os_keyring;
#[cfg(windows)]
mod windows;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub(crate) use os_keyring::NO_SECRET_STORE;

/// Where secrets (access tokens, store passphrases) are kept.
#[derive(Clone)]
pub enum SecretStore {
    /// OS credential store: macOS Keychain, Windows Credential Manager, or
    /// Secret Service on Linux. On Windows the credential is a key and the
    /// secret an encrypted file next to it (see `secrets/windows.rs`).
    /// Not on iOS and Android, where the app supplies a [`SecretStore::Custom`].
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    Keyring,
    /// Plain-text file in the data directory. For tests and development only.
    File,
    /// A store supplied by the app.
    Custom(Arc<dyn SecretBackend>),
}

impl fmt::Debug for SecretStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(not(any(target_os = "ios", target_os = "android")))]
            Self::Keyring => f.write_str("Keyring"),
            Self::File => f.write_str("File"),
            Self::Custom(_) => f.write_str("Custom"),
        }
    }
}

/// Secret storage supplied by the app, e.g. the iOS Keychain or the Android
/// Keystore. It serves one data root, so secrets are named relative to it
/// (`profiles/<name>`): the root's path may change, as iOS moves an app's
/// container when the app is updated.
pub trait SecretBackend: Send + Sync {
    /// The secret `key`, or `None` if there is none.
    fn load(&self, key: &str) -> std::result::Result<Option<String>, String>;
    /// Store `value` as the secret `key`, replacing any earlier one.
    fn save(&self, key: &str, value: &str) -> std::result::Result<(), String>;
    /// Remove the secret `key`; succeeds if there is none.
    fn delete(&self, key: &str) -> std::result::Result<(), String>;
}

/// A secret slot: keyring entry `name`, `file`, or the app's secret `key`,
/// depending on the store.
#[derive(Clone, Debug)]
pub struct Secret {
    store: SecretStore,
    /// Unique across data roots. Only the OS keyring uses it.
    #[cfg_attr(any(target_os = "ios", target_os = "android"), allow(dead_code))]
    name: String,
    /// Relative to the data root.
    key: String,
    file: PathBuf,
}

impl Secret {
    pub fn new(store: SecretStore, name: String, key: String, file: PathBuf) -> Self {
        Self {
            store,
            name,
            key,
            file,
        }
    }

    pub fn load(&self) -> Result<Option<String>> {
        match &self.store {
            #[cfg(not(any(target_os = "ios", target_os = "android")))]
            SecretStore::Keyring => self.keyring_load(),
            SecretStore::File => match fs::read_to_string(&self.file) {
                Ok(value) => Ok(Some(value)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            },
            SecretStore::Custom(backend) => backend.load(&self.key).map_err(Error::SecretStorage),
        }
    }

    pub fn save(&self, value: &str) -> Result<()> {
        match &self.store {
            #[cfg(not(any(target_os = "ios", target_os = "android")))]
            SecretStore::Keyring => self.keyring_save(value),
            SecretStore::File => Ok(write_private(&self.file, value)?),
            SecretStore::Custom(backend) => {
                backend.save(&self.key, value).map_err(Error::SecretStorage)
            }
        }
    }

    /// Remove the secret; succeeds if there is none.
    pub fn delete(&self) -> Result<()> {
        match &self.store {
            #[cfg(not(any(target_os = "ios", target_os = "android")))]
            SecretStore::Keyring => self.keyring_delete(),
            SecretStore::File => match fs::remove_file(&self.file) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.into()),
            },
            SecretStore::Custom(backend) => backend.delete(&self.key).map_err(Error::SecretStorage),
        }
    }
}

fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use super::*;

    /// An app's store, in memory.
    #[derive(Default)]
    struct Memory(Mutex<HashMap<String, String>>);

    impl SecretBackend for Memory {
        fn load(&self, key: &str) -> std::result::Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }

        fn save(&self, key: &str, value: &str) -> std::result::Result<(), String> {
            self.0.lock().unwrap().insert(key.into(), value.into());
            Ok(())
        }

        fn delete(&self, key: &str) -> std::result::Result<(), String> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }

    struct Failing;

    impl SecretBackend for Failing {
        fn load(&self, _: &str) -> std::result::Result<Option<String>, String> {
            Err("locked".into())
        }

        fn save(&self, _: &str, _: &str) -> std::result::Result<(), String> {
            Err("locked".into())
        }

        fn delete(&self, _: &str) -> std::result::Result<(), String> {
            Err("locked".into())
        }
    }

    #[test]
    fn custom_store_gets_the_relative_key() {
        let memory = Arc::new(Memory::default());
        let secret = Secret::new(
            SecretStore::Custom(memory.clone()),
            "/data/profiles/p".into(),
            "profiles/p".into(),
            PathBuf::from("/data/profiles/p/secrets.json"),
        );
        assert_eq!(secret.load().unwrap(), None);
        secret.save("value").unwrap();
        assert_eq!(
            memory
                .0
                .lock()
                .unwrap()
                .get("profiles/p")
                .map(String::as_str),
            Some("value")
        );
        assert_eq!(secret.load().unwrap().as_deref(), Some("value"));
        secret.delete().unwrap();
        secret.delete().unwrap();
        assert_eq!(secret.load().unwrap(), None);
    }

    #[test]
    fn custom_store_errors_carry_its_message() {
        let secret = Secret::new(
            SecretStore::Custom(Arc::new(Failing)),
            String::new(),
            "profiles/p".into(),
            PathBuf::new(),
        );
        let error = secret.load().unwrap_err();
        assert!(
            matches!(&error, Error::SecretStorage(m) if m == "locked"),
            "{error}"
        );
        assert!(secret.save("value").is_err());
        assert!(secret.delete().is_err());
    }
}
