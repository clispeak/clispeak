//! The node: accepts local requests, speaks them, and relays them to peers.
//!
//! Two loops run side by side — a local IPC socket for the CLI, and an iroh
//! endpoint for other devices. The CLI is a thin client that hands over text
//! and exits; everything durable lives here.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result};
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, ToNsName, tokio::Stream, traits::tokio::Listener,
};
use tokio::sync::Mutex;
use voicecast_engine::SpeechEngine;
use voicecast_proto::{DeviceInfo, Member, PeerMessage, Priority, Request, Response, Status};
use voicecast_text::chunk;

use crate::ipc::{read_frame, socket_name, write_frame};
use crate::transport::{read_msg, write_msg};
use crate::{Identity, Roster, Ticket, Transport};

/// A speech request waiting its turn.
struct Job {
    msg_id: String,
    chunks: Vec<String>,
}

/// Shared state both loops need.
/// What to do when the CLI asks for the window or for shutdown.
///
/// The node owns no UI, so the app installs these. Without an app — a headless
/// `voicecastd` — `show` has nothing to do and `quit` still works.
pub type WindowHook = Arc<dyn Fn() + Send + Sync>;

struct Shared {
    engine: Arc<dyn SpeechEngine>,
    identity: Identity,
    name: String,
    roster: Mutex<Roster>,
    roster_path: PathBuf,
    /// The invite currently outstanding, if any. One at a time: an invite is
    /// a deliberate act, and allowing several open at once would widen the
    /// window in which a leaked ticket still works.
    pending: Mutex<Option<Ticket>>,
    tx: tokio::sync::mpsc::UnboundedSender<Job>,
    queued: AtomicUsize,
    on_show: Mutex<Option<WindowHook>>,
    on_quit: Mutex<Option<WindowHook>>,
}

/// The running node.
pub struct Node {
    shared: Arc<Shared>,
    transport: Arc<Transport>,
}

impl Node {
    /// Start a node with the given engine, identity and transport.
    pub async fn new(
        engine: Arc<dyn SpeechEngine>,
        identity: Identity,
        transport: Transport,
        name: String,
    ) -> Result<Self> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Job>();

        let roster_path = Roster::default_path().context("locating roster")?;
        let mut roster = Roster::load(&roster_path).context("loading roster")?;
        // A device is always a member of its own space, even before anyone
        // else joins — otherwise it could not speak to itself.
        if roster.members().count() == 0 {
            roster = Roster::found(identity.secret(), &name);
            roster.save(&roster_path).context("saving roster")?;
        } else if roster.rename(&identity.id().to_string(), &name) {
            // The label is captured when the device joins; without this a
            // rename would show everywhere except in the device's own list.
            roster.save(&roster_path).context("saving roster")?;
        }

        let worker_engine = Arc::clone(&engine);
        let queued = AtomicUsize::new(0);
        let shared = Arc::new(Shared {
            engine,
            identity,
            name,
            roster: Mutex::new(roster),
            roster_path,
            pending: Mutex::new(None),
            tx,
            queued,
            on_show: Mutex::new(None),
            on_quit: Mutex::new(None),
        });

        let counter = Arc::clone(&shared);
        // Speaking blocks, so it gets a dedicated thread rather than starving
        // the runtime that is accepting connections.
        std::thread::spawn(move || {
            while let Some(job) = rx.blocking_recv() {
                for c in &job.chunks {
                    if let Err(e) = worker_engine.speak(c) {
                        eprintln!("[{}] speech failed: {e}", job.msg_id);
                        break;
                    }
                }
                counter.queued.fetch_sub(1, Ordering::SeqCst);
            }
        });

