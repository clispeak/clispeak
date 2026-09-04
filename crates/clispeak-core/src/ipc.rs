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
use interprocess::local_socket::{GenericNamespaced, ToNsName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Bytes in the shared secret, and in every value the handshake exchanges.
const TOKEN_LEN: usize = 32;

/// The secret a node and its CLI both read from a directory only the owner
/// can enter.
pub type Token = [u8; TOKEN_LEN];

/// How long either side will wait for the other to prove itself.
///
/// Short: both processes are on this machine and a handshake is four small
/// writes. The point is that a connection which never speaks cannot hold a
/// task open, which on the node's side is a way to exhaust it without ever
/// authenticating.
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Where the node leaves the secret.
///
/// Beside the identity, in the config directory `store::create_dir_private`
/// already keeps at 0700 — which is the whole mechanism. The file's contents
/// are not clever; its *location* is what another user cannot read.
pub fn token_path(config_dir: &std::path::Path) -> std::path::PathBuf {
    config_dir.join("ipc-token")
}

/// Make a fresh secret and write it where only this user can read it.
///
/// New on every node start rather than kept: a node that has exited holds
/// nothing, and a token left behind from a previous boot is a credential
/// nobody is tracking.
pub fn install_token(config_dir: &std::path::Path) -> std::io::Result<Token> {
    let mut token = [0u8; TOKEN_LEN];
    rand::fill(&mut token);
    crate::store::create_dir_private(config_dir)?;
    crate::store::write_private(&token_path(config_dir), &token)?;
    Ok(token)
}

/// Read the secret a running node left.
pub fn read_token(config_dir: &std::path::Path) -> std::io::Result<Token> {
    let bytes = std::fs::read(token_path(config_dir))?;
    Token::try_from(bytes.as_slice()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the local socket token is not the right size",
        )
    })
}

/// One side's proof that it knows the token, bound to the other side's nonce.
///
/// Keyed rather than a plain hash of secret-and-nonce, and labelled so the two
/// directions cannot be replayed against each other: without the label, the
/// node's answer to a client is exactly what a client owes a node, and an
/// attacker could hold two connections and have each side answer the other.
fn proof(token: &Token, label: &[u8], nonce: &[u8; TOKEN_LEN]) -> blake3::Hash {
    let mut input = Vec::with_capacity(label.len() + TOKEN_LEN);
    input.extend_from_slice(label);
    input.extend_from_slice(nonce);
    blake3::keyed_hash(token, &input)
}

/// A connection that closed before identifying itself.
///
/// Almost always this crate's own liveness probe. Given a type so the accept
/// loop can stay quiet about it without matching on a message.
#[derive(Debug)]
pub struct Probe;

impl std::fmt::Display for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a connection closed before identifying itself")
    }
}

impl std::error::Error for Probe {}

const NODE_LABEL: &[u8] = b"clispeak-ipc-node";
const CLIENT_LABEL: &[u8] = b"clispeak-ipc-client";

/// Prove this is the node, then demand the same of the caller.
///
/// **The node answers first, on purpose.** The local socket is a name any
/// other user on the machine can take — on Linux the abstract namespace has
/// no permissions at all, and on macOS `interprocess` puts it in `/tmp`,
/// which its own source calls "the world-writable temporary directory". So a
/// client that spoke first would hand its text, its invites and its history
/// to whoever got the name (#54).
pub async fn accept_handshake<S>(s: &mut S, token: &Token) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let mut client_nonce = [0u8; TOKEN_LEN];
        // A connection that closes without saying anything is a probe, not a
        // caller: `node_is_listening` and `bind_ipc` both connect and drop to
        // find out whether a node is alive. Distinguished so those do not
        // print a refusal on every app start, which would be a warning about
        // the app's own health check.
        if let Err(e) = s.read_exact(&mut client_nonce).await {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                return Err(anyhow::Error::new(Probe));
            }
            return Err(e.into());
        }

        let mut node_nonce = [0u8; TOKEN_LEN];
        rand::fill(&mut node_nonce);

        s.write_all(proof(token, NODE_LABEL, &client_nonce).as_bytes())
            .await?;
        s.write_all(&node_nonce).await?;
        s.flush().await?;

        let mut offered = [0u8; TOKEN_LEN];
        s.read_exact(&mut offered).await?;
        // `blake3::Hash` compares in constant time, which is why the
        // comparison is between hashes rather than between byte arrays.
        if blake3::Hash::from(offered) != proof(token, CLIENT_LABEL, &node_nonce) {
            bail!("the caller could not prove it may drive this node");
        }
        Ok(())
    })
    .await
    .context("timed out waiting for the caller to identify itself")?
}

