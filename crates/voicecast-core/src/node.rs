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
    /// When each peer was last reached, as unix seconds.
    ///
    /// Recorded from real contact rather than a separate heartbeat protocol:
    /// every sync and every message already proves reachability, so a device
    /// that is being used needs no extra traffic to look alive.
    last_seen: Mutex<std::collections::HashMap<String, u64>>,
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
        } else if roster.rename(&identity.id().to_string(), &name)
            || roster.stamp_own_label(&identity.id().to_string())
        {
            // Two reasons to rewrite: the label changed, or it predates the
            // `renamed_at` stamp. Rosters written before that field existed
            // deserialize it as zero, and a merge comparing 0 > 0 keeps the
            // stale copy forever — so an unstamped entry is stamped once, on
            // the device that owns it.
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
            last_seen: Mutex::new(std::collections::HashMap::new()),
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

    /// Change this device's label.
    pub async fn rename(&self, name: &str) -> Response {
        rename(&self.shared, name).await
    }

    /// Remove another device from this space.
    pub async fn revoke(&self, name: &str) -> Response {
        revoke(&self.shared, name).await
    }

    /// Leave the space, keeping this device's identity.
    pub async fn leave(&self) -> Response {
        leave(&self.shared, &self.transport).await
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

    /// Check in with every peer on a timer, so presence stays current.
    ///
    /// A minute is a compromise: often enough that a device which dropped off
    /// shows as stale reasonably soon, rare enough that idle devices are not
    /// kept awake by us. Real traffic already refreshes presence, so this only
    /// matters when nothing is being said.
    pub fn start_presence_checks(self: &Arc<Self>) {
        let node = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                tick.tick().await;
                let me = node.shared.identity.id().to_string();
                let peers: Vec<String> = {
                    let roster = node.shared.roster.lock().await;
                    roster
                        .members()
                        .filter(|m| m.endpoint_id != me)
                        .map(|m| m.endpoint_id.clone())
                        .collect()
                };
                for peer in peers {
                    // Failure is the useful signal here: the peer simply stays
                    // stale until it can be reached again.
                    if let Ok(id) = peer.parse()
                        && let Ok(conn) = node.transport.connect(id).await
                    {
                        let _ = sync_roster(&node.shared, &conn).await;
                    }
                }
            }
        });
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
        Request::Revoke { name } => revoke(shared, &name).await,
        Request::Leave => leave(shared, transport).await,
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

/// Speak here, on a named peer, or on everything in the space.
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

    let target = to.as_deref().unwrap_or("here");

    if target == "all" {
        return speak_everywhere(shared, transport, &msg_id, &chunks, priority).await;
    }

    if target == "here" || target == shared.name {
        return enqueue(shared, msg_id, chunks, priority);
    }

    let peer = {
        let roster = shared.roster.lock().await;
        match roster.by_name(target) {
            Some(m) => m.endpoint_id.clone(),
            None => {
                return Response::Error {
                    message: format!("no device named '{target}' in this space"),
                };
            }
        }
    };

    match send_to_peer(shared, transport, &peer, &msg_id, &chunks, priority).await {
        Ok(status) => Response::Finished { status },
        Err(e) => Response::Error {
            message: format!("could not reach '{target}': {e:#}"),
        },
    }
}

/// Speak on every device in the space, this one included.
///
/// Failures are reported rather than aborting the rest: one unreachable phone
/// should not stop the laptop from speaking. Per-target detail arrives with
/// `--wait`/`--json`; for now the summary says what did not work.
async fn speak_everywhere(
    shared: &Arc<Shared>,
    transport: &Arc<Transport>,
    msg_id: &str,
    chunks: &[String],
    priority: Priority,
) -> Response {
    let me = shared.identity.id().to_string();
    let peers: Vec<(String, String)> = {
        let roster = shared.roster.lock().await;
        roster
            .members()
            .filter(|m| m.endpoint_id != me)
            .map(|m| (m.name.clone(), m.endpoint_id.clone()))
            .collect()
    };

    let local = enqueue(shared, msg_id.to_string(), chunks.to_vec(), priority);
    let mut spoken = matches!(local, Response::Accepted { .. });
    let mut failures = Vec::new();
    if let Response::Error { message } = &local {
        failures.push(format!("{}: {message}", shared.name));
    }

    for (name, peer) in peers {
        match send_to_peer(shared, transport, &peer, msg_id, chunks, priority).await {
            Ok(Status::Queued | Status::Speaking | Status::Spoken) => spoken = true,
            Ok(status) => failures.push(format!("{name}: {status:?}")),
            Err(e) => failures.push(format!("{name}: {e:#}")),
        }
    }

    if spoken && failures.is_empty() {
        Response::Accepted {
            msg_id: msg_id.to_string(),
        }
    } else if spoken {
        Response::Error {
            message: format!("spoken, except — {}", failures.join("; ")),
        }
    } else {
        Response::Error {
            message: format!("nothing was spoken — {}", failures.join("; ")),
        }
    }
}

