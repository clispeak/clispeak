//! Desktop key storage.
//!
//! Lives here rather than in `voicecast-core` on purpose: the `keyring` crate
//! pulls in platform backends, and core has to compile for all five targets.
//! Keeping it in the desktop binary means the portability gate stays honest
//! without anyone having to remember a rule.

use std::path::{Path, PathBuf};

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
    /// Records that a key was once stored in the keyring.
    ///
    /// Without it, "the keyring is unreachable" is indistinguishable from
    /// "this device has no identity yet", and the second reading silently
    /// mints a *new* identity — changing the device's address and evicting it
    /// from every space it belongs to. Holds no secret, only the fact that
    /// one exists.
    marker: PathBuf,
}

impl DesktopKeyStore {
    /// Build a store using the platform keyring where possible.
    pub fn new() -> Result<Self, IdentityError> {
        Ok(Self {
            fallback: FileKeyStore::default_location()?,
            marker: voicecast_core::config_dir()?.join("identity.in-keyring"),
        })
    }

    /// Whether a key was previously stored in the keyring.
    fn keyring_was_used(&self) -> bool {
        self.marker.exists()
    }

    /// Record that the keyring holds this device's key.
    ///
    /// Best-effort: failing to write the marker weakens the protection but
    /// must not stop the node from starting.
    fn mark_keyring_used(&self) {
        if self.marker.exists() {
            return;
        }
        if let Some(parent) = self.marker.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.marker, b"keyring\n");
    }

    /// An entry handle, or `None` if the keyring is unusable here.
    ///
    /// Skipped entirely when `VOICECAST_CONFIG_DIR` is set: a second node on
    /// the same machine must not share the first one's keyring entry, or both
    /// would end up with the same identity.
    fn entry() -> Option<Entry> {
        if std::env::var_os("VOICECAST_CONFIG_DIR").is_some() {
            return None;
        }
        Entry::new(SERVICE, ACCOUNT).ok()
    }
}

/// What to do given what each store holds and whether the keyring was used before.
///
/// Pulled out as a pure function because the interesting case — refusing to
/// mint a new identity when the keyring has gone missing — is impossible to
/// exercise against a live keyring without breaking D-Bus, which hangs.
fn decide(
    from_keyring: Option<[u8; 32]>,
    from_file: Option<[u8; 32]>,
    keyring_was_used: bool,
    marker: &Path,
) -> Result<Option<[u8; 32]>, IdentityError> {
    if let Some(key) = from_keyring {
        return Ok(Some(key));
    }
    if let Some(key) = from_file {
        return Ok(Some(key));
    }

    // Nothing anywhere. If a key was stored in the keyring before, this is a
    // broken or cleared keyring rather than a first run — and generating a
    // fresh identity would silently change this device's address and evict it
    // from every space it belongs to.
    if keyring_was_used {
        return Err(IdentityError::Store(format!(
            "this device has an identity in the system keyring, but it cannot be read. \
             Refusing to generate a new one, which would change this device's address \
             and remove it from its spaces. Unlock or start the keyring and retry, or \
             delete {} to deliberately start over",
            marker.display()
        )));
    }

    Ok(None)
}

impl KeyStore for DesktopKeyStore {
    fn load(&self) -> Result<Option<[u8; 32]>, IdentityError> {
        let mut from_keyring = None;
        if let Some(entry) = Self::entry()
            && let Ok(secret) = entry.get_secret()
        {
            let arr: [u8; 32] = secret.as_slice().try_into().map_err(|_| {
                IdentityError::Malformed(format!(
                    "keyring holds {} bytes, expected 32",
                    secret.len()
                ))
            })?;
            // Also recorded here, not only on save: a device provisioned
            // before the marker existed would otherwise never gain one, and
            // the protection above would never engage for it.
            self.mark_keyring_used();
            from_keyring = Some(arr);
        }

        decide(
            from_keyring,
            self.fallback.load()?,
            self.keyring_was_used(),
            &self.marker,
        )
    }

    fn save(&self, key: &[u8; 32]) -> Result<(), IdentityError> {
        if let Some(entry) = Self::entry()
            && entry.set_secret(key).is_ok()
        {
            self.mark_keyring_used();
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

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7u8; 32];

    fn marker() -> &'static Path {
        Path::new("/tmp/voicecast-marker-test")
    }

    #[test]
    fn keyring_key_wins() {
        assert_eq!(decide(Some(KEY), None, true, marker()).unwrap(), Some(KEY));
    }

    #[test]
    fn file_key_is_used_when_the_keyring_has_none() {
        assert_eq!(decide(None, Some(KEY), false, marker()).unwrap(), Some(KEY));
    }

    #[test]
    fn first_run_reports_no_identity_rather_than_failing() {
        assert_eq!(decide(None, None, false, marker()).unwrap(), None);
    }

    #[test]
    fn a_vanished_keyring_key_is_an_error_not_a_new_identity() {
        // The bug this guards: the daemon quietly minted a different device id
        // when the keyring was unreachable, which would evict it from every
        // space it belongs to.
        let err = decide(None, None, true, marker()).expect_err("must refuse");
        let message = err.to_string();
        assert!(
            message.contains("Refusing to generate"),
            "unhelpful: {message}"
        );
        assert!(
            message.contains("voicecast-marker-test"),
            "should say how to override"
        );
    }
}