/// Check the node is ours, then prove we may drive it.
///
/// Kept here beside the node's half so the two are read together and tested
/// together. `clispeak-cli` carries its own copy — it depends on `proto` and
/// `text` only, deliberately, and the socket name and frame format are
/// duplicated for the same reason.
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
        if blake3::Hash::from(claimed) != proof(token, NODE_LABEL, &client_nonce) {
            // Deliberately not "that is an impostor". Two things produce this
            // and they are indistinguishable from here: something else holds
            // the socket, or the token read is not the one the listener has.
            // Asserting the first would send someone hunting an attacker when
            // they had pointed a CLI at another node's socket.
            bail!(
                "it did not answer with this machine's clispeak token — either \
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
/// `CLISPEAK_SOCKET` overrides it, which is how a second node runs
/// alongside the first — needed to exercise the join flow on one machine.
pub fn socket_name() -> String {
    std::env::var("CLISPEAK_SOCKET").unwrap_or_else(|_| "clispeak.sock".to_string())
}

/// Whether a socket name has been given as though it were a path.
///
/// Not an error: it binds, and both ends map it the same way. But every tool
/// a person reaches for next — `ls`, `lsof`, a systemd unit, a health check —
/// takes it for a path, and on Linux there is no file to find under any name.
/// Worth one line at startup rather than a silent mismatch.
///
/// Deliberately portable: no `cfg` here decides what a path looks like,
/// because `clispeak-core` may not hold one and because a Windows name and a
/// Unix name are both wrong in the same way for this purpose.
pub fn path_shaped(name: &str) -> bool {
    name.contains('/') || name.contains('\\')
}

/// Whether a node is already listening on this machine's socket.
///
/// Asked *before* anything else is brought up. `Node::serve` already refuses
/// to bind a name another node holds, but by then the transport is online: a
/// second endpoint is bound under this device's secret key, presence checks
/// are running, and only the socket is missing. The app then had a window
/// that looked healthy and a node that reached nobody — issue #72.
///
/// Connecting is the test, not the presence of a file. A node that died
/// leaves its name behind on some platforms, and only a refused connection
/// proves nothing is listening — the same reasoning `bind_ipc` uses to tell
/// a live node from a dead one's leftovers.
pub async fn node_is_listening() -> bool {
    use interprocess::local_socket::traits::tokio::Stream as _;
    let Ok(name) = socket_name().to_ns_name::<GenericNamespaced>() else {
        return false;
    };
    interprocess::local_socket::tokio::Stream::connect(name)
        .await
        .is_ok()
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
        assert!(!path_shaped("clispeak.sock"));
        assert!(!path_shaped("clispeak-test-2.sock"));
    }

    /// The value that started #43. It binds, and it is not a path.
    #[test]
    fn a_unix_path_is_path_shaped() {
        assert!(path_shaped("/tmp/vc-d.sock"));
        assert!(path_shaped("run/clispeak.sock"));
    }

    /// Windows names are wrong in the same way, so the check is not
    /// per-platform. `clispeak-core` may not hold a `cfg` in any case.
    #[test]
    fn a_windows_path_is_path_shaped() {
        assert!(path_shaped(r"C:\\temp\\clispeak.sock"));
    }

    /// Whatever the default is, it must not be the thing we warn about.
    ///
    /// Asserted on the literal rather than through `socket_name`, which reads
    /// the environment: a test that mutates `CLISPEAK_SOCKET` would race
    /// every other test in the binary.
    #[test]
    fn the_default_name_passes_its_own_check() {
        assert!(!path_shaped("clispeak.sock"));
    }

    /// Drive both halves against each other over a pipe.
    async fn handshake(node_token: Token, client_token: Token) -> (Result<()>, Result<()>) {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let node = tokio::spawn(async move { accept_handshake(&mut a, &node_token).await });
        let client = offer_handshake(&mut b, &client_token).await;
        // Dropped before the node is awaited. A caller that walks away mid
        // handshake is what a refusal looks like, and holding this open would
        // make the node sit out its full timeout instead — five seconds per
        // test, for nothing.
        drop(b);
        (node.await.expect("node task"), client)
    }

    #[tokio::test]
    async fn the_same_token_on_both_sides_is_accepted() {
        let token = [7u8; TOKEN_LEN];
        let (node, client) = handshake(token, token).await;
        assert!(node.is_ok(), "node rejected a caller with the right token");
        assert!(
            client.is_ok(),
            "caller rejected a node with the right token"
        );
    }

    #[tokio::test]
    async fn a_caller_without_the_token_is_refused_and_told_nothing() {
        // The whole of #54: on Linux any other user can reach this socket,
        // because the abstract namespace carries no permissions at all.
        let (node, client) = handshake([7u8; TOKEN_LEN], [8u8; TOKEN_LEN]).await;
        assert!(node.is_err(), "node accepted a caller with the wrong token");
        assert!(
            client.is_err(),
            "caller accepted a node it could not verify"
        );
    }

    #[tokio::test]
    async fn the_node_proves_itself_before_the_caller_says_anything() {
        // Order matters more than the check does. If the caller spoke first,
        // whoever took the socket name would receive the text, the invites
        // and the history before being found out — so a wrong token has to
        // fail on the *node's* proof, which is the first thing checked.
        let (_node, client) = handshake([1u8; TOKEN_LEN], [2u8; TOKEN_LEN]).await;
        let message = format!("{:#}", client.expect_err("should have refused"));
        assert!(
            message.contains("did not answer with this machine"),
            "the caller should stop at the node's proof, not its own: {message}"
        );
    }

    #[tokio::test]
    async fn a_connection_that_says_nothing_is_a_probe_not_a_refusal() {
        // `node_is_listening` and `bind_ipc` both connect and drop. Printing a
        // refusal for those would warn about the app's own health check.
        let (mut a, b) = tokio::io::duplex(1024);
        drop(b);
        let e = accept_handshake(&mut a, &[3u8; TOKEN_LEN])
            .await
            .expect_err("a closed connection cannot handshake");
        assert!(e.is::<Probe>(), "expected a probe, got: {e:#}");
    }

    #[test]
    fn a_token_is_written_where_only_this_user_can_read_it() {
        let dir = std::env::temp_dir().join(format!("vc-token-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let written = install_token(&dir).expect("writing a token");
        let read = read_token(&dir).expect("reading it back");
        assert_eq!(written, read);
        assert_ne!(written, [0u8; TOKEN_LEN], "a token of zeroes is not random");
        // A second start replaces it: a token outliving its node is a
        // credential nobody is tracking.
        let again = install_token(&dir).expect("writing a second token");
        assert_ne!(written, again);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
