//! Local IPC between the CLI and the node.
//!
//! Uses `interprocess` local sockets — Unix domain sockets on Unix, named
//! pipes on Windows — behind one API, so this crate stays free of platform
//! conditionals. `cargo xtask portability` enforces that.
//!
//! Frames are length-prefixed CBOR: a `u32` byte count, then the payload.
//! Sharing the encoding with the wire protocol means one serialisation path
//! to reason about, and CBOR's self-description means a stale CLI talking to
//! a newer node degrades rather than misparses.

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Refuse absurd frames rather than trusting a length and allocating for it.
const MAX_FRAME: u32 = 8 * 1024 * 1024;

/// Name of the local socket the node listens on.
///
/// Namespaced rather than a filesystem path, so the OS decides where it
/// lives and cleanup is not our problem. **It is a name, not a path**, and
/// where it ends up differs per platform: Linux puts it in the abstract
/// namespace, where no file exists at all; macOS prefixes `$TMPDIR` or
/// `/tmp/`; Android `/data/local/tmp/`; Windows makes it a named pipe.
///
/// So a path-shaped value still works — it is just used as a name, and
/// `/tmp/vc.sock` becomes `/tmp//tmp/vc.sock` on a Mac. That it works is why
/// it went unnoticed: the CLI and the node apply the same mapping, so they
/// agree with each other and disagree only with whoever goes looking. See
/// [`path_shaped`] and issue #43.
///
/// `VOICECAST_SOCKET` overrides it, which is how a second node runs
/// alongside the first — needed to exercise the join flow on one machine.
pub fn socket_name() -> String {
    std::env::var("VOICECAST_SOCKET").unwrap_or_else(|_| "voicecast.sock".to_string())
}

/// Whether a socket name has been given as though it were a path.
///
/// Not an error: it binds, and both ends map it the same way. But every tool
/// a person reaches for next — `ls`, `lsof`, a systemd unit, a health check —
/// takes it for a path, and on Linux there is no file to find under any name.
/// Worth one line at startup rather than a silent mismatch.
///
/// Deliberately portable: no `cfg` here decides what a path looks like,
/// because `voicecast-core` may not hold one and because a Windows name and a
/// Unix name are both wrong in the same way for this purpose.
pub fn path_shaped(name: &str) -> bool {
    name.contains('/') || name.contains('\\')
}

/// Write one length-prefixed CBOR frame.
pub async fn write_frame<W, T>(w: &mut W, value: &T) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: serde::Serialize,
{
    let mut buf = Vec::new();
    ciborium::into_writer(value, &mut buf).context("encoding frame")?;
    let len = u32::try_from(buf.len()).context("frame too large")?;
    w.write_all(&len.to_be_bytes()).await?;
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

/// Read one length-prefixed CBOR frame.
pub async fn read_frame<R, T>(r: &mut R) -> Result<T>
where
    R: AsyncReadExt + Unpin,
    T: serde::de::DeserializeOwned,
{
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)
        .await
        .context("reading frame length")?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME {
        bail!("frame of {len} bytes exceeds the {MAX_FRAME} byte limit");
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await.context("reading frame body")?;
    ciborium::from_reader(&buf[..]).context("decoding frame")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_name_is_not_path_shaped() {
        assert!(!path_shaped("voicecast.sock"));
        assert!(!path_shaped("voicecast-test-2.sock"));
    }

    /// The value that started #43. It binds, and it is not a path.
    #[test]
    fn a_unix_path_is_path_shaped() {
        assert!(path_shaped("/tmp/vc-d.sock"));
        assert!(path_shaped("run/voicecast.sock"));
    }

    /// Windows names are wrong in the same way, so the check is not
    /// per-platform. `voicecast-core` may not hold a `cfg` in any case.
    #[test]
    fn a_windows_path_is_path_shaped() {
        assert!(path_shaped(r"C:\\temp\\voicecast.sock"));
    }

    /// Whatever the default is, it must not be the thing we warn about.
    ///
    /// Asserted on the literal rather than through `socket_name`, which reads
    /// the environment: a test that mutates `VOICECAST_SOCKET` would race
    /// every other test in the binary.
    #[test]
    fn the_default_name_passes_its_own_check() {
        assert!(!path_shaped("voicecast.sock"));
    }
}
