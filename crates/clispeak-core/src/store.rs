//! Writing state to disk so that a crash cannot lose it and a neighbour
//! cannot read it.
//!
//! Every file this node keeps is one of two things: something private (the
//! live invite token, every message ever spoken, who this device is paired
//! with) or something whose absence breaks the node (the roster). They were
//! written with a plain `std::fs::write`, which gets both wrong.
//!
//! **Permissions.** A plain write creates 0644 under the usual umask, in a
//! directory created 0755. On a machine whose home is readable by others —
//! shared boxes, some older distributions, anything with a service account —
//! any local user could read every message this device had spoken, and during
//! the five minutes an invite is open, read the token and pair themselves.
//! Only `identity.key` was written privately (#56).
//!
//! **Atomicity.** A plain write truncates and then fills. Interrupted in the
//! middle, it leaves a file that is neither the old contents nor the new.
//! `spaces.cbor` failing to parse makes `Node::new` fail, so the node will
//! not start and the only way out is deleting the file, which deletes every
//! pairing this device has. A truncated `policy.json` is worse for being
//! quieter: it is read as "nothing configured", so a muted device
//! un-mutes itself. Android kills apps abruptly, which is where this is not
//! hypothetical.
//!
//! So: write a temporary file beside the target, flush it to the disk, then
//! rename over the target. Rename is atomic on every platform we build for,
//! and `std::fs::rename` replaces an existing file on Windows as well as on
//! Unix. The temporary sits in the same directory because rename is only
//! atomic within one filesystem.

use std::io::Write;
use std::path::Path;

/// Write `bytes` to `path`, readable only by this user, all or nothing.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        create_dir_private(parent)?;
    }

    // Beside the target rather than in a temp directory: rename is atomic
    // only within a filesystem, and `$TMPDIR` is frequently another one.
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    set_owner_only(&mut opts);

    // Scoped so the handle is closed before the rename. Windows refuses to
    // rename a file that is still open, and this code has to work there even
    // though nobody has run it there.
    {
        let mut file = opts.open(&tmp)?;
        file.write_all(bytes)?;
        // Without this the rename can land before the contents do, which on a
        // power cut leaves an intact name over an empty file — precisely the
        // failure the rename was supposed to prevent.
        file.sync_all()?;
    }

    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Leaving a stray `.tmp` behind would be read as a half-written
            // state file by anyone debugging this later.
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Create a directory, and everything above it, readable only by this user.
///
/// The files inside are already owner-only, so this is about the listing: an
/// open invite is a file that exists for five minutes and then does not, so a
/// readable directory tells anyone watching exactly when to try scanning,
/// even though they cannot read the token itself.
///
/// Only the mode of directories this call creates is set. An existing
/// directory is left as it is, because tightening a path somebody else set up
/// is not this function's business.
pub fn create_dir_private(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    set_dir_owner_only(&mut builder);
    builder.create(dir)
}

/// Restrict a file to its owner, where the platform has the concept.
///
/// `cfg(unix)` rather than `cfg(target_os)` because the distinction really is
/// Unix-versus-not: every Unix we build for spells this the same way. Windows
/// inherits the directory's ACL, which for a user's own profile directory is
/// already owner-only.
// portability-exception: a file mode has no portable spelling, and the
// alternative is every state file going through the engine crate
#[cfg(unix)]
fn set_owner_only(opts: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    opts.mode(0o600);
}

// portability-exception: the other arm of the same rule; Windows inherits
// the directory ACL, which for a user's own profile is already owner-only
#[cfg(not(unix))]
fn set_owner_only(_opts: &mut std::fs::OpenOptions) {}

// portability-exception: as above, for the directory rather than the file
#[cfg(unix)]
fn set_dir_owner_only(builder: &mut std::fs::DirBuilder) {
    use std::os::unix::fs::DirBuilderExt;
    builder.mode(0o700);
}

// portability-exception: the other arm of the same rule
#[cfg(not(unix))]
fn set_dir_owner_only(_builder: &mut std::fs::DirBuilder) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("clispeak-store-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir.join("state.json")
    }

    #[test]
    fn what_was_written_is_what_comes_back() {
        let path = scratch("roundtrip");
        write_private(&path, b"hello").expect("write");
        assert_eq!(std::fs::read(&path).expect("read"), b"hello");
    }

    #[test]
    fn a_rewrite_replaces_the_old_contents_and_leaves_no_temporary() {
        let path = scratch("replace");
        write_private(&path, b"first").expect("write");
        write_private(&path, b"second").expect("rewrite");
        assert_eq!(std::fs::read(&path).expect("read"), b"second");
        let tmp = path.with_extension("json.tmp");
        assert!(!tmp.exists(), "the temporary was renamed away, not left");
    }

    #[test]
    fn a_missing_directory_is_created() {
        let path = scratch("nested").parent().unwrap().join("a/b/state.json");
        write_private(&path, b"x").expect("write");
        assert!(path.exists());
    }

    // portability-exception: asserts the mode the rule above sets, so it can
    // only run where modes exist
    #[cfg(unix)]
    #[test]
    fn the_directory_it_makes_is_private_too() {
        // Not for the contents, which are already owner-only, but for the
        // listing: `invite.json` exists only while an invite is open.
        use std::os::unix::fs::PermissionsExt;
        let base = scratch("dirperms").parent().unwrap().join("deep/inner");
        write_private(&base.join("state.json"), b"x").expect("write");
        let mode = std::fs::metadata(&base).expect("stat").permissions().mode();
        assert_eq!(mode & 0o077, 0, "no group or other bits: {mode:o}");
    }

    // portability-exception: asserts the mode the rule above sets, so it can
    // only run where modes exist
    #[cfg(unix)]
    #[test]
    fn nobody_else_can_read_it() {
        // The invite token lives in one of these for five minutes, and every
        // message ever spoken lives in another.
        use std::os::unix::fs::PermissionsExt;
        let path = scratch("perms");
        write_private(&path, b"secret").expect("write");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(mode & 0o077, 0, "no group or other bits: {mode:o}");
    }
}
