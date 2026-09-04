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
use clispeak_proto::PeerMessage;
use iroh::{
    Endpoint, EndpointAddr, EndpointId, SecretKey,
    dns::DnsResolver,
    endpoint::{Connection, presets},
};

/// Application-layer protocol identifier. Peers must agree on it to connect.
pub const ALPN: &[u8] = b"clispeak/1";

/// Refuse absurd frames rather than trusting a length and allocating for it.
const MAX_FRAME: u32 = 8 * 1024 * 1024;

/// How long to spend reaching a device before calling it unreachable.
///
/// Ours rather than iroh's, and written down, because the CLI mirrors it.
/// `patience()` in `clispeak-cli` explains the rule: the node applies the
/// bound and is the one that times out, because it knows *why* and can
/// answer `unreachable` with a reason. That only works if there is a bound
/// here to mirror — and there was not. The dial fell through to iroh's own
/// timeout at about thirty seconds while the CLI gave up at ten, so a speak
/// to an unreachable device reported that the *local* node had never
/// answered and told the reader to restart a healthy app (#151).
///
/// **Ninety seconds, measured rather than reasoned.** This was twenty, chosen
/// because M0 timed a relay-first connection across carrier-grade NAT at
/// about a second and twenty looked like an order of magnitude of headroom.
/// It was headroom over the wrong case.
///
/// Measured on 4 September 2026 against a real Android phone: **2.1 and 2.3
/// seconds warm, and 58 seconds cold** — the first message after the phone
/// had been idle for hours. A dozing phone is the *ordinary* case for this
/// project, not an edge one; the whole premise is reaching someone who is
/// not at their machine. At twenty seconds that first message would have
/// been reported unreachable to a phone that was about to answer.
///
/// Ninety is well past the one cold sample and still a bound. What it costs
/// is that a device genuinely switched off takes that long to be called
/// unreachable, which is slow and true — against ten seconds and false,
/// which is what this replaced (#151, decision 89).
///
/// **Changing it means changing the mirror.** `clispeak-cli` cannot import
/// this — it depends on `clispeak-proto` and `clispeak-text` only, which is
/// what keeps its startup at 3ms — so the number is duplicated there by hand
/// beside a comment naming this constant.
pub const PEER_CONNECT: std::time::Duration = std::time::Duration::from_secs(90);

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
    ///
    /// `dns` overrides how hostnames are resolved. Passing `None` reads the
    /// system's configuration, which is right everywhere except Android:
    /// there it requires a JNI context this crate has no way to obtain, and
    /// the lookup panics without one. The app supplies a fixed resolver
    /// instead — platform knowledge belongs in the shell, not here.
    pub async fn bind(secret: SecretKey, dns: Option<DnsResolver>) -> Result<Self> {
        let mut builder = Endpoint::builder(presets::N0)
            .secret_key(secret)
            .alpns(vec![ALPN.to_vec()]);
        if let Some(dns) = dns {
            builder = builder.dns_resolver(dns);
        }
        let endpoint = builder.bind().await.context("binding iroh endpoint")?;
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
        match tokio::time::timeout(PEER_CONNECT, self.endpoint.connect(addr, ALPN)).await {
            Ok(result) => result.context("connecting to peer"),
            // Says the number, because the next question anyone asks is
            // whether waiting longer would have worked.
            Err(_) => bail!(
                "connecting to peer: no answer in {}s",
                PEER_CONNECT.as_secs()
            ),
        }
    }

    /// Close the endpoint.
    pub async fn close(&self) {
        self.endpoint.close().await;
    }
}

/// What serving a peer actually needs from a connection.
///
/// `handle_peer` took an `iroh::endpoint::Connection`, which is a concrete
/// type that can only be obtained by binding an endpoint and having a real
/// device dial it. So the protocol — sixteen message arms, every join,
/// revocation and speak decision on the receiving side — could not be driven
/// by a test at all, and every fix to it says "verified by reading" rather
/// than "verified by test" (#80).
///
/// Two methods is the whole surface. The frame helpers below are already
/// generic over `AsyncRead` and `AsyncWrite`, so the streams needed nothing;
/// only the connection was concrete. A test supplies a pair of
/// `tokio::io::duplex` halves and drives the same code a peer reaches.
///
/// Deliberately not a wider abstraction. This is not "a transport" — it is
/// the two things one function asks for, named after what it asks for. A
/// trait that anticipated more would be a design nobody had tested either.
pub trait PeerConnection: Send + Sync {
    /// The writable half of an accepted stream.
    type Send: tokio::io::AsyncWrite + Unpin + Send;
    /// The readable half.
    type Recv: tokio::io::AsyncRead + Unpin + Send;

    /// Who is on the other end.
    ///
    /// Still an `EndpointId` rather than the string the roster stores: the
    /// policy checks take the key itself, and widening them to strings to
    /// suit a trait would trade real type safety for a convenience a test
    /// does not need — generating a key is one line.
    fn remote(&self) -> EndpointId;

    /// The next bidirectional stream, or `None` once the peer is gone.
    fn accept_bi(
        &self,
    ) -> impl std::future::Future<Output = Option<(Self::Send, Self::Recv)>> + Send;
}

impl PeerConnection for Connection {
    type Send = iroh::endpoint::SendStream;
    type Recv = iroh::endpoint::RecvStream;

    fn remote(&self) -> EndpointId {
        self.remote_id()
    }

    async fn accept_bi(&self) -> Option<(Self::Send, Self::Recv)> {
        Connection::accept_bi(self).await.ok()
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
