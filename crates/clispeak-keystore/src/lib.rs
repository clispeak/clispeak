//! Desktop key storage.
//!
//! Its own crate rather than part of `clispeak-core` on purpose: the
//! `keyring` crate pulls in platform backends, and core has to compile for
//! all five targets. Confining it here means the portability gate stays
//! honest without anyone having to remember a rule, and CI simply excludes
//! this crate from the mobile jobs.
//!
//! Shared by `clispeakd` and the desktop app so both are the *same device*.
//! When they each picked their own store they generated separate identities
//! while reading one roster, which looked like the roster was corrupt.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use clispeak_core::{FileKeyStore, IdentityError, KeyStore};
use keyring::v1::Entry;

/// Namespace under which the key is filed in the system keyring.
const SERVICE: &str = "clispeak";

/// What the service was called before the project was renamed.
const PREVIOUS_SERVICE: &str = "voicecast";
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
    /// What the keyring said, asked once and then remembered.
    ///
    /// `describe()` used to call `get_secret()` a second time on every start.
    /// On macOS each call is a keychain prompt, so starting a node asked the
    /// user twice for the same secret — once to load the identity and once to
    /// print a line saying where it came from. Issue #83.
    asked: Mutex<Option<Keyring>>,
}

/// What the platform keyring had to say, in a form that survives being cached.
#[derive(Clone, Debug)]
enum Keyring {
    /// It held a key of the right size.
    Held([u8; 32]),
    /// It answered, and holds nothing for this device.
    Empty,
    /// It held something that is not a key.
    Malformed(usize),
    /// It exists but would not answer — locked, or a broken session bus.
    ///
    /// Kept apart from [`Empty`] because the two need opposite responses and
    /// collapsing them told someone whose keychain was locked that their key
    /// was in a file. The `keyring` crate's own message is carried along, so
    /// the reason survives to whoever reads it.
    ///
    /// [`Empty`]: Keyring::Empty
    Unreadable(String),
    /// No keyring on this platform, or none this build can reach.
    Missing,
}

impl DesktopKeyStore {
    /// Build a store using the platform keyring where possible.
    pub fn new() -> Result<Self, IdentityError> {
        Ok(Self {
            fallback: FileKeyStore::default_location()?,
            marker: clispeak_core::config_dir()?.join("identity.in-keyring"),
            asked: Mutex::new(None),
        })
    }

    /// Whether a key was previously stored in the keyring.
    fn keyring_was_used(&self) -> bool {
        self.marker.exists()
    }

    /// Ask the keyring, at most once for the life of this store.
    ///
    /// Every path that wants to know goes through here, which is what makes
    /// "once" true rather than "once per caller who remembered".
    fn ask(&self) -> Keyring {
        let mut cached = self.asked.lock().expect("keyring cache");
        if let Some(answer) = cached.as_ref() {
            return answer.clone();
        }
        let answer = Self::probe();
        *cached = Some(answer.clone());
        answer
    }

    /// The one place that actually touches the keyring.
    fn probe() -> Keyring {
        let Some(entry) = Self::entry() else {
            return Keyring::Missing;
        };
        match entry.get_secret() {
            Ok(secret) => match <[u8; 32]>::try_from(secret.as_slice()) {
                Ok(key) => Keyring::Held(key),
                Err(_) => Keyring::Malformed(secret.len()),
            },
            // The one error that genuinely means "nothing here". Everything
            // else is the keyring refusing, which is a different situation
            // and used to read identically.
            //
            // Nothing under the current name may still mean something under
            // the previous one, on a machine that ran this project before it
            // was renamed.
            Err(keyring::Error::NoEntry) => match Self::adopt_previous() {
                Some(key) => Keyring::Held(key),
                None => Keyring::Empty,
            },
            Err(e) => Keyring::Unreadable(e.to_string()),
        }
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
    /// Skipped entirely when `CLISPEAK_CONFIG_DIR` is set: a second node on
    /// the same machine must not share the first one's keyring entry, or both
    /// would end up with the same identity.
    fn entry() -> Option<Entry> {
        if std::env::var_os("CLISPEAK_CONFIG_DIR").is_some() {
            return None;
        }
        Entry::new(SERVICE, ACCOUNT).ok()
    }

    /// The same handle under the name this project used to have.
    fn previous_entry() -> Option<Entry> {
        if std::env::var_os("CLISPEAK_CONFIG_DIR").is_some() {
            return None;
        }
        Entry::new(PREVIOUS_SERVICE, ACCOUNT).ok()
    }

    /// Move a key stored under the project's previous name.
    ///
    /// The service name is part of the address of a keyring item, so renaming
    /// the project orphaned every desktop identity — the config directory
    /// migrates, but the keyring is a different store and `migrate_from` never
    /// touched it. The failure would not have been silent: `decide` refuses to
    /// mint a fresh identity when the marker says one was in the keyring, so a
    /// desktop would have stopped with a clear message instead. Loud is better
    /// than quiet and worse than working (decision 82).
    ///
    /// **Written under the new name before the old one is removed.** If the
    /// write fails the key is still where it was; if the delete fails both
    /// exist and the new one is what gets read. The order that loses a key is
    /// the other one.
    fn adopt_previous() -> Option<[u8; 32]> {
        let old = Self::previous_entry()?;
        let secret = old.get_secret().ok()?;
        let key = <[u8; 32]>::try_from(secret.as_slice()).ok()?;
        Self::entry()?.set_secret(&key).ok()?;
        let _ = old.delete_credential();
        Some(key)
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
        let from_keyring = match self.ask() {
            Keyring::Held(key) => {
                // Also recorded here, not only on save: a device provisioned
                // before the marker existed would otherwise never gain one,
                // and the protection below would never engage for it.
                self.mark_keyring_used();
                Some(key)
            }
            Keyring::Malformed(len) => {
                return Err(IdentityError::Malformed(format!(
                    "keyring holds {len} bytes, expected 32"
                )));
            }
            Keyring::Empty | Keyring::Unreadable(_) | Keyring::Missing => None,
        };

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
            // The cache now describes a keyring that no longer holds this,
            // and a stale "empty" here would make `describe` contradict what
            // we just did.
            *self.asked.lock().expect("keyring cache") = Some(Keyring::Held(*key));
            return Ok(());
        }
        self.fallback.save(key)
    }