        Ok(Self {
            shared,
            transport: Arc::new(transport),
        })
    }

    /// Install what `voicecast show` and `voicecast quit` should do.
    ///
    /// Called by the app so the CLI can reach a window it cannot see — which
    /// matters most where the tray icon fails to appear, leaving no other way
    /// back to a hidden app.
    pub async fn set_window_hooks(&self, on_show: WindowHook, on_quit: WindowHook) {
        *self.shared.on_show.lock().await = Some(on_show);
        *self.shared.on_quit.lock().await = Some(on_quit);
    }

    /// Speak text here, or on a named peer.
    pub async fn speak(&self, text: String, priority: Priority, to: Option<String>) -> Response {
        speak(&self.shared, &self.transport, text, priority, to).await
    }

    /// Mint an invite for another device.
    pub async fn invite(&self) -> Response {
        invite(&self.shared).await
    }

    /// Join a space using someone else's invite.
    pub async fn join(&self, ticket: &str) -> Response {
        join(&self.shared, &self.transport, ticket).await
    }

    /// Devices in this space.
    pub async fn devices(&self) -> Response {
        devices(&self.shared).await
    }

    /// This node's health.
    pub fn status(&self) -> Response {
        status(&self.shared)
    }

    /// This device's local label.
    pub fn name(&self) -> &str {
        &self.shared.name
    }

    /// Stop whatever is being spoken.
    pub fn stop(&self) {
        self.shared.engine.stop();
    }

    /// Serve peers only.
    ///
    /// What the mobile app runs: there is no CLI on a phone, so binding a
    /// local IPC socket would be pointless and, on Android, may not work at
    /// all.
    pub async fn serve_peers(&self) -> Result<()> {
        let shared = Arc::clone(&self.shared);
        let transport = Arc::clone(&self.transport);
        while let Some(conn) = transport.accept().await {
            let shared = Arc::clone(&shared);
            match conn {
                Ok(conn) => {
                    tokio::spawn(async move {
                        if let Err(e) = handle_peer(&shared, conn).await {
                            eprintln!("peer: {e:#}");
                        }
                    });
                }
                Err(e) => eprintln!("peer handshake: {e:#}"),
            }
        }
        Ok(())
    }

    /// This device's public key.
    pub fn id(&self) -> String {
        self.shared.identity.id().to_string()
    }

    /// Run both loops until one of them fails.
    pub async fn serve(&self) -> Result<()> {
        let name = socket_name().to_ns_name::<GenericNamespaced>()?;
        let listener = ListenerOptions::new()
            .name(name)
            .create_tokio()
            .context("another node may already be running")?;

        eprintln!(
            "listening on {} and as {}",
            socket_name(),
            self.transport.id()
        );

        let ipc = {
            let shared = Arc::clone(&self.shared);
            let transport = Arc::clone(&self.transport);
            async move {
                loop {
                    let stream = listener
                        .accept()
                        .await
                        .context("accepting CLI connection")?;
                    if let Err(e) = handle_cli(&shared, &transport, stream).await {
                        eprintln!("cli: {e:#}");
                    }
                }
            }
        };

        let peers = {
            let shared = Arc::clone(&self.shared);
            let transport = Arc::clone(&self.transport);
            async move {
                while let Some(conn) = transport.accept().await {
                    let shared = Arc::clone(&shared);
                    match conn {
                        Ok(conn) => {
                            tokio::spawn(async move {
                                if let Err(e) = handle_peer(&shared, conn).await {
                                    eprintln!("peer: {e:#}");
                                }
                            });
                        }
                        Err(e) => eprintln!("peer handshake: {e:#}"),
                    }
                }
                Ok::<_, anyhow::Error>(())
            }
        };

        tokio::select! {
            r = ipc => r,
            r = peers => r,
        }
    }
}

/// Serve one CLI connection.
async fn handle_cli(shared: &Arc<Shared>, transport: &Arc<Transport>, mut s: Stream) -> Result<()> {
    let request: Request = read_frame(&mut s).await?;
    let response = match request {
        Request::Speak { text, priority, to } => speak(shared, transport, text, priority, to).await,
        Request::Stop => {
            shared.engine.stop();
            Response::Finished {
                status: Status::Cancelled,
            }
        }
        Request::Invite => invite(shared).await,
        Request::Join { ticket } => join(shared, transport, &ticket).await,
        Request::Devices => devices(shared).await,
        Request::Rename { name } => rename(shared, &name).await,
        Request::Show => match shared.on_show.lock().await.as_ref() {
            Some(hook) => {
                hook();
                Response::Done
            }
            None => Response::Error {
                message: "this node has no window".into(),
            },
        },
        Request::Quit => {
            let hook = shared.on_quit.lock().await.clone();
            // Reply before exiting, or the CLI sees a closed socket instead of
            // an acknowledgement.
            match hook {
                Some(hook) => {
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                        hook();
                    });
                    Response::Done
                }
                None => {
                    tokio::spawn(async {
                        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                        std::process::exit(0);
                    });
                    Response::Done
                }
            }
        }
        Request::Status => Response::Status {
            device_id: shared.identity.id().to_string(),
            key_store: shared.identity.location().to_string(),
            engine: shared
                .engine
                .voices()
                .first()
                .map_or("unknown", |v| &v.name)
                .to_string(),
            fallback: shared.engine.tier() == voicecast_engine::Tier::Fallback,
            queued: shared.queued.load(Ordering::SeqCst),
        },
    };
    write_frame(&mut s, &response).await
}

