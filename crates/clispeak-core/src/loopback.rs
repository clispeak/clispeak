//! Two or more nodes, wired to each other, inside one process.
//!
//! **What this exists for.** Every pairing, every roster sync, every
//! revocation and every spoken message had been verified by a person doing it
//! by hand on real devices (#80). The cost of that was not the typing; it was
//! that a four-device space dissolving itself took a day to explain, because
//! there was no way to reproduce a multi-node roster merge except with four
//! real devices in front of you (#166).
//!
//! [`Loopback`] is a [`Network`] that carries frames between nodes in memory.
//! No sockets, no QUIC, no relay, no DNS, no network at all — so these tests
//! are deterministic and take milliseconds, and they run on every platform
//! rather than only where a machine happens to have connectivity.
//!
//! **What it is not.** It is not a simulation of the transport. It does not
//! model latency, loss, reordering, hole punching or a peer that vanishes
//! mid-stream, and a test written here proves nothing about any of those. It
//! carries the same `PeerMessage` frames through the same `handle_peer`,
//! `do_join`, `sync_roster` and `send_to_peer` the real transport does, and
//! that is the whole claim. NAT traversal stays a thing tested on hardware,
//! and `CLAUDE.md` says so.
//!
//! Test-only: `#[cfg(test)]` in `lib.rs`, so none of this reaches a shipped
//! binary.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use iroh::EndpointId;
use tokio::sync::mpsc;

use crate::transport::{BoxFuture, Network, Reader, Streams, Wire, Writer};

/// How much a stream buffers before a writer waits, in bytes.
///
/// Generous on purpose. A real QUIC stream applies backpressure and the code
/// under test is written for it, but a *deadlock* caused by this number being
/// too small would look exactly like a protocol bug, and the point of these
/// tests is to make protocol bugs legible.
const STREAM_BUFFER: usize = 256 * 1024;

/// Where the nodes find each other.
///
/// One per test. Nodes register as they are built, and [`Loopback::connect`]
/// refuses a peer that is not on it — which is the closest thing here to
/// "that device is not reachable".
#[derive(Clone, Default)]
pub struct Switchboard {
    inboxes: Arc<Mutex<HashMap<EndpointId, mpsc::UnboundedSender<LoopWire>>>>,
}

impl Switchboard {
    pub fn new() -> Self {
        Self::default()
    }

    /// A network endpoint for this identity, reachable by every other one on
    /// the same switchboard.
    pub fn endpoint(&self, id: EndpointId) -> Loopback {
        let (tx, rx) = mpsc::unbounded_channel();
        self.inboxes.lock().expect("inboxes").insert(id, tx);
        Loopback {
            id,
            board: self.clone(),
            inbox: tokio::sync::Mutex::new(rx),
            open: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }
}

/// One device's place on a [`Switchboard`].
pub struct Loopback {
    id: EndpointId,
    board: Switchboard,
    inbox: tokio::sync::Mutex<mpsc::UnboundedReceiver<LoopWire>>,
    open: Arc<std::sync::atomic::AtomicBool>,
}

/// One connection between two nodes.
///
/// Streams travel whole: opening one builds both directions and hands the far
/// halves to the peer's queue, so `accept_bi` on the other side yields exactly
/// what `open_bi` produced here.
pub struct LoopWire {
    remote: EndpointId,
    /// Streams this side opened, waiting to be accepted over there.
    to_peer: mpsc::UnboundedSender<Streams>,
    /// Streams the peer opened, waiting to be accepted here.
    from_peer: tokio::sync::Mutex<mpsc::UnboundedReceiver<Streams>>,
}

impl Wire for LoopWire {
    fn remote(&self) -> EndpointId {
        self.remote
    }

    fn open_bi(&self) -> BoxFuture<'_, Result<Streams>> {
        Box::pin(async move {
            // Two pipes rather than one split in half: each side then holds a
            // whole `DuplexStream` for writing and another for reading, which
            // is what `SendHalf` is implemented for. Splitting would work too
            // and would need a second implementation to say the same thing.
            let (mine_w, theirs_r) = tokio::io::duplex(STREAM_BUFFER);
            let (theirs_w, mine_r) = tokio::io::duplex(STREAM_BUFFER);
            let theirs: Streams = (Box::new(theirs_w) as Writer, Box::new(theirs_r) as Reader);
            if self.to_peer.send(theirs).is_err() {
                bail!("the peer is gone");
            }
            Ok((Box::new(mine_w) as Writer, Box::new(mine_r) as Reader))
        })
    }

    fn accept_bi(&self) -> BoxFuture<'_, Option<Streams>> {
        Box::pin(async move { self.from_peer.lock().await.recv().await })
    }
}

impl Network for Loopback {
    fn id(&self) -> EndpointId {
        self.id
    }

    fn online(&self) -> BoxFuture<'_, ()> {
        // Already reachable: there is nothing to publish and nothing to
        // resolve.
        Box::pin(std::future::ready(()))
    }

    fn connect(&self, peer: EndpointId) -> BoxFuture<'_, Result<Box<dyn Wire>>> {
        Box::pin(async move {
            let inbox = self
                .board
                .inboxes
                .lock()
                .expect("inboxes")
                .get(&peer)
                .cloned();
            let Some(inbox) = inbox else {
                bail!("no such device on this switchboard: {peer}");
            };
            // One connection, two views of it. Streams opened here surface
            // over there and the other way round, which is what makes a
            // roster sync — where each side opens its own stream — behave the
            // way it does on a real connection.
            let (here_tx, there_rx) = mpsc::unbounded_channel();
            let (there_tx, here_rx) = mpsc::unbounded_channel();
            let theirs = LoopWire {
                remote: self.id,
                to_peer: there_tx,
                from_peer: tokio::sync::Mutex::new(there_rx),
            };
            if inbox.send(theirs).is_err() {
                bail!("device {peer} has stopped listening");
            }
            Ok(Box::new(LoopWire {
                remote: peer,
                to_peer: here_tx,
                from_peer: tokio::sync::Mutex::new(here_rx),
            }) as Box<dyn Wire>)
        })
    }

    fn accept(&self) -> BoxFuture<'_, Option<Result<Box<dyn Wire>>>> {
        Box::pin(async move {
            if !self.open.load(std::sync::atomic::Ordering::SeqCst) {
                return None;
            }
            let wire = self.inbox.lock().await.recv().await?;
            Some(Ok(Box::new(wire) as Box<dyn Wire>))
        })
    }

    fn close(&self) -> BoxFuture<'_, ()> {
        self.board.inboxes.lock().expect("inboxes").remove(&self.id);
        self.open.store(false, std::sync::atomic::Ordering::SeqCst);
        Box::pin(std::future::ready(()))
    }
}
