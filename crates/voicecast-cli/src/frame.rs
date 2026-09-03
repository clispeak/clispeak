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

/// Bytes in the shared secret, and in every value the handshake exchanges.
const TOKEN_LEN: usize = 32;

/// The secret the node left in a directory only this user can enter.
pub type Token = [u8; TOKEN_LEN];

/// How long to wait for the node to identify itself.
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

const NODE_LABEL: &[u8] = b"voicecast-ipc-node";
const CLIENT_LABEL: &[u8] = b"voicecast-ipc-client";

/// Read the secret a running node left beside its identity.
///
/// Deliberately duplicated from `voicecast_core::ipc`, like the framing above
/// and the socket name — this binary depends on `proto` and `text` only, and
/// the two copies are kept in step by hand.
pub fn read_token(config_dir: &std::path::Path) -> std::io::Result<Token> {
    let bytes = std::fs::read(config_dir.join("ipc-token"))?;
    Token::try_from(bytes.as_slice()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the local socket token is not the right size",
        )
    })
}

fn proof(token: &Token, label: &[u8], nonce: &[u8; TOKEN_LEN]) -> blake3::Hash {
    let mut input = Vec::with_capacity(label.len() + TOKEN_LEN);
    input.extend_from_slice(label);
    input.extend_from_slice(nonce);
    blake3::keyed_hash(token, &input)
}

/// Check the node is ours, then prove we may drive it.
///
/// **The node answers first.** The local socket is a name any other user on
/// this machine can take — Linux's abstract namespace has no permissions, and
/// on macOS `interprocess` puts the socket in `/tmp`, which its own source
/// calls "the world-writable temporary directory". Speaking first would hand
/// text, invites and history to whoever got there first (#54).
pub async fn offer_handshake<S>(s: &mut S, token: &Token) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let mut client_nonce = [0u8; TOKEN_LEN];
        rand::fill(&mut client_nonce);
        s.write_all(&client_nonce).await?;
        s.flush().await?;

        let mut claimed = [0u8; TOKEN_LEN];
        s.read_exact(&mut claimed).await?;
        // `blake3::Hash` compares in constant time.
        if blake3::Hash::from(claimed) != proof(token, NODE_LABEL, &client_nonce) {
            // Not "that is an impostor": something else holding the socket and
            // a token belonging to a different node look identical from here.
            bail!(
                "it did not answer with this machine's voicecast token — either \
                 something else holds the socket, or the token here belongs to a \
                 different node"
            );
        }

        let mut node_nonce = [0u8; TOKEN_LEN];
        s.read_exact(&mut node_nonce).await?;
        s.write_all(proof(token, CLIENT_LABEL, &node_nonce).as_bytes())
            .await?;
        s.flush().await?;
        Ok(())
    })
    .await
    .context("timed out identifying the node")?
}