/// This node's health.
fn status(shared: &Arc<Shared>) -> Response {
    Response::Status {
        device_id: shared.identity.id().to_string(),
        key_store: shared.identity.location().to_string(),
        engine: shared
            .engine
            .voices()
            .first()
            .map_or("unknown", |v| &v.name)
            .to_string(),
        fallback: shared.engine.tier() == voicecast_engine::Tier::Fallback,
        queued: shared.queued.load(Ordering::SeqCst),
    }
}

/// Speak locally, or relay to a named peer.
async fn speak(
    shared: &Arc<Shared>,
    transport: &Arc<Transport>,
    text: String,
    priority: Priority,
    to: Option<String>,
) -> Response {
    let chunks = chunk(&text);
    if chunks.is_empty() {
        return Response::Error {
            message: "nothing to say".into(),
        };
    }
    let msg_id = new_msg_id();

    let Some(target) = to else {
        return enqueue(shared, msg_id, chunks, priority);
    };

    if target == shared.name || target == "here" {
        return enqueue(shared, msg_id, chunks, priority);
    }

    let peer = {
        let roster = shared.roster.lock().await;
        match roster.by_name(&target) {
            Some(m) => m.endpoint_id.clone(),
            None => {
                return Response::Error {
                    message: format!("no device named '{target}' in this space"),
                };
            }
        }
    };

    match send_to_peer(transport, &peer, &msg_id, &chunks, priority).await {
        Ok(status) => Response::Finished { status },
        Err(e) => Response::Error {
            message: format!("could not reach '{target}': {e:#}"),
        },
    }
}

/// Queue chunks for the local engine.
fn enqueue(shared: &Arc<Shared>, msg_id: String, chunks: Vec<String>, p: Priority) -> Response {
    // High priority interrupts. Resuming the interrupted message at its chunk
    // boundary is M8; for now it is dropped, which is honest but not yet what
    // docs/cli.md promises.
    if p == Priority::High {
        shared.engine.stop();
    }
    shared.queued.fetch_add(1, Ordering::SeqCst);
    match shared.tx.send(Job {
        msg_id: msg_id.clone(),
        chunks,
    }) {
        Ok(()) => Response::Accepted { msg_id },
        Err(_) => Response::Error {
            message: "speech worker has stopped".into(),
        },
    }
}