/// Queue chunks for the local engine.
fn enqueue(shared: &Arc<Shared>, msg_id: String, chunks: Vec<String>, p: Priority) -> Response {
    // Refuse before accepting, so the sender is told rather than the failure
    // being buried in this device's log.
    if let Err(e) = shared.engine.ready() {
        return Response::Error {
            message: e.to_string(),
        };
    }
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

/// Exchange rosters with a peer, merging what they know into what we know.
///
/// Sync happens on contact rather than on a schedule: devices talk to each
/// other when there is something to say, and that is exactly when a stale
/// roster would be noticed. Without this, a rename or a newly joined device
/// never reaches anyone — which is what `voicecast rename` had to warn about.
async fn sync_roster(shared: &Arc<Shared>, conn: &iroh::endpoint::Connection) -> Result<()> {
    let (mut send, mut recv) = conn.open_bi().await.context("opening roster stream")?;
    let mine = {
        let roster = shared.roster.lock().await;
        PeerMessage::RosterSync {
            members: roster.members().cloned().collect(),
            revoked: roster.tombstones(),
        }
    };
    write_msg(&mut send, &mine).await?;
    send.finish().ok();

    match read_msg(&mut recv).await? {
        PeerMessage::RosterSync { members, revoked } => {
            merge_from_peer(shared, members, revoked).await?;
        }
        // The peer no longer counts us as a member — most likely it left, or
        // removed us. Not an error worth shouting about.
        PeerMessage::JoinRefused { .. } => {}
        other => anyhow::bail!("unexpected reply to roster sync: {other:?}"),
    }
    mark_seen(shared, &conn.remote_id().to_string()).await;
    Ok(())
}

/// Merge a peer's roster into ours and persist the result.
async fn merge_from_peer(
    shared: &Arc<Shared>,
    members: Vec<Member>,
    revoked: Vec<(String, u64)>,
) -> Result<()> {
    let mut roster = shared.roster.lock().await;
    let theirs = Roster::from_parts(members, revoked);
    roster.merge(&theirs);
    // Our own label is ours to decide; a peer's older copy must not overwrite
    // a rename we just made.
    roster.rename(&shared.identity.id().to_string(), &shared.name);
    roster.save(&shared.roster_path)?;
    Ok(())
}

/// Open a stream to a peer and stream the message down it.
async fn send_to_peer(
    shared: &Arc<Shared>,
    transport: &Arc<Transport>,
    peer_id: &str,
    msg_id: &str,
    chunks: &[String],
    priority: Priority,
) -> Result<Status> {
    let peer = peer_id.parse().context("bad endpoint id in roster")?;
    let conn = transport.connect(peer).await?;

    // Piggyback a roster exchange: this is the moment both sides are known to
    // be reachable, so it costs one extra stream and keeps names and
    // membership converging without any background chatter.
    if let Err(e) = sync_roster(shared, &conn).await {
        eprintln!("roster sync with {peer_id}: {e:#}");
    }

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

/// Remove another device from this space.
///
/// Local: the tombstone reaches other members as they sync, and the removed
/// device keeps working until it does. That is the eventual consistency the
/// architecture accepts, and the message says so rather than implying the
/// removal was instant everywhere.
async fn revoke(shared: &Arc<Shared>, name: &str) -> Response {
    let me = shared.identity.id().to_string();
    let mut roster = shared.roster.lock().await;

    let Some(target) = roster.by_name(name).map(|m| m.endpoint_id.clone()) else {
        return Response::Error {
            message: format!("no device named '{name}' in this space"),
        };
    };
    if target == me {
        return Response::Error {
            message: "that is this device — use `voicecast leave` instead".into(),
        };
    }

    roster.revoke(&target);
    if let Err(e) = roster.save(&shared.roster_path) {
        return Response::Error {
            message: e.to_string(),
        };
    }
    Response::Renamed {
        name: format!("removed {name}"),
    }
}

/// Leave the space, keeping this device's identity.
///
/// Two actions, as `docs/architecture.md` requires: tell the others, then drop
/// the roster locally. Doing only the second leaves this device still listed
/// everywhere else — which is exactly how it looked when it was missing.
///
/// Telling them is best-effort; the local removal is not. Leaving must work
/// with no network at all.
async fn leave(shared: &Arc<Shared>, transport: &Arc<Transport>) -> Response {
    let me = shared.identity.id().to_string();

    // A roster carrying our own tombstone. Peers merging this drop us.
    let (farewell, peers) = {
        let roster = shared.roster.lock().await;
        let mut goodbye = roster.clone();
        goodbye.revoke(&me);
        let peers: Vec<String> = roster
            .members()
            .filter(|m| m.endpoint_id != me)
            .map(|m| m.endpoint_id.clone())
            .collect();
        (goodbye, peers)
    };

    let mut told = 0usize;
    for peer in &peers {
        if announce_departure(transport, peer, &farewell).await.is_ok() {
            told += 1;
        }
    }

    let mut roster = shared.roster.lock().await;
    *roster = Roster::leave(shared.identity.secret(), &shared.name);
    if let Err(e) = roster.save(&shared.roster_path) {
        return Response::Error {
            message: e.to_string(),
        };
    }

    let unreached = peers.len() - told;
    Response::Renamed {
        name: if unreached == 0 {
            "left the space".into()
        } else {
            format!("left the space; {unreached} device(s) will find out when next reached")
        },
    }
}

/// Push a roster carrying our own tombstone to one peer.
async fn announce_departure(
    transport: &Arc<Transport>,
    peer_id: &str,
    farewell: &Roster,
) -> Result<()> {
    let peer = peer_id.parse().context("bad endpoint id in roster")?;
    let conn = transport.connect(peer).await?;
    let (mut send, _recv) = conn.open_bi().await?;
    write_msg(
        &mut send,
        &PeerMessage::RosterSync {
            members: farewell.members().cloned().collect(),
            revoked: farewell.tombstones(),
        },
    )
    .await?;
    send.finish().ok();
    Ok(())
}

/// List the space's devices.
async fn devices(shared: &Arc<Shared>) -> Response {
    let me = shared.identity.id().to_string();
    let seen = shared.last_seen.lock().await.clone();
    let roster = shared.roster.lock().await;
    Response::Devices {
        devices: roster
            .members()
            .map(|m| DeviceInfo {
                last_seen_secs: if m.endpoint_id == me {
                    Some(0)
                } else {
                    seen.get(&m.endpoint_id)
                        .map(|t| now_secs().saturating_sub(*t))
                },
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
    // Anything reaching us proves that peer is alive right now.
    mark_seen(shared, &remote.to_string()).await;
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
                    // The peer needs to know this device cannot speak, not
                    // just that something went wrong.
                    Response::Error { .. } => Status::NoEngine,
                    _ => Status::Dropped,
                };
                write_msg(&mut send, &PeerMessage::Report { status }).await?;
            }
            PeerMessage::RosterSync { members, revoked } => {
                // Only members may change our roster. Without this any device
                // that can reach us could inject entries — and, more visibly,
                // a device we just left would push us straight back into the
                // space it still thinks we are in.
                if !shared.roster.lock().await.allows(&remote) {
                    write_msg(
                        &mut send,
                        &PeerMessage::JoinRefused {
                            reason: "not a member of this space".into(),
                        },
                    )
                    .await?;
                    continue;
                }
                let mine = {
                    let roster = shared.roster.lock().await;
                    PeerMessage::RosterSync {
                        members: roster.members().cloned().collect(),
                        revoked: roster.tombstones(),
                    }
                };
                write_msg(&mut send, &mine).await?;
                merge_from_peer(shared, members, revoked).await?;
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

/// Note that a peer was reachable just now.
async fn mark_seen(shared: &Arc<Shared>, peer: &str) {
    shared
        .last_seen
        .lock()
        .await
        .insert(peer.to_string(), now_secs());
}

/// Unix seconds now.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A short, unique-enough message id.
fn new_msg_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("m_{:x}", nanos as u64 & 0xffff_ffff)
}
