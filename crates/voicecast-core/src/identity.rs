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

use std::path::PathBuf;

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

    /// Where the key lives, for `voicecast status`.
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
        let dirs = directories::ProjectDirs::from("", "", "voicecast")
            .ok_or_else(|| IdentityError::Store("no config directory".into()))?;
        Ok(Self::at(dirs.config_dir().join("identity.key")))
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
            std::fs::create_dir_all(parent).map_err(|e| IdentityError::Store(e.to_string()))?;
        }
        // Create with 0600 from the outset rather than writing then chmod'ing:
        // the gap between the two would leave the key briefly world-readable.
        write_private(&self.path, key).map_err(|e| IdentityError::Store(e.to_string()))
    }

    fn describe(&self) -> String {
        format!("file {}", self.path.display())
    }
}

/// Write bytes to a file only the owner can read.
///
/// Permissions are set at creation on Unix. Elsewhere the containing
/// directory is the protection, which is why the key lives in a
/// per-application config directory rather than anywhere shared.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    set_owner_only(&mut opts);

    let mut file = opts.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(unix)]
fn set_owner_only(opts: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    opts.mode(0o600);
}

#[cfg(not(unix))]
fn set_owner_only(_opts: &mut std::fs::OpenOptions) {}
