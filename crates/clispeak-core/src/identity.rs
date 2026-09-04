//! Device identity.
//!
//! A device *is* its ed25519 public key. The display name is a local label
//! that can change freely without breaking anything — which is what stops a
//! name being usable to impersonate a device, and why renaming never costs a
//! re-pairing. See `docs/architecture.md`.
//!
//! Where the private key is *stored* is a platform question, so it sits
//! behind [`KeyStore`] rather than being decided here. This crate ships the
//! portable file-backed implementation; desktop binaries supply a keyring.

use std::path::{Path, PathBuf};

use iroh_base::{EndpointId, SecretKey};

/// Why an identity could not be loaded or saved.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// The store could not be reached.
    #[error("key store unavailable: {0}")]
    Store(String),
    /// Stored bytes were not a valid key.
    #[error("stored key is malformed: {0}")]
    Malformed(String),
}

/// Somewhere a 32-byte private key can be kept.
///
/// Implementations must not log or display the key. Callers hand over raw
/// bytes because the store should have no opinion about key types.
pub trait KeyStore: Send + Sync {
    /// Return the stored key, or `None` if this device has no identity yet.
    fn load(&self) -> Result<Option<[u8; 32]>, IdentityError>;

    /// Persist the key, replacing anything already there.
    fn save(&self, key: &[u8; 32]) -> Result<(), IdentityError>;

    /// Where the key lives, for `clispeak status`.
    ///
    /// Worth surfacing: a user who believes their key is in the system
    /// keyring should be able to discover it silently fell back to a file.
    fn describe(&self) -> String;
}

/// This device's cryptographic identity.
pub struct Identity {
    secret: SecretKey,
    location: String,
}

impl Identity {
    /// Load the existing identity, creating one on first run.
    pub fn load_or_create(store: &dyn KeyStore) -> Result<Self, IdentityError> {
        let secret = match store.load()? {
            Some(bytes) => SecretKey::from_bytes(&bytes),
            None => {
                let secret = SecretKey::generate();
                store.save(&secret.to_bytes())?;
                secret
            }
        };
        // Asked after the key is in place: on a first run the keyring is
        // empty, so describing it beforehand reported the file fallback even
        // when the key then went to the keyring.
        let location = store.describe();
        Ok(Self { secret, location })
    }

    /// This device's public key — its address on the network.
    pub fn id(&self) -> EndpointId {
        self.secret.public()
    }

    /// The private key, for handing to the transport.
    pub fn secret(&self) -> &SecretKey {
        &self.secret
    }

    /// Human-readable description of where the key is stored.
    pub fn location(&self) -> &str {
        &self.location
    }
}

/// Shows the public id and where the key lives — never the key.
///
/// Written by hand rather than derived precisely so that a stray `{:?}` in a
/// log line can never print the private key.
impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("id", &self.id().to_string())
            .field("location", &self.location)
            .field("secret", &"<redacted>")
            .finish()
    }
}

/// A key on disk, readable only by its owner.
///
/// The documented fallback for platforms without a usable keyring, and the
/// only store that works identically on all five targets.
pub struct FileKeyStore {
    path: PathBuf,
}

impl FileKeyStore {
    /// Store the key at `path`.
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// Store the key in the platform's config directory.
    pub fn default_location() -> Result<Self, IdentityError> {
        Ok(Self::at(config_dir()?.join("identity.key")))
    }
}

impl KeyStore for FileKeyStore {
    fn load(&self) -> Result<Option<[u8; 32]>, IdentityError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(IdentityError::Store(e.to_string())),
        };
        let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
            IdentityError::Malformed(format!("expected 32 bytes, found {}", bytes.len()))
        })?;
        Ok(Some(arr))
    }

    fn save(&self, key: &[u8; 32]) -> Result<(), IdentityError> {
        if let Some(parent) = self.path.parent() {
            crate::store::create_dir_private(parent)
                .map_err(|e| IdentityError::Store(e.to_string()))?;
        }
        // Create with 0600 from the outset rather than writing then chmod'ing:
        // the gap between the two would leave the key briefly world-readable.
        crate::store::write_private(&self.path, key)
            .map_err(|e| IdentityError::Store(e.to_string()))
    }

    fn describe(&self) -> String {
        format!("file {}", self.path.display())
    }
}

