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
/// lives and cleanup is not our problem.
pub fn socket_name() -> String {
    "voicecast.sock".to_string()
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
