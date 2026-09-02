//! Peer-to-peer transport.
//!
//! One bidirectional QUIC stream per message, plus a stream for joining. That
//! choice inherits three things from the transport rather than building them:
//! cancellation is a stream reset, a long document cannot head-of-line block
//! an urgent notification, and backpressure is QUIC's flow control. See
//! `docs/protocol.md`.
//!
//! Peers are addressed by public key alone. M0 measured this working across
//! carrier-grade NAT — relay first, hole-punched to a direct path within a
//! second — which is what makes "pair once, ever" possible.

use anyhow::{Context, Result, bail};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, SecretKey,
    endpoint::{Connection, presets},
};
use voicecast_proto::PeerMessage;

/// Application-layer protocol identifier. Peers must agree on it to connect.
pub const ALPN: &[u8] = b"voicecast/1";

/// Refuse absurd frames rather than trusting a length and allocating for it.
const MAX_FRAME: u32 = 8 * 1024 * 1024;

/// This device's connection to the network.
pub struct Transport {
    endpoint: Endpoint,
}

impl Transport {
    /// Bind an endpoint using this device's identity.
    ///
    /// The `N0` preset supplies pkarr publishing and resolution plus relay
    /// fallback — the three-rung discovery ladder from `docs/architecture.md`,
    /// none of which we have to run.
    pub async fn bind(secret: SecretKey) -> Result<Self> {
        let endpoint = Endpoint::builder(presets::N0)
            .secret_key(secret)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .context("binding iroh endpoint")?;
        Ok(Self { endpoint })
    }

    /// This device's public key.
    pub fn id(&self) -> EndpointId {
        self.endpoint.id()
    }

    /// Wait until this device is reachable by others.
    pub async fn online(&self) {
        self.endpoint.online().await;
    }

    /// Accept the next incoming connection.
    pub async fn accept(&self) -> Option<Result<Connection>> {
        let incoming = self.endpoint.accept().await?;
        Some(incoming.await.context("completing handshake"))
    }

    /// Dial a peer by public key alone.
    ///
    /// No address and no relay hint: resolution goes through pkarr, which is
    /// what lets a device move between networks without re-pairing.
    pub async fn connect(&self, peer: EndpointId) -> Result<Connection> {
        let addr: EndpointAddr = peer.into();
        self.endpoint
            .connect(addr, ALPN)
            .await
            .context("connecting to peer")
    }

    /// Close the endpoint.
    pub async fn close(&self) {
        self.endpoint.close().await;
    }
}

/// Write one length-prefixed CBOR frame.
pub async fn write_msg<W>(w: &mut W, msg: &PeerMessage) -> Result<()>
where
    W: tokio::io::AsyncWriteExt + Unpin,
{
    let mut buf = Vec::new();
    ciborium::into_writer(msg, &mut buf).context("encoding peer message")?;
    let len = u32::try_from(buf.len()).context("message too large")?;
    w.write_all(&len.to_be_bytes()).await?;
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

/// Read one length-prefixed CBOR frame.
pub async fn read_msg<R>(r: &mut R) -> Result<PeerMessage>
where
    R: tokio::io::AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)
        .await
        .context("reading message length")?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME {
        bail!("message of {len} bytes exceeds the {MAX_FRAME} byte limit");
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)
        .await
        .context("reading message body")?;
    ciborium::from_reader(&buf[..]).context("decoding peer message")
}