/// Open a stream to a peer and stream the message down it.
async fn send_to_peer(
    transport: &Arc<Transport>,
    peer_id: &str,
    msg_id: &str,
    chunks: &[String],
    priority: Priority,
) -> Result<Status> {
    let peer = peer_id.parse().context("bad endpoint id in roster")?;
    let conn = transport.connect(peer).await?;
    let (mut send, mut recv) = conn.open_bi().await.context("opening message stream")?;

    write_msg(
        &mut send,
        &PeerMessage::SpeakBegin {
            msg_id: msg_id.into(),
            priority,
        },
    )
    .await?;
    for (seq, text) in chunks.iter().enumerate() {
        write_msg(
            &mut send,
            &PeerMessage::Chunk {
                seq: seq as u32,
                text: text.clone(),
            },
        )
        .await?;
    }
    write_msg(&mut send, &PeerMessage::SpeakEnd).await?;

    match read_msg(&mut recv).await? {
        PeerMessage::Report { status } => Ok(status),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

/// Mint an invite.
async fn invite(shared: &Arc<Shared>) -> Response {
    let ticket = Ticket::mint(shared.identity.id().to_string());
    let url = match ticket.to_url() {
        Ok(u) => u,
        Err(e) => {
            return Response::Error {
                message: e.to_string(),
            };
        }
    };
    let expires_in = ticket.remaining();
    *shared.pending.lock().await = Some(ticket);
    Response::Invite { url, expires_in }
}

/// Join a space using someone else's ticket.
async fn join(shared: &Arc<Shared>, transport: &Arc<Transport>, raw: &str) -> Response {
    let ticket = match Ticket::parse(raw) {
        Ok(t) => t,
        Err(e) => {
            return Response::Error {
                message: format!("{e:#}"),
            };
        }
    };
    match do_join(shared, transport, &ticket).await {
        Ok(count) => Response::Joined { members: count },
        Err(e) => Response::Error {
            message: format!("{e:#}"),
        },
    }
}

async fn do_join(shared: &Arc<Shared>, transport: &Arc<Transport>, t: &Ticket) -> Result<usize> {
    let peer = t.endpoint_id.parse().context("bad endpoint id in ticket")?;
    let conn = transport
        .connect(peer)
        .await
        .context("reaching the inviting device")?;
    let (mut send, mut recv) = conn.open_bi().await?;

    write_msg(
        &mut send,
        &PeerMessage::JoinRequest {
            endpoint_id: shared.identity.id().to_string(),
            display_name: shared.name.clone(),
            token: t.token.clone(),
        },
    )
    .await?;

    match read_msg(&mut recv).await? {
        PeerMessage::JoinAccepted { member, members } => {
            let mut roster = shared.roster.lock().await;
            // Replace rather than merge: joining a space means adopting its
            // membership, not blending it with whatever we had before.
            let all = members.into_iter().chain(std::iter::once(member));
            *roster = Roster::adopt(all);
            roster.save(&shared.roster_path)?;
            Ok(roster.members().count())
        }
        PeerMessage::JoinRefused { reason } => anyhow::bail!("{reason}"),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

/// Change this device's label.
///
/// Local only. Peers keep the old label until roster sync exists, so this
/// says so rather than implying the change travelled.
async fn rename(shared: &Arc<Shared>, name: &str) -> Response {
    let name = name.trim();
    if name.is_empty() {
        return Response::Error {
            message: "a device name cannot be empty".into(),
        };
    }
    if let Err(e) = crate::set_device_name(name) {
        return Response::Error {
            message: e.to_string(),
        };
    }
    let mut roster = shared.roster.lock().await;
    roster.rename(&shared.identity.id().to_string(), name);
    if let Err(e) = roster.save(&shared.roster_path) {
        return Response::Error {
            message: e.to_string(),
        };
    }
    Response::Renamed {
        name: name.to_string(),
    }
}

/// List the space's devices.
async fn devices(shared: &Arc<Shared>) -> Response {
    let me = shared.identity.id().to_string();
    let roster = shared.roster.lock().await;
    Response::Devices {
        devices: roster
            .members()
            .map(|m| DeviceInfo {
                name: m.name.clone(),
                endpoint_id: m.endpoint_id.clone(),
                is_self: m.endpoint_id == me,
            })
            .collect(),
    }
}

/// Serve one peer connection.
async fn handle_peer(shared: &Arc<Shared>, conn: iroh::endpoint::Connection) -> Result<()> {
    let remote = conn.remote_id();
    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
        match read_msg(&mut recv).await? {
            PeerMessage::JoinRequest {
                endpoint_id,
                display_name,
                token,
            } => {
                let reply = accept_join(shared, &endpoint_id, &display_name, &token).await;
                write_msg(&mut send, &reply).await?;
            }
            PeerMessage::SpeakBegin { msg_id, priority } => {
                // Authorisation is the roster, and nothing else: an unpaired
                // device cannot make this one speak.
                let allowed = shared.roster.lock().await.allows(&remote);
                if !allowed {
                    write_msg(
                        &mut send,
                        &PeerMessage::Report {
                            status: Status::Rejected,
                        },
                    )
                    .await?;
                    continue;
                }
                let mut chunks = Vec::new();
                loop {
                    match read_msg(&mut recv).await? {
                        PeerMessage::Chunk { text, .. } => chunks.push(text),
                        PeerMessage::SpeakEnd => break,
                        other => anyhow::bail!("unexpected in message stream: {other:?}"),
                    }
                }
                let status = match enqueue(shared, msg_id, chunks, priority) {
                    Response::Accepted { .. } => Status::Queued,
                    _ => Status::Dropped,
                };
                write_msg(&mut send, &PeerMessage::Report { status }).await?;
            }
            PeerMessage::Hello { .. } => {}
            other => anyhow::bail!("unexpected message: {other:?}"),
        }
    }
    Ok(())
}

/// Decide whether to admit a joiner, and sign its record if so.
async fn accept_join(
    shared: &Arc<Shared>,
    endpoint_id: &str,
    name: &str,
    token: &str,
) -> PeerMessage {
    let mut pending = shared.pending.lock().await;
    let Some(ticket) = pending.as_ref() else {
        return PeerMessage::JoinRefused {
            reason: "no invite is open on this device; run `voicecast invite` first".into(),
        };
    };
    if !ticket.is_valid() {
        *pending = None;
        return PeerMessage::JoinRefused {
            reason: "that invite has expired".into(),
        };
    }
    if ticket.token != token {
        return PeerMessage::JoinRefused {
            reason: "that invite is not valid".into(),
        };
    }
    // Single use: consumed here so a ticket seen over a shoulder, or left in
    // scrollback, cannot be replayed.
    *pending = None;
    drop(pending);

    let mut r = shared.roster.lock().await;
    let member = r.invite(shared.identity.secret(), endpoint_id, name);
    if let Err(e) = r.save(&shared.roster_path) {
        return PeerMessage::JoinRefused {
            reason: format!("could not record membership: {e}"),
        };
    }
    let members: Vec<Member> = r.members().cloned().collect();
    PeerMessage::JoinAccepted { member, members }
}

/// A short, unique-enough message id.
fn new_msg_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("m_{:x}", nanos as u64 & 0xffff_ffff)
}
