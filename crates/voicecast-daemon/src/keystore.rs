//! Desktop key storage.
//!
//! Lives here rather than in `voicecast-core` on purpose: the `keyring` crate
//! pulls in platform backends, and core has to compile for all five targets.
//! Keeping it in the desktop binary means the portability gate stays honest
//! without anyone having to remember a rule.

use keyring::v1::Entry;
use voicecast_core::{FileKeyStore, IdentityError, KeyStore};

/// Namespace under which the key is filed in the system keyring.
const SERVICE: &str = "voicecast";
/// Which key — there is one identity per device, so the name is fixed.
const ACCOUNT: &str = "device-identity";

/// The system keyring, falling back to a file when unavailable.
///
/// A headless box, a session without a running secret service, or a locked
/// keyring all mean "no keyring right now" rather than "no identity". Falling
/// back keeps the tool usable, and [`describe`](KeyStore::describe) reports
/// which one is actually in use so the fallback is never silent.
pub struct DesktopKeyStore {
    fallback: FileKeyStore,
}

impl DesktopKeyStore {
    /// Build a store using the platform keyring where possible.
    pub fn new() -> Result<Self, IdentityError> {
        Ok(Self {
            fallback: FileKeyStore::default_location()?,
        })
    }

    /// An entry handle, or `None` if the keyring is unusable here.
    fn entry() -> Option<Entry> {
        Entry::new(SERVICE, ACCOUNT).ok()
    }
}

impl KeyStore for DesktopKeyStore {
    fn load(&self) -> Result<Option<[u8; 32]>, IdentityError> {
        if let Some(entry) = Self::entry()
            && let Ok(secret) = entry.get_secret()
        {
            let arr: [u8; 32] = secret.as_slice().try_into().map_err(|_| {
                IdentityError::Malformed(format!(
                    "keyring holds {} bytes, expected 32",
                    secret.len()
                ))
            })?;
            return Ok(Some(arr));
        }
        // Not an error: either the keyring is unavailable, or this device
        // simply has no identity yet. The file store distinguishes those.
        self.fallback.load()
    }

    fn save(&self, key: &[u8; 32]) -> Result<(), IdentityError> {
        if let Some(entry) = Self::entry()
            && entry.set_secret(key).is_ok()
        {
            return Ok(());
        }
        self.fallback.save(key)
    }

    fn describe(&self) -> String {
        match Self::entry() {
            Some(entry) if entry.get_secret().is_ok() => "system keyring".to_string(),
            _ => self.fallback.describe(),
        }
    }
}
