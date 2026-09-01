//! Identity persistence and key file safety.

use std::path::PathBuf;
use std::sync::Mutex;

use voicecast_core::{FileKeyStore, Identity, IdentityError, KeyStore};

/// A store that keeps the key in memory, for testing the logic without touching
/// a real keyring or the filesystem.
#[derive(Default)]
struct MemoryStore {
    key: Mutex<Option<[u8; 32]>>,
    saves: Mutex<usize>,
}

impl KeyStore for MemoryStore {
    fn load(&self) -> Result<Option<[u8; 32]>, IdentityError> {
        Ok(*self.key.lock().unwrap())
    }
    fn save(&self, key: &[u8; 32]) -> Result<(), IdentityError> {
        *self.key.lock().unwrap() = Some(*key);
        *self.saves.lock().unwrap() += 1;
        Ok(())
    }
    fn describe(&self) -> String {
        "memory".into()
    }
}

/// A unique scratch path per test, cleaned up on drop.
struct TempPath(PathBuf);

impl TempPath {
    fn new(tag: &str) -> Self {
        let mut p = std::env::temp_dir();
        p.push(format!("voicecast-test-{tag}-{}", std::process::id()));
        Self(p)
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn identity_is_created_once_then_reused() {
    let store = MemoryStore::default();

    let first = Identity::load_or_create(&store).expect("create");
    let second = Identity::load_or_create(&store).expect("reload");

    assert_eq!(first.id(), second.id(), "identity must survive a restart");
    assert_eq!(
        *store.saves.lock().unwrap(),
        1,
        "must not rewrite an existing key"
    );
}

#[test]
fn file_store_round_trips() {
    let path = TempPath::new("roundtrip");
    let store = FileKeyStore::at(path.0.clone());

    assert!(
        store.load().unwrap().is_none(),
        "absent file is not an error"
    );

    let created = Identity::load_or_create(&store).expect("create");
    let reloaded = Identity::load_or_create(&store).expect("reload");
    assert_eq!(created.id(), reloaded.id());
}

#[cfg(unix)]
#[test]
fn key_file_is_not_readable_by_others() {
    use std::os::unix::fs::PermissionsExt;

    let path = TempPath::new("perms");
    let store = FileKeyStore::at(path.0.clone());
    Identity::load_or_create(&store).expect("create");

    let mode = std::fs::metadata(&path.0).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "key file must be owner-only, found {mode:o}");
}

#[test]
fn a_truncated_key_file_is_reported_not_ignored() {
    // Silently regenerating would change the device's identity and quietly
    // break every space it belongs to. Better to fail loudly.
    let path = TempPath::new("malformed");
    std::fs::write(&path.0, b"too short").unwrap();

    let store = FileKeyStore::at(path.0.clone());
    let err = Identity::load_or_create(&store).expect_err("should reject");
    assert!(matches!(err, IdentityError::Malformed(_)), "got {err:?}");
}

#[test]
fn location_reports_where_the_key_actually_is() {
    let path = TempPath::new("location");
    let store = FileKeyStore::at(path.0.clone());
    let identity = Identity::load_or_create(&store).expect("create");
    assert!(
        identity.location().contains("file"),
        "got {}",
        identity.location()
    );
}