/// Where this device keeps its state, when the host has told us.
static CONFIG_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Tell the library where to keep this device's state.
///
/// Android has no XDG-style config directory — `ProjectDirs` returns nothing
/// there — so the app passes its own app-private path in. Ignored if called
/// twice; the first caller wins.
pub fn set_config_dir(dir: PathBuf) {
    let _ = CONFIG_DIR.set(dir);
}

/// Where this device keeps its state.
///
/// Resolution order: a path the host set, then `CLISPEAK_CONFIG_DIR` (which
/// lets a second node run alongside the first for testing), then the
/// platform's own config directory.
pub fn config_dir() -> Result<PathBuf, IdentityError> {
    if let Some(dir) = CONFIG_DIR.get() {
        return Ok(dir.clone());
    }
    if let Ok(dir) = std::env::var("CLISPEAK_CONFIG_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let dirs = directories::ProjectDirs::from("", "", "clispeak").ok_or_else(|| {
        IdentityError::Store(
            "no config directory on this platform; the host must call set_config_dir".into(),
        )
    })?;
    Ok(dirs.config_dir().to_path_buf())
}

/// Everything this device keeps beside its identity.
///
/// An allowlist rather than "move the directory", because the app's data
/// directory also holds a web view's caches and databases, which are its own
/// business and would be nonsense in a config directory.
const STATE_FILES: &[&str] = &[
    "identity.key",
    "identity.in-keyring",
    "name",
    "voice",
    "roster.cbor",
    "spaces.cbor",
    "policy.json",
    "history.json",
    "invite.json",
    "skill-destination",
];

/// Move state written under an older location into the current one.
///
/// The app used to tell the core to keep its files in the app's own data
/// directory, which mobile genuinely needs and desktop does not — and on
/// desktop `clispeakd` was already using the ordinary config directory. One
/// device ended up with two rosters, two histories and two mute settings
/// hanging off a single identity, because the identity lives in the keyring
/// and was the only thing they shared.
///
/// Returns the files it moved, so the caller can say so rather than doing it
/// silently. Idempotent: a moved file is gone from the old place, so a second
/// run finds nothing.
///
/// A file already present at the destination is renamed aside rather than
/// overwritten or skipped. Skipping would let a stale copy — such as one a
/// daemon wrote before the app existed — win over the state someone has
/// actually been using, and overwriting would destroy the evidence either
/// way. Being wrong here costs someone every device pairing they have.
pub fn migrate_from(old: &Path) -> Result<Vec<String>, IdentityError> {
    migrate_between(old, &config_dir()?)
}

/// What this project used to be called, and where that left its files.
///
/// The rename from `voicecast` moved the config directory out from under
/// every existing install: `ProjectDirs` derives the path from the project
/// name, so a device that had a roster, a history and an identity yesterday
/// would have looked like a first run today. On a phone that is unavoidable —
/// an application identifier is the app's name to the operating system, and a
/// new one is a new app with a sandbox the old one cannot reach. On a desktop
/// it is only a directory, and directories can be moved (decision 82).
const PREVIOUS_PROJECT_NAME: &str = "voicecast";

/// Move state left by the project's previous name into the current directory.
///
/// Called explicitly at startup rather than from `config_dir`, which is a
/// getter that many things call and no place for a side effect that moves the
/// file holding every device pairing.
///
/// Returns the files it moved so the caller can say so out loud. Idempotent,
/// and a no-op on a machine that never ran the old name.
pub fn migrate_from_previous_name() -> Result<Vec<String>, IdentityError> {
    let Some(dirs) = directories::ProjectDirs::from("", "", PREVIOUS_PROJECT_NAME) else {
        return Ok(Vec::new());
    };
    // Only where the platform decides the location. A host that set the
    // directory itself, or a `CLISPEAK_CONFIG_DIR` pointing somewhere
    // deliberate, is not a machine that has anything to migrate *from*.
    if CONFIG_DIR.get().is_some() || std::env::var_os("CLISPEAK_CONFIG_DIR").is_some() {
        return Ok(Vec::new());
    }
    migrate_from(dirs.config_dir())
}