    fn describe(&self) -> String {
        match self.ask() {
            Keyring::Held(_) => "system keyring".to_string(),
            // Naming the keyring as the thing to deal with. Reporting "file"
            // was true and useless: the key it would read is not there, and
            // nothing told the user that the keychain was the problem.
            Keyring::Unreadable(why) => {
                format!(
                    "{} — the system keyring is present but would not answer ({why}). \
                     Unlock it and restart to use it instead",
                    self.fallback.describe()
                )
            }
            Keyring::Malformed(len) => {
                format!("system keyring, holding {len} bytes where 32 are expected")
            }
            Keyring::Empty | Keyring::Missing => self.fallback.describe(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7u8; 32];

    fn marker() -> &'static Path {
        Path::new("/tmp/clispeak-marker-test")
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
            message.contains("clispeak-marker-test"),
            "should say how to override"
        );
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    /// Issue #83: the keyring is asked once, not once per caller.
    ///
    /// Asserted through the cache rather than by counting keychain prompts,
    /// which no test can do. `ask` is the only path to the keyring, so a
    /// populated cache is what "asked once" means here — and `probe` being
    /// private and called from exactly one place is the other half.
    #[test]
    fn a_second_ask_does_not_reach_the_keyring() {
        let store = DesktopKeyStore {
            fallback: FileKeyStore::at(PathBuf::from("/tmp/clispeak-cache-test")),
            marker: PathBuf::from("/tmp/clispeak-cache-test-marker"),
            asked: Mutex::new(Some(Keyring::Empty)),
        };
        // Pre-seeded, so anything that reached the keyring would change it.
        assert!(matches!(store.ask(), Keyring::Empty));
        assert!(matches!(store.ask(), Keyring::Empty));
        assert!(matches!(
            store.asked.lock().unwrap().as_ref(),
            Some(Keyring::Empty)
        ));
    }

    /// A locked keyring and a missing key must not describe the same.
    #[test]
    fn a_keyring_that_will_not_answer_says_so() {
        let locked = DesktopKeyStore {
            fallback: FileKeyStore::at(PathBuf::from("/tmp/clispeak-cache-test")),
            marker: PathBuf::from("/tmp/clispeak-cache-test-marker"),
            asked: Mutex::new(Some(Keyring::Unreadable("keychain is locked".into()))),
        };
        let empty = DesktopKeyStore {
            fallback: FileKeyStore::at(PathBuf::from("/tmp/clispeak-cache-test")),
            marker: PathBuf::from("/tmp/clispeak-cache-test-marker"),
            asked: Mutex::new(Some(Keyring::Empty)),
        };

        let locked = locked.describe();
        assert_ne!(
            locked,
            empty.describe(),
            "a locked keyring reported the same as one with no key, which is \
             what stopped anyone learning the keychain was the thing to unlock"
        );
        assert!(locked.contains("keychain is locked"), "{locked}");
        assert!(locked.contains("Unlock"), "{locked}");
    }

    /// Saving updates what `describe` will say, rather than leaving a stale no.
    #[test]
    fn a_stale_empty_does_not_survive_a_save() {
        let store = DesktopKeyStore {
            fallback: FileKeyStore::at(PathBuf::from("/tmp/clispeak-cache-test")),
            marker: PathBuf::from("/tmp/clispeak-cache-test-marker"),
            asked: Mutex::new(Some(Keyring::Empty)),
        };
        *store.asked.lock().unwrap() = Some(Keyring::Held([9u8; 32]));
        assert_eq!(store.describe(), "system keyring");
    }
}
