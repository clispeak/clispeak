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
use voicecast_proto::{
    DeviceInfo, Member, PeerMessage, Priority, Request, Response, Status, TargetResult,
};
use voicecast_text::chunk;

use crate::ipc::{read_frame, socket_name, write_frame};
use crate::policy::{self, Policy};
use crate::transport::{read_msg, write_msg};
use crate::{Identity, Roster, Ticket, Transport};

/// A speech request waiting its turn.
struct Job {
    msg_id: String,
    chunks: Vec<String>,
    /// Signalled when speaking ends, for callers that asked to wait.
    ///
    /// Optional because the common case is fire-and-forget: an agent firing
    /// notifications should not pay for machinery it is not using.
    done: Option<tokio::sync::oneshot::Sender<Status>>,
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
    /// Whether this device is willing to speak right now.
    ///
    /// A plain mutex, not the async one: it is consulted from the synchronous
    /// enqueue path, and holding it spans a copy of a handful of bytes.
    policy: std::sync::Mutex<Policy>,
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

        // Apply a remembered voice before anything can be spoken with the
        // wrong one.
        if let Some((voice, rate)) = crate::load_voice_settings() {
            let _ = engine.set_voice(&voice);
            let _ = engine.set_rate(rate);
        }

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
            policy: std::sync::Mutex::new(policy::load()),
            on_show: Mutex::new(None),
            on_quit: Mutex::new(None),
        });

        let counter = Arc::clone(&shared);
        // Speaking blocks, so it gets a dedicated thread rather than starving
        // the runtime that is accepting connections.
        std::thread::spawn(move || {
            while let Some(job) = rx.blocking_recv() {
                let mut outcome = Status::Spoken;
                for c in &job.chunks {
                    if let Err(e) = worker_engine.speak(c) {
                        eprintln!("[{}] speech failed: {e}", job.msg_id);
                        outcome = Status::NoEngine;
                        break;
                    }
                }
                counter.queued.fetch_sub(1, Ordering::SeqCst);
                // Ignored deliberately: the caller may have stopped waiting.
                if let Some(done) = job.done {
                    let _ = done.send(outcome);
                }
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
        speak(&self.shared, &self.transport, text, priority, to, false).await
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

    /// The speech engine, for reading and changing its settings.
    pub fn engine(&self) -> &Arc<dyn SpeechEngine> {
        &self.shared.engine
    }

    /// This device's local label.
    pub fn name(&self) -> &str {
        &self.shared.name
    }

    /// This device's speaking policy.
    pub fn policy(&self) -> Response {
        policy_response(&self.shared)
    }

    /// Silence this device, or let it speak again.
    pub fn set_mute(&self, muted: bool) -> Response {
        set_mute(&self.shared, muted)
    }

    /// Set or clear the daily quiet window.
    pub fn set_quiet(
        &self,
        from: Option<String>,
        to: Option<String>,
        high_breaks_through: bool,
    ) -> Response {
        set_quiet(&self.shared, from, to, high_breaks_through)
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
        Request::Speak {
            text,
            priority,
            to,
            wait,
        } => speak(shared, transport, text, priority, to, wait).await,
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
        Request::Policy => policy_response(shared),
        Request::SetMute { muted } => set_mute(shared, muted),
        Request::SetQuiet {
            from,
            to,
            high_breaks_through,
        } => set_quiet(shared, from, to, high_breaks_through),
        Request::Status => status(shared),
    };
    write_frame(&mut s, &response).await
}

/// The name of the voice actually in use.
///
/// Resolved from the engine's current selection rather than whichever voice
/// happens to come first: with one voice those agreed, and with many the
/// status line and the picker disagreed on screen.
fn current_voice_name(engine: &Arc<dyn SpeechEngine>) -> String {
    let voices = engine.voices();
    // No fallback to the first voice. On a device offering a hundred of
    // them that names one we are not using, which is worse than admitting we
    // do not know yet — the engine may still be starting.
    engine
        .current_voice()
        .and_then(|id| voices.iter().find(|v| v.id == id))
        .map_or_else(
            || {
                if voices.is_empty() {
                    "starting…".to_string()
                } else {
                    "default voice".to_string()
                }
            },
            |v| v.name.clone(),
        )
}

/// This node's health.
fn status(shared: &Arc<Shared>) -> Response {
    let policy = *shared.policy.lock().expect("policy lock");
    Response::Status {
        device_id: shared.identity.id().to_string(),
        key_store: shared.identity.location().to_string(),
        engine: current_voice_name(&shared.engine),
        fallback: shared.engine.tier() == voicecast_engine::Tier::Fallback,
        queued: shared.queued.load(Ordering::SeqCst),
        muted: policy.muted,
        quiet: policy.quiet.map(|q| {
            format!(
                "{}-{}{}",
                policy::format_time(q.from),
                policy::format_time(q.to),
                if q.high_breaks_through {
                    " (high breaks through)"
                } else {
                    ""
                }
            )
        }),
    }
}

/// Speak here, on a named peer, or on everything in the space.
///
/// Always answers with a per-target report. Without `wait` those say `queued`
/// — accepted, not yet spoken — which is the honest thing to claim when the
/// sound has not happened yet.
async fn speak(
    shared: &Arc<Shared>,
    transport: &Arc<Transport>,
    text: String,
    priority: Priority,
    to: Option<String>,
    wait: bool,
) -> Response {
    let chunks = chunk(&text);
    if chunks.is_empty() {
        return Response::Error {
            message: "nothing to say".into(),
        };
    }
    let msg_id = new_msg_id();

    let targets = match resolve(shared, to.as_deref().unwrap_or("here")).await {
        Ok(targets) => targets,
        Err(message) => return Response::Error { message },
    };

    let targets = deliver(shared, transport, &msg_id, &chunks, priority, wait, targets).await;
    Response::Report { msg_id, targets }
}

/// One resolved destination for a message.
#[derive(Clone, PartialEq, Eq)]
enum Target {
    /// This device.
    Here,
    /// A peer, by label and public key.
    Peer { name: String, id: String },
}

/// Turn a selector into the devices it names.
///
/// Accepts a comma-separated list, so `--to desk,pixel` reaches both. Each
/// element is a device label, `all`, or `here`; duplicates collapse, because
/// `--to all,pixel` should not make the phone say it twice.
///
/// An unknown name is an error naming every name that *is* known, rather than
/// a message that quietly reaches fewer devices than asked for. Partial
/// delivery from a typo is the failure worth preventing here: it looks like
/// it worked.
async fn resolve(shared: &Arc<Shared>, selector: &str) -> Result<Vec<Target>, String> {
    let me = shared.identity.id().to_string();
    let members: Vec<(String, String)> = {
        let roster = shared.roster.lock().await;
        roster
            .members()
            .filter(|m| m.endpoint_id != me)
            .map(|m| (m.name.clone(), m.endpoint_id.clone()))
            .collect()
    };

    let mut targets: Vec<Target> = Vec::new();
    let mut unknown: Vec<String> = Vec::new();
    let push = |t: Target, targets: &mut Vec<Target>| {
        if !targets.contains(&t) {
            targets.push(t);
        }
    };

    for raw in selector.split(',') {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        if name.eq_ignore_ascii_case("all") {
            push(Target::Here, &mut targets);
            for (n, id) in &members {
                push(
                    Target::Peer {
                        name: n.clone(),
                        id: id.clone(),
                    },
                    &mut targets,
                );
            }
        } else if name.eq_ignore_ascii_case("here") || name == shared.name {
            push(Target::Here, &mut targets);
        } else if let Some((n, id)) = members.iter().find(|(n, _)| n == name) {
            push(
                Target::Peer {
                    name: n.clone(),
                    id: id.clone(),
                },
                &mut targets,
            );
        } else {
            unknown.push(name.to_string());
        }
    }

    if !unknown.is_empty() {
        let mut known: Vec<&str> = members.iter().map(|(n, _)| n.as_str()).collect();
        known.push(&shared.name);
        known.sort_unstable();
        return Err(format!(
            "no device named {} in this space. Known: {}",
            unknown
                .iter()
                .map(|u| format!("'{u}'"))
                .collect::<Vec<_>>()
                .join(", "),
            known.join(", "),
        ));
    }
    if targets.is_empty() {
        return Err(format!("'{selector}' names no devices"));
    }
    Ok(targets)
}

/// Speak on every resolved target at once.
///
/// Concurrent, not sequential: with `--wait` a serial loop would not even
/// *send* to the second device until the first had finished speaking, so
/// three devices meant three messages one after another when the caller
/// asked for one message on three devices.
///
/// Results come back in the order the targets were resolved, not the order
/// they happened to finish, so repeated runs read the same way.
#[allow(clippy::too_many_arguments)]
async fn deliver(
    shared: &Arc<Shared>,
    transport: &Arc<Transport>,
    msg_id: &str,
    chunks: &[String],
    priority: Priority,
    wait: bool,
    targets: Vec<Target>,
) -> Vec<TargetResult> {
    let mut set = tokio::task::JoinSet::new();
    for (index, target) in targets.into_iter().enumerate() {
        let shared = Arc::clone(shared);
        let transport = Arc::clone(transport);
        let msg_id = msg_id.to_string();
        let chunks = chunks.to_vec();
        set.spawn(async move {
            let result = match target {
                Target::Here => {
                    let (status, took_ms, detail) =
                        speak_here(&shared, &msg_id, chunks, priority, wait).await;
                    TargetResult {
                        device: shared.name.clone(),
                        status,
                        took_ms,
                        detail,
                    }
                }
                Target::Peer { name, id } => {
                    to_peer(
                        &shared, &transport, &name, &id, &msg_id, &chunks, priority, wait,
                    )
                    .await
                }
            };
            (index, result)
        });
    }

    let mut done: Vec<(usize, TargetResult)> = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(pair) => done.push(pair),
            // A panicked task must not silently shrink the report.
            Err(e) => eprintln!("delivery task failed: {e}"),
        }
    }
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

/// Send to one peer and turn the outcome into a result row.
#[allow(clippy::too_many_arguments)]
async fn to_peer(
    shared: &Arc<Shared>,
    transport: &Arc<Transport>,
    name: &str,
    peer_id: &str,
    msg_id: &str,
    chunks: &[String],
    priority: Priority,
    wait: bool,
) -> TargetResult {
    let started = std::time::Instant::now();
    match send_to_peer(shared, transport, peer_id, msg_id, chunks, priority, wait).await {
        Ok(status) => TargetResult {
            device: name.to_string(),
            took_ms: wait.then(|| started.elapsed().as_millis() as u64),
            status,
            detail: None,
        },
        Err(e) => TargetResult {
            device: name.to_string(),
            status: Status::Unreachable,
            took_ms: None,
            detail: Some(format!("{e:#}")),
        },
    }
}

/// Queue chunks for the local engine.
fn enqueue(shared: &Arc<Shared>, msg_id: String, chunks: Vec<String>, p: Priority) -> Response {
    enqueue_inner(shared, msg_id, chunks, p, None)
}

/// Queue chunks, optionally with a channel signalled when speaking ends.
///
/// The channel is optional because the common case is fire-and-forget: an
/// agent firing notifications should not pay for machinery it is not using.
fn enqueue_inner(
    shared: &Arc<Shared>,
    msg_id: String,
    chunks: Vec<String>,
    p: Priority,
    done: Option<tokio::sync::oneshot::Sender<Status>>,
) -> Response {
    // Policy comes first. A muted device has no business reporting a broken
    // engine: the sender needs to hear the reason that actually applies, and
    // "muted" is both truer and more actionable than "no engine".
    let refusal = {
        let policy = shared.policy.lock().expect("policy lock");
        policy.verdict(
            p,
            policy::local_minute(),
            shared.queued.load(Ordering::SeqCst),
        )
    };
    if let Some(status) = refusal {
        return Response::Finished { status };
    }
    // Refuse before accepting, so the sender is told rather than the failure
    // being buried in this device's log.
    if let Err(e) = shared.engine.ready() {
        return Response::Error {
            message: e.to_string(),
        };
    }
    if p == Priority::High {
        shared.engine.stop();
    }
    shared.queued.fetch_add(1, Ordering::SeqCst);
    match shared.tx.send(Job {
        msg_id: msg_id.clone(),
        chunks,
        done,
    }) {
        Ok(()) => Response::Accepted { msg_id },
        Err(_) => Response::Error {
            message: "speech worker has stopped".into(),
        },
    }
}

/// Speak here, reporting what actually happened.
///
/// Waits for the worker when asked, so a caller learns "spoken" rather than
/// merely "queued". Bounded, because a device speaking a long document should
/// not hold a caller open indefinitely.
async fn speak_here(
    shared: &Arc<Shared>,
    msg_id: &str,
    chunks: Vec<String>,
    p: Priority,
    wait: bool,
) -> (Status, Option<u64>, Option<String>) {
    if !wait {
        return match enqueue(shared, msg_id.to_string(), chunks, p) {
            Response::Accepted { .. } => (Status::Queued, None, None),
            Response::Error { message } => (Status::NoEngine, None, Some(message)),
            // A policy refusal — muted, quiet hours, or dropped chatter. It
            // is a terminal answer, so it travels back to the sender as is.
            Response::Finished { status } => {
                let why = refusal_detail(&status);
                (status, None, why)
            }
            _ => (Status::Dropped, None, None),
        };
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    match enqueue_inner(shared, msg_id.to_string(), chunks, p, Some(tx)) {
        Response::Accepted { .. } => {}
        Response::Error { message } => return (Status::NoEngine, None, Some(message)),
        Response::Finished { status } => {
            let why = refusal_detail(&status);
            return (status, None, why);
        }
        _ => return (Status::Dropped, None, None),
    }

    let started = std::time::Instant::now();
    match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
        Ok(Ok(status)) => (status, Some(started.elapsed().as_millis() as u64), None),
        // The worker went away, or we gave up first. Either way it was
        // accepted, so say that rather than claim a failure.
        _ => (Status::Queued, None, Some("still speaking".into())),
    }
}

/// This device's policy, in the shape the CLI and the app read.
fn policy_response(shared: &Arc<Shared>) -> Response {
    let p = *shared.policy.lock().expect("policy lock");
    Response::Policy {
        muted: p.muted,
        quiet_from: p.quiet.map(|q| policy::format_time(q.from)),
        quiet_to: p.quiet.map(|q| policy::format_time(q.to)),
        high_breaks_through: p.quiet.is_some_and(|q| q.high_breaks_through),
    }
}

/// Silence this device, or let it speak again.
///
/// Muting stops what is being said now as well as what comes next. Letting
/// the current message run to the end would be a strange reading of "quiet".
fn set_mute(shared: &Arc<Shared>, muted: bool) -> Response {
    {
        let mut p = shared.policy.lock().expect("policy lock");
        p.muted = muted;
        if let Err(e) = policy::save(&p) {
            return Response::Error {
                message: format!("could not save the policy: {e}"),
            };
        }
    }
    if muted {
        shared.engine.stop();
    }
    policy_response(shared)
}

/// Set or clear the daily quiet window.
fn set_quiet(
    shared: &Arc<Shared>,
    from: Option<String>,
    to: Option<String>,
    high_breaks_through: bool,
) -> Response {
    let quiet = match (from, to) {
        (Some(f), Some(t)) => {
            let (Some(from), Some(to)) = (policy::parse_time(&f), policy::parse_time(&t)) else {
                return Response::Error {
                    message: format!("times must look like 22:00, got '{f}' and '{t}'"),
                };
            };
            Some(crate::QuietHours {
                from,
                to,
                high_breaks_through,
            })
        }
        // Either end missing clears the window. Half a window has no meaning,
        // and guessing the other end would silence a device by accident.
        _ => None,
    };
    let mut p = shared.policy.lock().expect("policy lock");
    p.quiet = quiet;
    if let Err(e) = policy::save(&p) {
        return Response::Error {
            message: format!("could not save the policy: {e}"),
        };
    }
    drop(p);
    policy_response(shared)
}

/// Say in words why a device stayed silent.
///
/// The status alone reads as a failure at the sending end; naming the policy
/// makes it clear the message arrived and the device chose not to speak it.
fn refusal_detail(status: &Status) -> Option<String> {
    match status {
        Status::Muted => Some("device is muted".into()),
        Status::QuietHours => Some("quiet hours are active on that device".into()),
        Status::Dropped => Some("low priority, and the queue is already deep".into()),
        _ => None,
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
        // The peer no longer counts us as a member: it left, or removed us.
        // Either way the relationship is over, so drop it rather than keep
        // showing a device that will never answer. This is what makes the
        // system self-heal when a departure announcement goes missing — the
        // next check-in settles it.
        //
        // Safe by construction: a peer can only ever make us forget *itself*,
        // which it could already do by leaving.
        PeerMessage::JoinRefused { .. } => {
            let peer = conn.remote_id().to_string();
            let mut roster = shared.roster.lock().await;
            if roster.allows(&conn.remote_id()) {
                roster.revoke(&peer);
                roster.save(&shared.roster_path)?;
                eprintln!("{peer} no longer shares a space with us; removed");
            }
        }
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
    wait: bool,
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
            wait,
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
            PeerMessage::SpeakBegin {
                msg_id,
                priority,
                wait,
            } => {
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
                // Waiting here is what lets a sender learn "spoken" rather
                // than "queued": only this device knows when the sound ended.
                let (status, _took, _detail) =
                    speak_here(shared, &msg_id, chunks, priority, wait).await;
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
                // Merge before replying. A departing peer sends its tombstone
                // and closes without waiting, so writing first meant the reply
                // failed and `?` returned before the news was ever acted on —
                // which is why a device that left stayed listed here.
                merge_from_peer(shared, members, revoked).await?;

                let mine = {
                    let roster = shared.roster.lock().await;
                    PeerMessage::RosterSync {
                        members: roster.members().cloned().collect(),
                        revoked: roster.tombstones(),
                    }
                };
                // Best-effort: the peer may already be gone, and that is fine.
                let _ = write_msg(&mut send, &mine).await;
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