/// The move itself, with both ends named.
///
/// Split out because `config_dir` is settled once per process and cannot be
/// changed between tests — so a test that had to set it could only ever
/// exercise this once, and the second case would silently run against the
/// first one's directory.
fn migrate_between(old: &Path, current: &Path) -> Result<Vec<String>, IdentityError> {
    if old == current || !old.exists() {
        return Ok(Vec::new());
    }
    crate::store::create_dir_private(current).map_err(|e| IdentityError::Store(e.to_string()))?;

    let mut moved = Vec::new();
    for name in STATE_FILES {
        let from = old.join(name);
        if !from.exists() {
            continue;
        }
        let to = current.join(name);
        if to.exists() {
            let aside = current.join(format!("{name}.superseded"));
            let _ = std::fs::rename(&to, &aside);
        }
        // Rename first: it is atomic within a filesystem, which these two
        // normally share. Copy only if they do not.
        if std::fs::rename(&from, &to).is_err() {
            std::fs::copy(&from, &to).map_err(|e| IdentityError::Store(e.to_string()))?;
            std::fs::remove_file(&from).map_err(|e| IdentityError::Store(e.to_string()))?;
        }
        moved.push((*name).to_string());
    }
    Ok(moved)
}

/// This device's local label, falling back to `fallback` when nothing names
/// it.
///
/// A convenience for humans, never an identity — it is deliberately excluded
/// from a join record's signature so renaming costs nothing. Resolution
/// order: an explicitly chosen name, then `CLISPEAK_NAME`, then the hostname,
/// then whatever the caller offers.
///
/// **The fallback is the caller's because it is platform-shaped.** A phone
/// answers `gethostname` with "localhost", and the plausible thing to call it
/// instead is "Android phone" or "iPhone" — which is a `target_os` decision,
/// and this crate does not get to make one. It made it anyway for months,
/// through `cfg!`, which the portability gate could not see (#161).
pub fn device_name_or(fallback: &str) -> String {
    if let Ok(dir) = config_dir()
        && let Ok(name) = std::fs::read_to_string(dir.join("name"))
    {
        let name = name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    if let Ok(name) = std::env::var("CLISPEAK_NAME")
        && !name.trim().is_empty()
    {
        return name.trim().to_string();
    }
    hostname().unwrap_or_else(|| fallback.to_string())
}

/// [`device_name_or`] with the one fallback this crate can honestly offer.
///
/// "this device" reads like a bug in a device list, which is exactly why the
/// platform-specific answers exist — and exactly why they belong to whoever
/// knows the platform. A host that has something better to say passes it.
pub fn device_name() -> String {
    device_name_or("this device")
}

/// Choose this device's label, remembering it across restarts.
pub fn set_device_name(name: &str) -> Result<(), IdentityError> {
    let dir = config_dir()?;
    crate::store::create_dir_private(&dir).map_err(|e| IdentityError::Store(e.to_string()))?;
    crate::store::write_private(&dir.join("name"), name.trim().as_bytes())
        .map_err(|e| IdentityError::Store(e.to_string()))
}

/// The voice and rate this device was last set to.
///
/// Stored beside the identity rather than in the engine: engines are
/// recreated on every start, and a preference that does not survive a restart
/// is not a preference.
pub fn load_voice_settings() -> Option<(String, f32)> {
    let text = std::fs::read_to_string(config_dir().ok()?.join("voice")).ok()?;
    let (id, rate) = text.trim().split_once('\n')?;
    Some((id.to_string(), rate.parse().ok()?))
}

/// Remember the voice and rate for next time.
pub fn save_voice_settings(id: &str, rate: f32) -> Result<(), IdentityError> {
    let dir = config_dir()?;
    crate::store::create_dir_private(&dir).map_err(|e| IdentityError::Store(e.to_string()))?;
    crate::store::write_private(&dir.join("voice"), format!("{id}\n{rate}").as_bytes())
        .map_err(|e| IdentityError::Store(e.to_string()))
}

/// This machine's hostname, if it has a usable one.
///
/// Asked of the system rather than read from `/etc/hostname`, which is a
/// Linux file: macOS and Windows have a perfectly good name and neither has
/// that file, so both fell through to [`default_name`] and every device in
/// the roster was called "this device" — including the remote ones, which
/// made `--to <name>` pick one of them silently. See issue #38.
fn hostname() -> Option<String> {
    usable_hostname(&gethostname::gethostname().to_string_lossy())
}

/// The part of a raw hostname worth showing a person, if any.
///
/// Split out from [`hostname`] because the system call is not testable and
/// the trimming is where the decisions are.
fn usable_hostname(raw: &str) -> Option<String> {
    // The mDNS suffix every Mac carries. It says nothing that distinguishes
    // one of your devices from another, which is the only job this name has.
    let name = raw.trim().strip_suffix(".local").unwrap_or(raw.trim());
    // A phone or a container answers with this. It is not a name, it is the
    // absence of one, and it is the same on every device that gives it —
    // which is worse than a placeholder, because it looks deliberate.
    if name.is_empty() || name.eq_ignore_ascii_case("localhost") {
        return None;
    }
    Some(name.to_string())
}

/// Write bytes to a file only the owner can read.
///
/// Permissions are set at creation on Unix. Elsewhere the containing
/// directory is the protection, which is why the key lives in a
/// per-application config directory rather than anywhere shared.
#[cfg(test)]
mod migration_tests {
    use super::*;

    /// A scratch directory unique to this test.
    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("clispeak-migrate-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn state_moves_across_and_leaves_the_web_view_behind() {
        let root = scratch("move");
        let old = root.join("old");
        let new = root.join("new");
        std::fs::create_dir_all(&old).expect("old");

        std::fs::write(old.join("spaces.cbor"), b"roster").expect("write");
        std::fs::write(old.join("history.json"), b"[]").expect("write");
        // The app's data directory also holds a web view's caches. They are
        // not ours and must not follow.
        std::fs::write(old.join("hsts-storage.sqlite"), b"x").expect("write");

        let moved = migrate_between(&old, &new).expect("migrate");
        assert!(moved.contains(&"spaces.cbor".to_string()));
        assert!(moved.contains(&"history.json".to_string()));
        assert!(!moved.contains(&"hsts-storage.sqlite".to_string()));

        assert_eq!(std::fs::read(new.join("spaces.cbor")).unwrap(), b"roster");
        assert!(
            !old.join("spaces.cbor").exists(),
            "the old copy must be gone"
        );
        assert!(old.join("hsts-storage.sqlite").exists(), "not ours to move");

        // Running again finds nothing, because the first run emptied it.
        assert!(migrate_between(&old, &new).expect("second").is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_already_there_is_set_aside_rather_than_left_to_win() {
        // The case on the machine that found this: a stale roster written by
        // a daemon before the app existed, sitting where the live one is
        // about to land. Skipping would let the stale copy win and cost every
        // device pairing; overwriting would destroy the evidence.
        let root = scratch("clash");
        let old = root.join("old");
        let new = root.join("new");
        std::fs::create_dir_all(&old).expect("old");
        std::fs::create_dir_all(&new).expect("new");

        std::fs::write(old.join("spaces.cbor"), b"live").expect("write");
        std::fs::write(new.join("spaces.cbor"), b"stale").expect("write");

        migrate_between(&old, &new).expect("migrate");

        assert_eq!(std::fs::read(new.join("spaces.cbor")).unwrap(), b"live");
        assert_eq!(
            std::fs::read(new.join("spaces.cbor.superseded")).unwrap(),
            b"stale",
            "the displaced copy has to survive somewhere"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod hostname_tests {
    use super::*;

    #[test]
    fn a_mac_hostname_loses_its_mdns_suffix() {
        assert_eq!(
            usable_hostname("Patricks-Mac-mini.local").as_deref(),
            Some("Patricks-Mac-mini")
        );
    }

    #[test]
    fn a_plain_hostname_survives_untouched() {
        assert_eq!(usable_hostname("laptop").as_deref(), Some("laptop"));
        assert_eq!(
            usable_hostname("  build-box\n").as_deref(),
            Some("build-box")
        );
    }

    #[test]
    fn a_domain_that_is_not_mdns_is_left_alone() {
        // Only the mDNS suffix is noise we can be sure about. Trimming every
        // dotted part would turn two machines in different domains into the
        // same name, which is the failure this whole change is about.
        assert_eq!(
            usable_hostname("host.corp.example.com").as_deref(),
            Some("host.corp.example.com")
        );
    }

    #[test]
    fn a_name_that_is_not_one_is_refused() {
        // So the caller falls back to something per-platform and plausible
        // rather than naming every phone the same thing.
        assert_eq!(usable_hostname(""), None);
        assert_eq!(usable_hostname("   "), None);
        assert_eq!(usable_hostname("localhost"), None);
        assert_eq!(usable_hostname("LocalHost"), None);
        assert_eq!(usable_hostname(".local"), None);
    }
}
