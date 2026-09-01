//! Length-prefixed CBOR framing.
//!
//! Deliberately duplicated from `voicecast_core::ipc`: depending on the node
//! crate would pull its whole dependency graph into a binary whose entire
//! purpose is starting instantly.

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Refuse absurd frames rather than trusting a length and allocating for it.
const MAX_FRAME: u32 = 8 * 1024 * 1024;

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
