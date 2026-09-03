//! The node: accepts local requests, speaks them, and relays them to peers.
//!
//! Two loops run side by side — a local IPC socket for the CLI, and an iroh
//! endpoint for other devices. The CLI is a thin client that hands over text
//! and exits; everything durable lives here.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use interprocess::local_socket::{
    GenericNamespaced,
    ListenerOptions,
    ToNsName,
    tokio::{Listener as TokioListener, Stream},
    // Anonymous: `connect` and `accept` are wanted, but the trait names
    // themselves would collide with the concrete types above.
    traits::tokio::{Listener, Stream as _},
};
use tokio::sync::Mutex;
use voicecast_engine::SpeechEngine;
use voicecast_proto::{
    Control, DeviceInfo, Member, PeerMessage, Priority, Request, Response, Status, TargetResult,
};
use voicecast_text::chunk;

use crate::history::{Entry, History};
use crate::ipc::{read_frame, socket_name, write_frame};
use crate::policy::{self, Policies};
use crate::queue::{Job, Speaker, words_in};
use crate::spaces::Spaces;
use crate::transport::{read_msg, write_msg};
use crate::{Identity, Roster, Ticket, Transport};

/// Bind the local socket, reclaiming one a dead node left behind.
///
/// On Linux the name lives in the abstract namespace and vanishes with the
/// process that held it, so `AddrInUse` can only mean a node is running. Not
/// so anywhere else: the name is a file, and a crash or a `kill` leaves it
/// on disk. Every node started afterwards then fails to bind, while every
/// CLI call gets `connection refused` from a socket that looks perfectly
/// healthy — a state nothing recovers from without deleting a file by hand.
///
/// A live node is told from a dead one's leftovers by connecting, not by
/// inspecting the file: only a refused connection proves nothing is
/// listening. Overwriting on `AddrInUse` alone would let a second node
/// displace a running one, which is the very thing the error exists to
/// prevent.
async fn bind_ipc(socket: &str) -> Result<TokioListener> {
    let name = || socket.to_string().to_ns_name::<GenericNamespaced>();

    let refused = match ListenerOptions::new().name(name()?).create_tokio() {
        Ok(listener) => return Ok(listener),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            // Nothing answering means the name outlived its node.
            Stream::connect(name()?).await.is_err()
        }
        Err(e) => {
            // The `sun_path` limit is the one that actually bites — 104 bytes
            // on macOS — and the platform's own message names the limit but
            // not the string that exceeded it. That string is not the one the
            // caller set, because a prefix is added, so without this they
            // cannot tell how far over they are. Issue #43.
            return Err(e).context(format!(
                "binding the local socket named {socket:?} ({} bytes, before \
                 the prefix this platform adds)",
                socket.len()
            ));
        }
    };

    if !refused {
        anyhow::bail!("another node is already running on {socket}");
    }

    eprintln!("removing the socket a previous node left at {socket}");
    ListenerOptions::new()
        .name(name()?)
        .try_overwrite(true)
        .create_tokio()
        .context("reclaiming the local socket")
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
    /// This device's own label.
    ///
    /// Behind a lock because it can change while the node runs, and because
    /// every roster sync writes it back into our own entry. Holding it as a
    /// plain `String` meant `rename` updated the file and one roster while
    /// this copy stayed stale — and the next sync, within the minute, wrote
    /// the stale copy back over the new name with a fresher `renamed_at`, so
    /// the old name then won on every peer too (#62).
    name: std::sync::RwLock<String>,
    /// Every space this device belongs to.
    ///
    /// Operations that predate several spaces act on the default one, which
    /// is what kept the rest of the node unchanged when spaces arrived.
    spaces: Mutex<Spaces>,
    spaces_path: PathBuf,
    /// The invite currently outstanding, if any. One at a time: an invite is
    /// a deliberate act, and allowing several open at once would widen the
    /// window in which a leaked ticket still works.
    pending: Mutex<Option<Ticket>>,
    /// The speaking thread and everything waiting for it.
    speaker: Speaker,
    /// What this device was asked to say, spoken or not.
    ///
    /// A plain mutex: it is written from the speech thread, which is not
    /// async, and every operation is over in microseconds.
    history: std::sync::Mutex<History>,
    history_path: PathBuf,
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
    policy: std::sync::Mutex<Policies>,
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
        // Apply a remembered voice before anything can be spoken with the
        // wrong one.
        if let Some((voice, rate)) = crate::load_voice_settings() {
            let _ = engine.set_voice(&voice);
            let _ = engine.set_rate(rate);
        }

        let spaces_path = Spaces::default_path().context("locating spaces")?;
        let legacy = Roster::default_path().context("locating roster")?;
        // Whether this device has been through the migration yet. Persisted
        // eagerly below so it happens exactly once, rather than being redone
        // from a roster file that nothing writes to any more.
        let migrating = !spaces_path.exists();
        let mut spaces = Spaces::load(&spaces_path, &legacy).context("loading spaces")?;
        // A device is always a member of its own space, even before anyone
        // else joins — otherwise it could not speak to itself.
        if spaces.ids().is_empty() {
            spaces.insert(Roster::found(identity.secret(), &name), "main");
            spaces.save(&spaces_path).context("saving spaces")?;
        } else if migrating {
            spaces.save(&spaces_path).context("saving spaces")?;
        } else if spaces
            .current_mut()
            .rename(&identity.id().to_string(), &name)
            || spaces
                .current_mut()
                .stamp_own_label(&identity.id().to_string())
        {
            // Two reasons to rewrite: the label changed, or it predates the
            // `renamed_at` stamp. Rosters written before that field existed
            // deserialize it as zero, and a merge comparing 0 > 0 keeps the
            // stale copy forever — so an unstamped entry is stamped once, on
            // the device that owns it.
            spaces.save(&spaces_path).context("saving spaces")?;
        }

        let history_path = History::default_path()
            .ok_or_else(|| anyhow::anyhow!("no config directory for the history"))?;
        let history = std::sync::Mutex::new(History::load(&history_path));

        // The queue reports every outcome here, because most messages have
        // nobody waiting on them and would otherwise be recorded as "queued"
        // forever.
        //
        // Weak on purpose: the state owns the queue, which owns this
        // callback, so holding it strongly would be a cycle that never frees.
        let recorder: Arc<std::sync::Mutex<std::sync::Weak<Shared>>> =
            Arc::new(std::sync::Mutex::new(std::sync::Weak::new()));
        let notify = Arc::clone(&recorder);
        let gate = Arc::clone(&recorder);
        let speaker = Speaker::new(
            Arc::clone(&engine),
            Arc::new(move |msg_id, ended: crate::queue::Ended| {
                if let Some(shared) = notify.lock().expect("recorder lock").upgrade() {
                    remember_outcome(&shared, msg_id, ended.status);
                }
            }),
            // Policy again, at the moment of speaking. Checking only at
            // submit let a message accepted at 21:59 be spoken at 22:10 from
            // behind a long document, inside quiet hours (#77). Weak for the
            // same reason as the recorder above.
            Arc::new(move |space: Option<&str>| {
                let shared = gate.lock().expect("recorder lock").upgrade()?;
                let policy = shared.policy.lock().expect("policy lock");
                // Depth zero: the queue-depth rule drops a low-priority
                // message that would arrive too late to matter, and it has
                // already been applied once. Applying it again here, with
                // this message about to be spoken rather than waiting behind
                // anything, would be a different question with the same name.
                policy.verdict(space, Priority::Normal, policy::local_minute(), 0)
            }),
        );
        let shared = Arc::new(Shared {
            engine,
            identity,
            name: std::sync::RwLock::new(name),
            spaces: Mutex::new(spaces),
            spaces_path,
            // An invite outstanding when the app last stopped is still
            // valid if it has not expired.
            pending: Mutex::new(Ticket::recall()),
            speaker,
            history,
            history_path,
            last_seen: Mutex::new(std::collections::HashMap::new()),
            policy: std::sync::Mutex::new(policy::load()),
            on_show: Mutex::new(None),
            on_quit: Mutex::new(None),
        });

        // Closes the loop: the queue's callback needs the state that owns it.
        *recorder.lock().expect("recorder lock") = Arc::downgrade(&shared);

        Ok(Self {
            shared,
            transport: Arc::new(transport),
        })
    }

    /// Stop the speaking thread when the node goes away.
    ///
    /// The thread parks on a condvar rather than a channel, so nothing ends
    /// it implicitly — without this a dropped node leaves a thread waiting
    /// forever, which tests creating several nodes would accumulate.
    pub fn shutdown(&self) {
        self.shared.speaker.shutdown();
    }

    /// Stop the node entirely, including its endpoint on the network.
    ///
    /// [`shutdown`] ends the speaking thread and leaves the transport online,
    /// which is right when the process is going away anyway. It is wrong when
    /// the node has failed to start and the process continues: the endpoint
    /// stays bound under this device's secret key, so peers resolve the
    /// identity to an address that answers and then does nothing. The app hit
    /// exactly that — issue #72.
    ///
    /// [`shutdown`]: Node::shutdown
    pub async fn close(&self) {
        self.shutdown();
        self.transport.close().await;
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
        speak(
            &self.shared,
            &self.transport,
            SpeakRequest {
                text,
                priority,
                to,
                wait: false,
                voice: None,
                timeout_secs: None,
            },
        )
        .await
    }

    /// Mint an invite for another device.
    pub async fn invite(&self, space: Option<&str>) -> Response {
        invite(&self.shared, space).await
    }

    /// Join a space using someone else's invite.
    ///
    /// `label` is what this device will call it; `None` takes the inviter's
    /// name for it.
    pub async fn join(&self, ticket: &str, label: Option<String>) -> Response {
        join(&self.shared, &self.transport, ticket, label).await
    }

    /// Read an invite without acting on it.
    pub fn preview(&self, ticket: &str) -> Response {
        preview(ticket)
    }

    /// Change this device's label.
    pub async fn rename(&self, name: &str) -> Response {
        rename(&self.shared, name).await
    }

    /// Remove another device from this space.
    pub async fn revoke(&self, name: &str, space: Option<&str>) -> Response {
        revoke(&self.shared, name, space).await
    }

    /// Leave the space, keeping this device's identity.
    pub async fn leave(&self, space: Option<&str>) -> Response {
        leave(&self.shared, &self.transport, space).await
    }

    /// Replace this space with a fresh one, locking every other device out.
    pub async fn rotate(&self, space: Option<&str>) -> Response {
        rotate(&self.shared, space).await
    }

    /// Recent messages this device was asked to speak.
    pub fn history(&self, limit: Option<usize>) -> Response {
        history_response(&self.shared, limit)
    }

    /// Speak a message from the history again.
    pub fn replay(&self, msg_id: &str) -> Response {
        replay(&self.shared, msg_id)
    }

    /// Forget the history.
    pub fn clear_history(&self) -> Response {
        let mut history = self.shared.history.lock().expect("history lock");
        history.clear();
        let _ = history.save(&self.shared.history_path);
        Response::Done
    }

    /// The spaces this device belongs to.
    pub async fn spaces(&self) -> Response {
        list_spaces(&self.shared).await
    }

    /// Found a new space from this device, and make it the default.
    pub async fn new_space(&self, label: &str) -> Response {
        new_space(&self.shared, label).await
    }

    /// Drop one space, keeping the others.
    pub async fn leave_space(&self, label: &str) -> Response {
        leave_space(&self.shared, label).await
    }

    /// Choose which space bare device names resolve in.
    pub async fn default_space(&self, label: &str) -> Response {
        default_space(&self.shared, label).await
    }

    /// Rename a space locally.
    pub async fn rename_space(&self, label: &str, to: &str) -> Response {
        rename_space(&self.shared, label, to).await
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

    /// This device's local label, as it stands now.
    ///
    /// Owned rather than borrowed because it can change while the node runs.
    pub fn name(&self) -> String {
        my_name(&self.shared)
    }

    /// This device's speaking policy, and any per-space overrides.
    pub async fn policy(&self) -> Response {
        policy_response(&self.shared).await
    }

    /// Silence this device, or one space on it, or let it speak again.
    ///
    /// `space` is `None` for the whole device — not for the default space.
    pub async fn set_mute(&self, muted: bool, space: Option<&str>) -> Response {
        set_mute(&self.shared, muted, space).await
    }

    /// Set or clear a daily quiet window, device-wide or for one space.
    pub async fn set_quiet(
        &self,
        from: Option<String>,
        to: Option<String>,
        high_breaks_through: bool,
        space: Option<&str>,
    ) -> Response {
        set_quiet(&self.shared, from, to, high_breaks_through, space).await
    }

    /// Stop whatever is being spoken here, and drop the queue behind it.
    pub fn stop(&self) {
        self.shared.speaker.clear();
    }

    /// Hold what is being spoken here, keeping it to resume.
    pub fn pause(&self) {
        self.shared.speaker.pause();
    }

    /// Start speaking here again after a pause.
    pub fn unpause(&self) {
        self.shared.speaker.unpause();
    }

    /// Abandon the current message here and move to the next.
    pub fn skip(&self) {
        self.shared.speaker.skip();
    }

    /// What is being spoken here, and what is waiting.
    pub fn queue_state(&self) -> Response {
        let snap = self.shared.speaker.snapshot();
        Response::Queue {
            speaking: snap.speaking,
            pending: snap.pending,
            paused: snap.paused,
        }
    }

    /// One message from the history, for showing what is playing.
    pub fn message(&self, msg_id: &str) -> Option<Entry> {
        self.shared
            .history
            .lock()
            .expect("history lock")
            .get(msg_id)
            .cloned()
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
                // Every space, not just the default: a device that only
                // appears in the second one would otherwise never be checked
                // on and would show as stale forever.
                let peers: Vec<(String, String)> = {
                    let spaces = node.shared.spaces.lock().await;
                    spaces
                        .ids()
                        .into_iter()
                        .flat_map(|id| {
                            let roster = spaces.get(&id).expect("id came from this map");
                            roster
                                .members()
                                .filter(|m| m.endpoint_id != me)
                                .map(|m| (id.clone(), m.endpoint_id.clone()))
                                .collect::<Vec<_>>()
                        })
                        .collect()
                };
                for (space, peer) in peers {
                    // Failure is the useful signal here: the peer simply stays
                    // stale until it can be reached again.
                    if let Ok(id) = peer.parse()
                        && let Ok(conn) = node.transport.connect(id).await
                    {
                        let _ = sync_roster(&node.shared, &conn, &space).await;
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
        let listener = bind_ipc(&socket_name()).await?;

        let socket = socket_name();
        eprintln!("listening as {}", self.transport.id());
        // Not "listening on <socket>": that reads as a path, and the previous
        // wording sent people to `ls` a file that is at a different place on
        // macOS and does not exist at all on Linux. Issue #43.
        eprintln!("socket name: {socket:?} (a name, not a path)");
        if crate::ipc::path_shaped(&socket) {
            eprintln!(
                "  note: that looks like a path, and it is used as a name — the \
                 platform decides where it actually lives, so looking for a file \
                 at that exact location will not find one"
            );
        }

        let ipc = {
            let shared = Arc::clone(&self.shared);
            let transport = Arc::clone(&self.transport);
            async move {
                loop {
                    let stream = listener
                        .accept()
                        .await
                        .context("accepting CLI connection")?;
                    let shared = Arc::clone(&shared);
                    let transport = Arc::clone(&transport);
                    tokio::spawn(async move {
                        if let Err(e) = handle_cli(&shared, &transport, stream).await {
                            eprintln!("cli: {e:#}");
                        }
                    });
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
            voice,
            timeout_secs,
        } => {
            speak(
                shared,
                transport,
                SpeakRequest {
                    text,
                    priority,
                    to,
                    wait,
                    voice,
                    timeout_secs,
                },
            )
            .await
        }
        Request::Resolve { to } => match resolve(shared, to.as_deref().unwrap_or("here")).await {
            Ok(targets) => Response::Targets {
                devices: targets
                    .into_iter()
                    .map(|t| match t {
                        Target::Here { .. } => my_name(shared),
                        Target::Peer { name, .. } => name,
                    })
                    .collect(),
            },
            Err(message) => Response::no_target(message),
        },
        Request::Stop { to, msg_id } => {
            control(shared, transport, to, Control::Stop { msg_id }).await
        }
        Request::Skip { to } => control(shared, transport, to, Control::Skip).await,
        Request::Pause { to } => control(shared, transport, to, Control::Pause).await,
        Request::Resume { to } => control(shared, transport, to, Control::Resume).await,
        Request::Queue => {
            let snap = shared.speaker.snapshot();
            Response::Queue {
                speaking: snap.speaking,
                pending: snap.pending,
                paused: snap.paused,
            }
        }
        Request::Invite { space } => invite(shared, space.as_deref()).await,
        Request::Join { ticket, label } => join(shared, transport, &ticket, label).await,
        Request::Preview { ticket } => preview(&ticket),
        Request::Devices => devices(shared).await,
        Request::Rename { name } => rename(shared, &name).await,
        Request::Revoke { name, space } => revoke(shared, &name, space.as_deref()).await,
        Request::Leave { space } => leave(shared, transport, space.as_deref()).await,
        Request::Rotate { space } => rotate(shared, space.as_deref()).await,
        Request::Spaces => list_spaces(shared).await,
        Request::NewSpace { label } => new_space(shared, &label).await,
        Request::LeaveSpace { label } => leave_space(shared, &label).await,
        Request::DefaultSpace { label } => default_space(shared, &label).await,
        Request::RenameSpace { label, to } => rename_space(shared, &label, &to).await,
        Request::Show => match shared.on_show.lock().await.as_ref() {
            Some(hook) => {
                hook();
                Response::Done
            }
            None => Response::error("this node has no window"),
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
        Request::History { limit } => history_response(shared, limit),
        Request::Replay { msg_id } => replay(shared, &msg_id),
        Request::ClearHistory => {
            let mut history = shared.history.lock().expect("history lock");
            history.clear();
            let _ = history.save(&shared.history_path);
            Response::Done
        }
        Request::Policy => policy_response(shared).await,
        Request::SetMute { muted, space } => set_mute(shared, muted, space.as_deref()).await,
        Request::SetQuiet {
            from,
            to,
            high_breaks_through,
            space,
        } => set_quiet(shared, from, to, high_breaks_through, space.as_deref()).await,
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
                if !voices.is_empty() {
                    "default voice".to_string()
                } else if engine.ready().is_err() {
                    // An engine that cannot speak is not starting, and saying
                    // so told the reader to wait — the one thing that could
                    // not help. The reason travels beside this.
                    "unavailable".to_string()
                } else {
                    "starting…".to_string()
                }
            },
            |v| v.name.clone(),
        )
}

/// This node's health.
fn status(shared: &Arc<Shared>) -> Response {
    // The device policy alone. Per-space overrides do not belong on a health
    // line: it would have to report several answers to "is this muted", and
    // the app's settings screen is where that question is actually asked.
    let policy = shared.policy.lock().expect("policy lock").device;
    Response::Status {
        device_id: shared.identity.id().to_string(),
        key_store: shared.identity.location().to_string(),
        engine: current_voice_name(&shared.engine),
        // Carried rather than inferred. The node has held this all along and
        // only the sender ever saw it.
        engine_reason: shared.engine.ready().err().map(|e| e.reason().to_string()),
        fallback: shared.engine.tier() == voicecast_engine::Tier::Fallback,
        queued: shared.speaker.depth(),
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
async fn speak(shared: &Arc<Shared>, transport: &Arc<Transport>, ask: SpeakRequest) -> Response {
    let SpeakRequest {
        text,
        priority,
        to,
        wait,
        voice,
        timeout_secs,
    } = ask;
    let chunks = chunk(&text);
    if chunks.is_empty() {
        return Response::error("nothing to say");
    }

    let targets = match resolve(shared, to.as_deref().unwrap_or("here")).await {
        Ok(targets) => targets,
        // Every failure `resolve` reports is the selector matching nothing,
        // which is a well-formed command naming a device that is not here —
        // not the same as a malformed one, and promised its own exit code
        // since `docs/cli.md` was written (#66).
        Err(message) => return Response::no_target(message),
    };

    let outgoing = Outgoing {
        msg_id: new_msg_id(),
        chunks,
        priority,
        wait,
        voice,
        timeout: timeout_secs.map(std::time::Duration::from_secs),
        // Typed here, so this device is the origin, and in no space.
        from: None,
        space: None,
    };
    let msg_id = outgoing.msg_id.clone();
    let targets = deliver(shared, transport, &outgoing, targets).await;
    Response::Report { msg_id, targets }
}

/// Words per minute a device is assumed to speak at when estimating a wait.
///
/// Deliberately far slower than anything here actually manages: Piper's
/// `en_US-lessac-medium` measures 197–231 wpm on an M4, and espeak-ng
/// defaults to 175. Being wrong in this direction is nearly free, because the
/// estimate is an *upper bound on waiting* rather than a delay — the wait ends
/// the moment speaking does. Being wrong in the other direction is the bug
/// this replaced.
const ASSUMED_WPM: f32 = 100.0;

/// Added to every estimate, for starting the synthesiser and scheduling.
const STARTUP_ALLOWANCE: std::time::Duration = std::time::Duration::from_secs(15);

/// The shortest wait, so a two-word message still tolerates a slow start.
const MIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The longest wait, however much text was sent.
///
/// A stuck engine would otherwise hold a caller for as long as the text
/// implies, which for a whole document read from a file is hours. Anyone
/// genuinely speaking for longer than this can say so with `--timeout`.
const MAX_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// How long to allow for `words` to be spoken at `rate`.
///
/// `rate` is the engine's own multiplier, where 1.0 is its normal pace, so a
/// device set to speak at half speed waits twice as long. Clamped low so a
/// nonsense rate cannot divide by zero.
///
/// Estimated rather than fixed because a constant is wrong at exactly one
/// length. The previous 120 seconds was fine until someone sent 569 words,
/// which is around 148 seconds of audio — the device spoke all of it and the
/// caller was told it had not finished.
fn estimated_wait(words: usize, rate: f32) -> std::time::Duration {
    let per_minute = ASSUMED_WPM * rate.max(0.1);
    let seconds = (words as f32 / per_minute) * 60.0;
    std::time::Duration::from_secs_f32(seconds)
        .saturating_add(STARTUP_ALLOWANCE)
        .clamp(MIN_TIMEOUT, MAX_TIMEOUT)
}

/// What a caller asked to have spoken.
///
/// The fields of [`Request::Speak`], gathered so the layers below take one
/// value rather than a parameter list that grew every time the CLI did.
struct SpeakRequest {
    text: String,
    priority: Priority,
    to: Option<String>,
    wait: bool,
    voice: Option<String>,
    timeout_secs: Option<u64>,
}

/// A message on its way somewhere, minus the destination.
///
/// Introduced because every layer between `speak` and the wire took the same
/// six values and passed them straight down; adding a seventh meant editing
/// five signatures and their `too_many_arguments` waivers.
#[derive(Clone)]
struct Outgoing {
    msg_id: String,
    chunks: Vec<String>,
    priority: Priority,
    /// Whether the caller is waiting for a terminal state.
    wait: bool,
    /// A voice the sender would like, if the receiver has it.
    voice: Option<String>,
    /// How long to wait before answering "still speaking".
    ///
    /// `None` leaves it to the device that will do the speaking, which is the
    /// only one that knows its own engine, its own rate, and what is already
    /// queued ahead of this. `Some` is the caller saying `--timeout`, which
    /// always wins over any estimate.
    timeout: Option<std::time::Duration>,
    /// The device it came from, for the history. `None` means this one.
    from: Option<String>,
    /// The space it arrived in, which selects the receiver's policy.
    ///
    /// `None` for text typed or piped in here: speech this device originates
    /// is not *in* a space, so only the device policy governs it. Muting one
    /// space must not silence the local agent, and muting the device must.
    space: Option<String>,
}

/// One resolved destination for a message.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Target {
    /// This device.
    ///
    /// `shadowed` holds the peers that answer to the same name and were not
    /// sent to. This device's own label wins outright, which is defensible —
    /// but it used to win *silently*, and a one-row report saying "spoken"
    /// reads as a clean send whether or not a second machine answered to the
    /// name. Carrying them here is what makes the ambiguity visible on first
    /// read rather than traceable afterwards. See #39.
    Here { shadowed: Vec<String> },
    /// A peer, by label, public key, and the space it was found in.
    Peer {
        name: String,
        id: String,
        space: String,
    },
}

/// Turn a selector into the devices it names.
///
/// Accepts a comma-separated list, so `--to desk,pixel` reaches both. Each
/// element is a device label, `all`, `here`, or any of those qualified by a
/// space as `work/laptop`. Duplicates collapse, because `--to all,pixel`
/// should not make the phone say it twice.
///
/// `all` is scoped to one space, and there is deliberately no selector
/// meaning "every device everywhere": crossing spaces has to be spelled out,
/// because a work message arriving on the family tablet is exactly what
/// separate spaces exist to prevent.
///
/// A bare name resolves in the default space when it exists there, and
/// otherwise anywhere it is unique. A name in two other spaces is an error
/// asking for it to be qualified, never a guess.
///
/// An unknown name is an error naming every name that *is* known. Partial
/// delivery from a typo is the failure worth preventing: reaching two devices
/// out of three looks like it worked.
async fn resolve(shared: &Arc<Shared>, selector: &str) -> Result<Vec<Target>, String> {
    let me = shared.identity.id().to_string();
    let spaces = shared.spaces.lock().await;
    let default_id = spaces.default_id().to_string();

    /// One member, and which space it was found in.
    struct Known {
        space: String,
        name: String,
        id: String,
    }

    let known: Vec<Known> = spaces
        .ids()
        .into_iter()
        .flat_map(|space| {
            let roster = spaces.get(&space).expect("id came from this map");
            roster
                .members()
                .filter(|m| m.endpoint_id != me)
                .map(|m| Known {
                    space: space.clone(),
                    name: m.name.clone(),
                    id: m.endpoint_id.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect();

    let mut targets: Vec<Target> = Vec::new();
    let push = push_target;
    let peer = |k: &Known| Target::Peer {
        name: k.name.clone(),
        id: k.id.clone(),
        space: k.space.clone(),
    };

    for raw in selector.split(',') {
        let element = raw.trim();
        if element.is_empty() {
            continue;
        }

        // A qualified name names its space explicitly; a bare one is looked
        // up below.
        let (scope, name) = match element.split_once('/') {
            Some((label, device)) => match spaces.by_label(label.trim()) {
                Some(id) => (Some(id), device.trim()),
                None => {
                    return Err(format!(
                        "no space called '{}'. Known: {}",
                        label.trim(),
                        space_labels(&spaces)
                    ));
                }
            },
            None => (None, element),
        };

        if name.eq_ignore_ascii_case("all") {
            let space = scope.unwrap_or_else(|| default_id.clone());
            // This device is a member of every space it holds, so `all`
            // includes it whichever space is named. Leaving it out of a
            // non-default space made `main/all` reach nothing at all on a
            // machine whose default had moved on.
            if spaces.get(&space).is_some() {
                push(
                    Target::Here {
                        shadowed: Vec::new(),
                    },
                    &mut targets,
                );
            }
            for k in known.iter().filter(|k| k.space == space) {
                push(peer(k), &mut targets);
            }
            continue;
        }

        if scope.is_none() && (name.eq_ignore_ascii_case("here") || name == my_name(shared)) {
            // `here` is unambiguous by construction. A bare name equal to our
            // own label is not: a peer may answer to it too, and this branch
            // takes the local device without consulting the roster at all.
            // That is the right choice — your own machine is what you meant —
            // but the peers it beat belong in the report, or the send reads as
            // clean to anyone who did not already suspect a clash.
            let shadowed = if name.eq_ignore_ascii_case("here") {
                Vec::new()
            } else {
                known
                    .iter()
                    .filter(|k| k.name == name)
                    .map(|k| k.id.clone())
                    .collect()
            };
            push(Target::Here { shadowed }, &mut targets);
            continue;
        }

        let matches: Vec<&Known> = match &scope {
            Some(space) => known
                .iter()
                .filter(|k| &k.space == space && k.name == name)
                .collect(),
            None => {
                // The default space wins outright, so setting a default
                // actually decides something.
                let in_default: Vec<&Known> = known
                    .iter()
                    .filter(|k| k.space == default_id && k.name == name)
                    .collect();
                if in_default.is_empty() {
                    known.iter().filter(|k| k.name == name).collect()
                } else {
                    in_default
                }
            }
        };

        match matches.as_slice() {
            [] => {
                return Err(format!(
                    "no device named '{name}' in this space. Known: {}",
                    device_names(&spaces, &my_name(shared))
                ));
            }
            [only] => push(peer(only), &mut targets),
            several => {
                let mut seen: Vec<&str> = several.iter().map(|k| k.space.as_str()).collect();
                seen.sort_unstable();
                let distinct = {
                    let mut d = seen.clone();
                    d.dedup();
                    d.len() == seen.len()
                };
                // Qualifying separates them only when the spaces differ. Two
                // devices sharing a name *inside* one space were told to
                // "Qualify it: work/twin  or  work/twin" — the same command
                // twice, and the one that had just failed. An agent following
                // that suggestion loops, and neither device is addressable by
                // any selector this resolver accepts. Issue #39.
                if !distinct {
                    let rows = several
                        .iter()
                        .map(|k| format!("\n  {}  in {}", short_id(&k.id), spaces.label(&k.space)))
                        .collect::<String>();
                    return Err(format!(
                        "more than one device is called '{name}' in the same space{rows}\n\
                         Qualifying by space cannot separate them. Rename one on the \
                         device itself: voicecast rename <new>"
                    ));
                }
                let where_ = seen
                    .iter()
                    .map(|id| spaces.label(id))
                    .collect::<Vec<_>>()
                    .join(", ");
                let hint = several
                    .iter()
                    .map(|k| format!("{}/{name}", spaces.label(&k.space)))
                    .collect::<Vec<_>>()
                    .join("  or  ");
                return Err(format!(
                    "'{name}' exists in {} spaces ({where_}). Qualify it: {hint}",
                    several.len()
                ));
            }
        }
    }

    if targets.is_empty() {
        return Err(format!("'{selector}' names no devices"));
    }
    Ok(targets)
}

/// Add a target unless it is already there.
///
/// `Here` collapses on identity, not on payload. Two selector elements can
/// both mean this device while disagreeing about what they shadowed —
/// `--to here,laptop` on a machine called `laptop` — and comparing the whole
/// value would push it twice and speak it twice, which is exactly what this
/// dedup exists to stop. The shadow lists merge instead, so whichever element
/// saw a clash still reports it.
fn push_target(t: Target, targets: &mut Vec<Target>) {
    if let Target::Here { shadowed } = t {
        match targets
            .iter_mut()
            .find(|e| matches!(e, Target::Here { .. }))
        {
            Some(Target::Here { shadowed: existing }) => {
                for id in shadowed {
                    if !existing.contains(&id) {
                        existing.push(id);
                    }
                }
            }
            _ => targets.push(Target::Here { shadowed }),
        }
        return;
    }
    if !targets.contains(&t) {
        targets.push(t);
    }
}

/// Say that other devices answer to the name this row was addressed by.
///
/// `None` when nothing was shadowed, so the common case adds no text at all.
fn also_answers_to(shadowed: &[String]) -> Option<String> {
    if shadowed.is_empty() {
        return None;
    }
    let ids = shadowed
        .iter()
        .map(|id| short_id(id))
        .collect::<Vec<_>>()
        .join(", ");
    let n = shadowed.len();
    let (device, answers, was) = if n == 1 {
        ("device", "answers", "was")
    } else {
        ("devices", "answer", "were")
    };
    Some(format!(
        "this device's own name was used; {n} other {device} also {answers} \
         to it and {was} not sent to: {ids}"
    ))
}

/// Every space label this device knows, for an error message.
fn space_labels(spaces: &Spaces) -> String {
    let mut labels: Vec<&str> = spaces.ids().iter().map(|id| spaces.label(id)).collect();
    labels.sort_unstable();
    labels.join(", ")
}

/// Every device name this device knows, qualified where a space is needed.
fn device_names(spaces: &Spaces, own: &str) -> String {
    let ids = spaces.ids();
    let several = ids.len() > 1;
    let mut names: Vec<String> = vec![own.to_string()];
    for id in &ids {
        let roster = spaces.get(id).expect("id came from this map");
        for m in roster.members() {
            names.push(if several {
                format!("{}/{}", spaces.label(id), m.name)
            } else {
                m.name.clone()
            });
        }
    }
    names.sort_unstable();
    names.dedup();
    names.join(", ")
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
async fn deliver(
    shared: &Arc<Shared>,
    transport: &Arc<Transport>,
    outgoing: &Outgoing,
    targets: Vec<Target>,
) -> Vec<TargetResult> {
    let mut set = tokio::task::JoinSet::new();
    for (index, target) in targets.into_iter().enumerate() {
        let shared = Arc::clone(shared);
        let transport = Arc::clone(transport);
        let outgoing = outgoing.clone();
        set.spawn(async move {
            let result = match target {
                Target::Here { shadowed } => {
                    let (status, took_ms, detail) = speak_here(&shared, &outgoing).await;
                    TargetResult {
                        device: my_name(&shared),
                        endpoint_id: shared.identity.id().to_string(),
                        status,
                        took_ms,
                        // Never overwrites a real explanation: a device that
                        // refused has something more useful to say than that
                        // the name was also taken elsewhere.
                        detail: detail.or_else(|| also_answers_to(&shadowed)),
                    }
                }
                Target::Peer { name, id, space } => {
                    to_peer(&shared, &transport, &name, &id, &space, &outgoing).await
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
async fn to_peer(
    shared: &Arc<Shared>,
    transport: &Arc<Transport>,
    name: &str,
    peer_id: &str,
    space: &str,
    outgoing: &Outgoing,
) -> TargetResult {
    let started = std::time::Instant::now();
    match send_to_peer(shared, transport, peer_id, space, outgoing).await {
        Ok((status, detail)) => TargetResult {
            device: name.to_string(),
            endpoint_id: peer_id.to_string(),
            took_ms: outgoing.wait.then(|| started.elapsed().as_millis() as u64),
            status,
            detail,
        },
        Err(e) => TargetResult {
            device: name.to_string(),
            endpoint_id: peer_id.to_string(),
            status: Status::Unreachable,
            took_ms: None,
            detail: Some(format!("{e:#}")),
        },
    }
}

/// Queue chunks for the local engine.
fn enqueue(
    shared: &Arc<Shared>,
    msg_id: String,
    chunks: Vec<String>,
    p: Priority,
    voice: Option<String>,
    space: Option<&str>,
) -> Response {
    enqueue_inner(shared, msg_id, chunks, p, voice, space, None)
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
    voice: Option<String>,
    space: Option<&str>,
    done: Option<tokio::sync::oneshot::Sender<crate::queue::Ended>>,
) -> Response {
    // Policy comes first. A muted device has no business reporting a broken
    // engine: the sender needs to hear the reason that actually applies, and
    // "muted" is both truer and more actionable than "no engine".
    //
    // The space is passed rather than looked up because only the caller knows
    // it: a peer message carries one, and text typed here belongs to none.
    let refusal = {
        let policy = shared.policy.lock().expect("policy lock");
        policy.verdict(space, p, policy::local_minute(), shared.speaker.depth())
    };
    if let Some(status) = refusal {
        return Response::Finished { status };
    }
    // Refuse before accepting, so the sender is told rather than the failure
    // being buried in this device's log.
    if let Err(e) = shared.engine.ready() {
        return Response::error(e.to_string());
    }
    shared.speaker.submit(
        Job {
            msg_id: msg_id.clone(),
            chunks,
            voice,
            space: space.map(str::to_string),
            done,
        },
        p == Priority::High,
    );
    Response::Accepted { msg_id }
}

/// Speak here, reporting what actually happened.
///
/// Waits for the worker when asked, so a caller learns "spoken" rather than
/// merely "queued". Bounded, because a device speaking a long document should
/// not hold a caller open indefinitely.
async fn speak_here(
    shared: &Arc<Shared>,
    outgoing: &Outgoing,
) -> (Status, Option<u64>, Option<String>) {
    let Outgoing {
        msg_id,
        chunks,
        priority: p,
        wait,
        voice,
        timeout,
        from,
        space,
    } = outgoing;
    let (p, wait) = (*p, *wait);

    // Recorded before the policy has its say. The chunks are joined back:
    // they were split on sentence boundaries, so this is the message as it
    // was meant to be heard.
    remember(
        shared,
        Entry {
            msg_id: msg_id.clone(),
            text: chunks.join(" "),
            from: from.clone().unwrap_or_else(|| my_name(shared)),
            at: now_secs(),
            status: Status::Queued,
            priority: p,
            // Recorded so the history can say which space a message came in,
            // and so a per-space refusal can be read back to the space it
            // applied to rather than looking like a device-wide one.
            space: space.clone(),
        },
    );

    // A message the queue never accepted gets no completion callback, so its
    // outcome is written here instead. Without this a muted message sat in
    // the history reading "queued" for ever — and the whole point of keeping
    // one is to find the messages that were never heard.
    let settle = |status: Status| {
        remember_outcome(shared, msg_id, status.clone());
        status
    };

    if !wait {
        return match enqueue(
            shared,
            msg_id.clone(),
            chunks.clone(),
            p,
            voice.clone(),
            space.as_deref(),
        ) {
            Response::Accepted { .. } => (Status::Queued, None, None),
            Response::Error { message, .. } => (settle(Status::NoEngine), None, Some(message)),
            // A policy refusal — muted, quiet hours, or dropped chatter. It
            // is a terminal answer, so it travels back to the sender as is.
            Response::Finished { status } => {
                let why = refusal_detail(&status);
                (settle(status), None, why)
            }
            _ => (settle(Status::Dropped), None, None),
        };
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    match enqueue_inner(
        shared,
        msg_id.clone(),
        chunks.clone(),
        p,
        voice.clone(),
        space.as_deref(),
        Some(tx),
    ) {
        Response::Accepted { .. } => {}
        Response::Error { message, .. } => return (settle(Status::NoEngine), None, Some(message)),
        Response::Finished { status } => {
            let why = refusal_detail(&status);
            return (settle(status), None, why);
        }
        _ => return (settle(Status::Dropped), None, None),
    }

    // Worked out here rather than by the sender, because this is the device
    // that knows its own engine and rate — and, for a message arriving from a
    // peer, the sender could not have known them at all. Everything already
    // queued counts: a message waiting its turn is not being spoken slowly,
    // but the caller is waiting for it just the same.
    let limit = timeout.unwrap_or_else(|| {
        let ahead = shared.speaker.pending_words();
        estimated_wait(words_in(chunks) + ahead, shared.engine.rate())
    });

    let started = std::time::Instant::now();
    match tokio::time::timeout(limit, rx).await {
        // The reason travels with the status now, so a receiver that has an
        // engine which ran and failed says which command failed and how,
        // instead of "no engine" and nothing (#86).
        Ok(Ok(ended)) => (
            ended.status,
            Some(started.elapsed().as_millis() as u64),
            ended.detail,
        ),
        // We gave up waiting before the device finished. It was accepted and
        // is still going, so say *that* — "queued" reads as though nothing
        // had started, which is exactly wrong for a long message that is
        // halfway through being read aloud.
        _ => (
            Status::Speaking,
            None,
            Some("still speaking; --timeout to wait longer".into()),
        ),
    }
}

/// Carry out a control command here, on named devices, or on both.
///
/// Resolved through the same selector machinery as speech, so `stop --to all`
/// means what `--to all` means everywhere else, and reaches every device
/// concurrently rather than one after another.
async fn control(
    shared: &Arc<Shared>,
    transport: &Arc<Transport>,
    to: Option<String>,
    control: Control,
) -> Response {
    let targets = match resolve(shared, to.as_deref().unwrap_or("here")).await {
        Ok(targets) => targets,
        // Every failure `resolve` reports is the selector matching nothing,
        // which is a well-formed command naming a device that is not here —
        // not the same as a malformed one, and promised its own exit code
        // since `docs/cli.md` was written (#66).
        Err(message) => return Response::no_target(message),
    };

    let mut set = tokio::task::JoinSet::new();
    for (index, target) in targets.into_iter().enumerate() {
        let shared = Arc::clone(shared);
        let transport = Arc::clone(transport);
        let control = control.clone();
        set.spawn(async move {
            let result = match target {
                Target::Here { shadowed } => TargetResult {
                    device: my_name(&shared),
                    endpoint_id: shared.identity.id().to_string(),
                    status: apply_control(&shared, &control),
                    took_ms: None,
                    detail: also_answers_to(&shadowed),
                },
                Target::Peer { name, id, space } => {
                    match send_control(&transport, &id, &space, &control).await {
                        Ok((status, detail)) => TargetResult {
                            device: name,
                            endpoint_id: id,
                            status,
                            took_ms: None,
                            detail,
                        },
                        Err(e) => TargetResult {
                            device: name,
                            endpoint_id: id,
                            status: Status::Unreachable,
                            took_ms: None,
                            detail: Some(format!("{e:#}")),
                        },
                    }
                }
            };
            (index, result)
        });
    }

    let mut done: Vec<(usize, TargetResult)> = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(pair) => done.push(pair),
            Err(e) => eprintln!("control task failed: {e}"),
        }
    }
    done.sort_by_key(|(i, _)| *i);
    Response::Controlled {
        targets: done.into_iter().map(|(_, r)| r).collect(),
    }
}

/// Do it here.
fn apply_control(shared: &Arc<Shared>, control: &Control) -> Status {
    match control {
        Control::Stop { msg_id: Some(id) } => {
            // Distinguished so `stop --id` on a message that already finished
            // says so, rather than reporting a cancellation that never was.
            if shared.speaker.stop_message(id) {
                Status::Cancelled
            } else {
                Status::Dropped
            }
        }
        Control::Stop { msg_id: None } => {
            shared.speaker.clear();
            Status::Cancelled
        }
        Control::Skip => {
            shared.speaker.skip();
            Status::Cancelled
        }
        Control::Pause => {
            shared.speaker.pause();
            Status::Queued
        }
        Control::Resume => {
            shared.speaker.unpause();
            Status::Speaking
        }
    }
}

/// Ask a peer to do it.
async fn send_control(
    transport: &Arc<Transport>,
    peer_id: &str,
    _space: &str,
    control: &Control,
) -> Result<(Status, Option<String>)> {
    let peer = peer_id.parse().context("bad endpoint id in roster")?;
    let conn = transport.connect(peer).await?;
    let (mut send, mut recv) = conn.open_bi().await.context("opening control stream")?;
    write_msg(
        &mut send,
        &PeerMessage::Control {
            control: control.clone(),
        },
    )
    .await?;
    send.finish().ok();
    match read_msg(&mut recv).await? {
        PeerMessage::Report { status, detail } => Ok((status, detail)),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

/// This device's policy and its per-space overrides, as the CLI and app read.
///
/// Overrides are labelled rather than keyed by id, and a space whose override
/// outlived it is skipped: a row naming a space this device no longer holds
/// would be a control with nothing behind it.
async fn policy_response(shared: &Arc<Shared>) -> Response {
    let policies = shared.policy.lock().expect("policy lock").clone();
    let p = policies.device;
    let spaces = {
        let held = shared.spaces.lock().await;
        let mut rows: Vec<voicecast_proto::SpacePolicy> = policies
            .spaces
            .iter()
            .filter(|(id, _)| held.get(id).is_some())
            .map(|(id, over)| voicecast_proto::SpacePolicy {
                label: held.label(id).to_string(),
                muted: over.muted,
                quiet_from: over.quiet.map(|q| policy::format_time(q.from)),
                quiet_to: over.quiet.map(|q| policy::format_time(q.to)),
                high_breaks_through: over.quiet.is_some_and(|q| q.high_breaks_through),
            })
            .collect();
        // By label, because that is the order they are shown in. The map is
        // ordered by space id, which is a hash and means nothing to a reader.
        rows.sort_by(|a, b| a.label.cmp(&b.label));
        rows
    };
    Response::Policy {
        muted: p.muted,
        quiet_from: p.quiet.map(|q| policy::format_time(q.from)),
        quiet_to: p.quiet.map(|q| policy::format_time(q.to)),
        high_breaks_through: p.quiet.is_some_and(|q| q.high_breaks_through),
        spaces,
    }
}

/// Drop a space's policy override, for a space that no longer exists.
///
/// Best effort: failing to persist this leaves a stale entry that
/// `policy_response` already filters out of what anyone can see, so a write
/// error here is not worth failing the operation the caller actually asked for.
fn forget_policy(shared: &Arc<Shared>, space: &str) {
    let mut p = shared.policy.lock().expect("policy lock");
    if p.space(space).is_none() {
        return;
    }
    p.forget(space);
    let _ = policy::save(&p);
}

/// Which policy a request is editing: the device's, or one space's.
///
/// Returns the space id, or `None` for the device. An unknown label is an
/// error rather than a quiet fall back to the device policy — silently muting
/// a whole device because a space name was mistyped is the worst outcome
/// available here.
/// The error is the message rather than a whole `Response`: a `Result` whose
/// `Err` carries one is large enough for clippy to object, and the caller has
/// to build a `Response::Error` from it either way.
async fn policy_target(
    shared: &Arc<Shared>,
    space: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(label) = space else {
        return Ok(None);
    };
    space_named(shared, Some(label)).await.map(Some)
}

/// Silence this device or one of its spaces, or let it speak again.
///
/// Muting stops what is being said now as well as what comes next. Letting
/// the current message run to the end would be a strange reading of "quiet".
/// A space mute stops the current message too — this device cannot tell which
/// space the sound already leaving the speaker belongs to, and stopping is the
/// safe way to be wrong.
async fn set_mute(shared: &Arc<Shared>, muted: bool, space: Option<&str>) -> Response {
    let target = match policy_target(shared, space).await {
        Ok(target) => target,
        Err(message) => return Response::error(message),
    };
    {
        let mut p = shared.policy.lock().expect("policy lock");
        match &target {
            None => p.device.muted = muted,
            Some(id) => {
                let mut over = p.space(id).copied().unwrap_or_default();
                over.muted = muted;
                p.set_space(id, over);
            }
        }
        if let Err(e) = policy::save(&p) {
            return Response::error(format!("could not save the policy: {e}"));
        }
    }
    if muted {
        shared.engine.stop();
    }
    policy_response(shared).await
}

/// Set or clear a daily quiet window, device-wide or for one space.
async fn set_quiet(
    shared: &Arc<Shared>,
    from: Option<String>,
    to: Option<String>,
    high_breaks_through: bool,
    space: Option<&str>,
) -> Response {
    let target = match policy_target(shared, space).await {
        Ok(target) => target,
        Err(message) => return Response::error(message),
    };
    let quiet = match (from, to) {
        (Some(f), Some(t)) => {
            let (Some(from), Some(to)) = (policy::parse_time(&f), policy::parse_time(&t)) else {
                return Response::error(format!("times must look like 22:00, got '{f}' and '{t}'"));
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
    {
        let mut p = shared.policy.lock().expect("policy lock");
        match &target {
            None => p.device.quiet = quiet,
            Some(id) => {
                let mut over = p.space(id).copied().unwrap_or_default();
                over.quiet = quiet;
                p.set_space(id, over);
            }
        }
        if let Err(e) = policy::save(&p) {
            return Response::error(format!("could not save the policy: {e}"));
        }
    }
    policy_response(shared).await
}

/// Recent messages, newest first.
fn history_response(shared: &Arc<Shared>, limit: Option<usize>) -> Response {
    let history = shared.history.lock().expect("history lock");
    Response::History {
        entries: history
            .recent(limit.unwrap_or(50))
            .into_iter()
            .map(|e| voicecast_proto::HistoryEntry {
                unheard: e.unheard(),
                msg_id: e.msg_id,
                text: e.text,
                from: e.from,
                at: e.at,
                status: e.status,
                priority: e.priority,
            })
            .collect(),
    }
}

/// Speak a message from the history again, here.
///
/// Deliberately skips mute and quiet hours. Those exist to stop a device
/// making noise unasked; pressing play *is* the ask, and refusing it would
/// make the history unreadable exactly when it is most useful — while the
/// device is still muted.
///
/// Keeps the original id, so a message that was never heard is marked as
/// heard once it has been played.
fn replay(shared: &Arc<Shared>, msg_id: &str) -> Response {
    let entry = {
        let history = shared.history.lock().expect("history lock");
        history.get(msg_id).cloned()
    };
    let Some(entry) = entry else {
        return Response::error(format!("no message {msg_id} in the history"));
    };
    if let Err(e) = shared.engine.ready() {
        return Response::error(e.to_string());
    }
    let chunks = chunk(&entry.text);
    if chunks.is_empty() {
        return Response::error("that message has no text");
    }
    shared.speaker.submit(
        Job {
            msg_id: entry.msg_id.clone(),
            chunks,
            voice: None,
            // A replay is this device speaking its own history, not the
            // space's message arriving again.
            space: None,
            done: None,
        },
        false,
    );
    Response::Accepted {
        msg_id: entry.msg_id,
    }
}

/// Note that a message was asked for, before anything is decided about it.
///
/// Recorded even when policy is about to refuse it: a message that arrived
/// while the device was muted is precisely the one someone will want to go
/// back and read, and dropping it would make muting silently lose things.
fn remember(shared: &Arc<Shared>, entry: Entry) {
    let mut history = shared.history.lock().expect("history lock");
    history.record(entry);
    if let Err(e) = history.save(&shared.history_path) {
        eprintln!("could not save history: {e}");
    }
}

/// Fill in how a message ended, once it has.
fn remember_outcome(shared: &Arc<Shared>, msg_id: &str, status: Status) {
    let mut history = shared.history.lock().expect("history lock");
    history.set_status(msg_id, status);
    if let Err(e) = history.save(&shared.history_path) {
        eprintln!("could not save history: {e}");
    }
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
async fn sync_roster(
    shared: &Arc<Shared>,
    conn: &iroh::endpoint::Connection,
    space: &str,
) -> Result<()> {
    let (mut send, mut recv) = conn.open_bi().await.context("opening roster stream")?;
    let mine = {
        let spaces = shared.spaces.lock().await;
        let Some(roster) = spaces.get(space) else {
            // The space went away under us — rotated, or left.
            return Ok(());
        };
        PeerMessage::RosterSync {
            members: roster.members().cloned().collect(),
            revoked: roster.tombstones(),
            space: Some(space.to_string()),
        }
    };
    write_msg(&mut send, &mine).await?;
    send.finish().ok();

    match read_msg(&mut recv).await? {
        PeerMessage::RosterSync {
            members, revoked, ..
        } => {
            merge_from_peer(shared, space, members, revoked).await?;
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
            let mut spaces = shared.spaces.lock().await;
            // The space this sync was about, not whichever one this peer
            // happens to share with us first. Being dropped from one space
            // says nothing about any other, and `space_of` answers with the
            // default before anything else — so a peer leaving a second
            // space was removed from the one it was still a member of (#51).
            if let Some(roster) = spaces.get_mut(space)
                && roster.allows(&conn.remote_id())
            {
                roster.revoke(&peer);
                spaces.save(&shared.spaces_path)?;
                eprintln!("{peer} no longer shares that space with us; removed");
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
    space: &str,
    members: Vec<Member>,
    revoked: Vec<(String, u64)>,
) -> Result<()> {
    let mut spaces = shared.spaces.lock().await;
    let theirs = Roster::from_parts(members, revoked);
    let Some(roster) = spaces.get_mut(space) else {
        return Ok(());
    };
    roster.merge(&theirs);
    // Our own label is ours to decide; a peer's older copy must not overwrite
    // a rename we just made.
    roster.rename(&shared.identity.id().to_string(), &my_name(shared));
    spaces.save(&shared.spaces_path)?;
    Ok(())
}

/// Open a stream to a peer and stream the message down it.
async fn send_to_peer(
    shared: &Arc<Shared>,
    transport: &Arc<Transport>,
    peer_id: &str,
    space: &str,
    outgoing: &Outgoing,
) -> Result<(Status, Option<String>)> {
    let peer = peer_id.parse().context("bad endpoint id in roster")?;
    let conn = transport.connect(peer).await?;

    // Piggyback a roster exchange: this is the moment both sides are known to
    // be reachable, so it costs one extra stream and keeps names and
    // membership converging without any background chatter.
    if let Err(e) = sync_roster(shared, &conn, space).await {
        eprintln!("roster sync with {peer_id}: {e:#}");
    }

    let (mut send, mut recv) = conn.open_bi().await.context("opening message stream")?;

    write_msg(
        &mut send,
        &PeerMessage::SpeakBegin {
            msg_id: outgoing.msg_id.clone(),
            priority: outgoing.priority,
            wait: outgoing.wait,
            voice: outgoing.voice.clone(),
            space: Some(space.to_string()),
            // Only what the caller asked for. Absent means the receiver
            // estimates, which it is far better placed to do.
            timeout_secs: outgoing.timeout.map(|t| t.as_secs()),
        },
    )
    .await?;
    for (seq, text) in outgoing.chunks.iter().enumerate() {
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
        PeerMessage::Report { status, detail } => Ok((status, detail)),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

/// Mint an invite.
async fn invite(shared: &Arc<Shared>, space: Option<&str>) -> Response {
    let space = match space_named(shared, space).await {
        Ok(id) => id,
        Err(message) => return Response::error(message),
    };
    // Recorded on the ticket rather than decided when the joiner arrives:
    // those are different questions, and the person pressing the button was
    // answering the first one.
    // The label as well as the id: the id picks the roster, the label is what
    // the joining device can show a person before it has spoken to anyone.
    let label = shared.spaces.lock().await.label(&space).to_string();
    let ticket = Ticket::mint(shared.identity.id().to_string(), Some(space), Some(label));
    let url = match ticket.to_url() {
        Ok(u) => u,
        Err(e) => {
            return Response::error(e.to_string());
        }
    };
    let expires_in = ticket.remaining();
    // Written down as well as held, so restarting the app mid-pairing does
    // not silently invalidate a code someone is looking at.
    ticket.remember();
    *shared.pending.lock().await = Some(ticket);
    Response::Invite { url, expires_in }
}

/// Join a space using someone else's ticket.
async fn join(
    shared: &Arc<Shared>,
    transport: &Arc<Transport>,
    raw: &str,
    label: Option<String>,
) -> Response {
    let ticket = match Ticket::parse(raw) {
        Ok(t) => t,
        Err(e) => {
            return Response::error(format!("{e:#}"));
        }
    };
    match do_join(shared, transport, &ticket, label.as_deref()).await {
        Ok((count, space)) => Response::Joined {
            members: count,
            space,
        },
        Err(e) => Response::error(format!("{e:#}")),
    }
}

/// Read an invite without acting on it.
///
/// No network and no state: `Ticket::parse` already refuses an expired or
/// mangled code with a sentence the person can act on, so the failure a
/// preview reports is the same one the join would have reported — just
/// before rather than after committing to it.
fn preview(raw: &str) -> Response {
    match Ticket::parse(raw) {
        Ok(t) => Response::Preview {
            label: t.label.clone(),
            expires_in: t.remaining(),
            endpoint_id: t.endpoint_id.clone(),
        },
        Err(e) => Response::error(format!("{e:#}")),
    }
}

async fn do_join(
    shared: &Arc<Shared>,
    transport: &Arc<Transport>,
    t: &Ticket,
    wanted: Option<&str>,
) -> Result<(usize, String)> {
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
            display_name: my_name(shared),
            token: t.token.clone(),
        },
    )
    .await?;

    match read_msg(&mut recv).await? {
        PeerMessage::JoinAccepted {
            member,
            members,
            space: agreed,
            label: theirs,
        } => {
            let mut spaces = shared.spaces.lock().await;
            // Adopt the space's membership wholesale rather than blending it
            // with whatever we had — but *add* it rather than replace what we
            // already belong to. Replacing was right when a device could hold
            // one space and became wrong the moment it could hold several: a
            // device in `home` that joined `work` lost `home`.
            let all = members.into_iter().chain(std::iter::once(member));
            let mut joined = Roster::from_parts(all.collect(), Vec::new());
            // The inviter has already told us what this space is called, and
            // its answer beats the one derived here — see `adopt_id`.
            if let Some(agreed) = agreed.as_deref() {
                joined.adopt_id(agreed);
            }
            // Rejoining a space we already hold is ordinary — a re-pair, or
            // a second scan — but the roster that arrives carries no
            // tombstones, so taking it wholesale forgot every revocation this
            // device had made and let a removed peer sync itself back in
            // (#76). Merging is add-only, so the arriving members land and
            // our own revocations survive.
            let joined_id = joined.space_id();
            if let Some(existing) = spaces.get(&joined_id) {
                joined.merge(existing);
            }
            let count = joined.members().count();

            // A space holding only this device is one nobody ever joined —
            // the empty space every node founds for itself at first start.
            // Displacing that is what `replace_current` was written for, and
            // it stops a fresh device ending up with an abandoned space
            // beside the one it just joined.
            let me = shared.identity.id().to_string();
            // What to call it here. The joiner's choice first, then the name
            // the inviter uses — live from the reply, which beats the ticket's
            // copy if the space was renamed after the invite was minted — then
            // the ticket, and only then a counter. Naming it `space-2` when
            // every hop carried "work" was issue #36: the person is told what
            // they are joining and then shown something else.
            let name = pick_space_label(
                &spaces,
                Some(joined_id.as_str()),
                [wanted, theirs.as_deref(), t.label.as_deref()],
            );
            let id = if spaces.current_is_unshared(&me) {
                // The empty space every node founds for itself is displaced
                // rather than kept beside the one just joined. Its name goes
                // with it: "main" was a placeholder for a roster that no
                // longer exists, and keeping it would be the same bug in the
                // other direction.
                spaces.replace_current(joined);
                spaces.default_id().to_string()
            } else {
                spaces.insert(joined, &name)
            };
            // Renaming after the fact rather than at insert, because the
            // displacing branch re-keys the space and never sees `name`.
            // A clash cannot happen — `pick_space_label` already skipped
            // every taken name — but a refusal here is not worth failing a
            // join that has already been accepted on the other side.
            let _ = spaces.set_label(&id, &name);
            let label = spaces.label(&id).to_string();
            spaces.save(&shared.spaces_path)?;
            Ok((count, label))
        }
        PeerMessage::JoinRefused { reason } => anyhow::bail!("{reason}"),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

/// Change this device's label.
///
/// Roster sync now exists, so the new name does travel: it is stamped with
/// `renamed_at` and the newer stamp wins on every peer. What made that stop
/// working was this function updating the file and one roster while
/// `Shared.name` kept the old string — the next sync, within the minute,
/// restamped our own entry with the *stale* copy and a fresher time, so the
/// old name then won everywhere including here (#62).
///
/// Three things therefore have to move together: the name file, the copy in
/// memory that sync writes back, and the entry in every roster rather than
/// only the default one.
async fn rename(shared: &Arc<Shared>, name: &str) -> Response {
    let name = name.trim();
    if name.is_empty() {
        return Response::error("a device name cannot be empty");
    }
    // The same rule spaces use, for the same reason: these names are what
    // `--to` parses, so a comma splits one device into two targets and a
    // slash reads as a space qualifier. `all` and `here` are decided before
    // any lookup happens, so a device wearing one can never be addressed.
    if let Some(message) = name_objection(name) {
        return Response::error(message);
    }
    if let Err(e) = crate::set_device_name(name) {
        return Response::error(e.to_string());
    }
    // Before the rosters, so a sync racing this cannot write the old name
    // back: `merge_from_peer` reads this lock to restamp our own entry.
    *shared.name.write().expect("name lock") = name.to_string();

    let mut spaces = shared.spaces.lock().await;
    // Every space, not just the default. A device in two spaces was renamed
    // in one of them, so the other went on calling it by the old name and
    // `--to work/desk` never matched.
    let me = shared.identity.id().to_string();
    for id in spaces.ids() {
        if let Some(roster) = spaces.get_mut(&id) {
            roster.rename(&me, name);
        }
    }
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return Response::error(e.to_string());
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
async fn revoke(shared: &Arc<Shared>, name: &str, space: Option<&str>) -> Response {
    let space = match space_named(shared, space).await {
        Ok(id) => id,
        Err(message) => return Response::error(message),
    };
    let me = shared.identity.id().to_string();
    let mut spaces = shared.spaces.lock().await;

    let Some(target) = spaces
        .get(&space)
        .and_then(|r| r.by_name(name))
        .map(|m| m.endpoint_id.clone())
    else {
        return Response::error(format!("no device named '{name}' in this space"));
    };
    if target == me {
        return Response::error("that is this device — use `voicecast leave` instead");
    }

    if let Some(roster) = spaces.get_mut(&space) {
        roster.revoke(&target);
    }
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return Response::error(e.to_string());
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
async fn leave(shared: &Arc<Shared>, transport: &Arc<Transport>, space: Option<&str>) -> Response {
    cancel_open_invite(shared).await;
    let space = match space_named(shared, space).await {
        Ok(id) => id,
        Err(message) => return Response::error(message),
    };
    let me = shared.identity.id().to_string();

    // A roster carrying our own tombstone. Peers merging this drop us.
    let (farewell, peers, label) = {
        let spaces = shared.spaces.lock().await;
        let Some(roster) = spaces.get(&space) else {
            return Response::error("that space is no longer held");
        };
        let label = spaces.label(&space).to_string();
        let mut goodbye = roster.clone();
        goodbye.revoke(&me);
        let peers: Vec<String> = roster
            .members()
            .filter(|m| m.endpoint_id != me)
            .map(|m| m.endpoint_id.clone())
            .collect();
        (goodbye, peers, label)
    };

    let mut told = 0usize;
    for peer in &peers {
        if announce_departure(transport, peer, &space, &farewell)
            .await
            .is_ok()
        {
            told += 1;
        }
    }

    let mut spaces = shared.spaces.lock().await;
    // Remove it, unless it is the only space this device has. A device with
    // no space cannot speak even to itself, so the last one is replaced by a
    // fresh empty one instead — which is what leaving used to do to *every*
    // space, and why leaving one of several appeared to do nothing at all:
    // the replacement carried the same local name and the same lone member.
    let refounded = spaces.ids().len() <= 1;
    if refounded {
        spaces.replace_current(Roster::leave(shared.identity.secret(), &my_name(shared)));
    } else if let Err(message) = spaces.remove(&space) {
        return Response::error(message);
    }
    // The id is gone either way — removed, or re-keyed by refounding. An
    // override left behind would apply again to whatever space is founded
    // with that id next, which is silence nobody asked for.
    forget_policy(shared, &space);
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return Response::error(e.to_string());
    }

    Response::Left {
        space: label,
        unreached: peers.len() - told,
        refounded,
    }
}

/// Replace this space with a fresh one founded here.
///
/// Fold one arriving chunk into the message being assembled.
///
/// Returns false once the message has grown past what this device will speak,
/// at which point the caller refuses the whole thing rather than saying part
/// of it. Oversized chunks are re-chunked rather than refused, so a peer that
/// chunks differently still works: the limit is about what this device will
/// synthesise in one breath, not about who is allowed to talk to it.
fn accept_chunk(chunks: &mut Vec<String>, seen: &mut usize, text: String) -> bool {
    *seen = seen.saturating_add(text.chars().count());
    if *seen > MAX_MESSAGE_CHARS {
        return false;
    }
    if text.chars().count() > voicecast_text::MAX_CHUNK {
        chunks.extend(voicecast_text::chunk(&text));
    } else {
        chunks.push(text);
    }
    true
}

/// Why this cannot be a device label, if it cannot.
///
/// The same rule spaces use, for the same reason: a device name is what
/// `--to` parses, so a comma splits one device into two targets and a slash
/// reads as a space qualifier. `all` and `here` are answered before any
/// lookup happens, so a device wearing either can never be addressed at all.
/// Nothing checked, which meant a device could be renamed — locally, or by
/// any member over a roster sync — into something unaddressable.
fn name_objection(name: &str) -> Option<String> {
    if name.trim().is_empty() {
        return Some("a device name cannot be empty".into());
    }
    if name.contains('/') || name.contains(',') {
        return Some("a device name cannot contain '/' or ','".into());
    }
    if name.eq_ignore_ascii_case("all") || name.eq_ignore_ascii_case("here") {
        return Some(format!(
            "'{name}' already means something to --to; pick another name"
        ));
    }
    None
}

/// This device's label, as it stands now.
fn my_name(shared: &Shared) -> String {
    shared.name.read().expect("name lock").clone()
}

/// Longest message this device will speak in one go, in characters.
///
/// About two hours of speech, which is far past anything anyone sends and
/// still a bound. The number that matters is that there *is* one: without it
/// a member could stream 8 MB frames until the receiver ran out of memory,
/// and a single frame that size would occupy the speech thread for hours
/// with `stop` unable to interrupt it (#53, and #58 for the second half).
const MAX_MESSAGE_CHARS: usize = 100_000;

/// Cancel any invite still open on this device.
///
/// The three operations that call this change what an outstanding ticket
/// would admit somebody *to*, and a ticket is a bearer token with a
/// five-minute life: it names a space by id and carries a token this device
/// checks against its own copy. Rotating while one was on screen left that
/// copy in place, so the ticket stayed valid, and the space it named was
/// gone — which is how a scan after the panic button was pressed landed in
/// the space the panic button had just built (#50).
///
/// Locks `pending` and nothing else. `accept_join` takes `pending` before
/// `spaces`, so every caller here must cancel *before* taking the spaces
/// lock or the two orders meet in the middle.
async fn cancel_open_invite(shared: &Arc<Shared>) {
    shared.pending.lock().await.take();
    Ticket::forget();
}

/// The panic button. Revocation is eventually consistent, so a device that
/// has been offline since the revoke still honours the revoked member until
/// it syncs — fine for a laptop that was sold, useless for a phone that was
/// stolen. Rotating sidesteps the wait entirely: the excluded device is not
/// in the new space and never was, so there is nothing for it to find out.
///
/// Deliberately silent. `leave` announces itself because the point is to be
/// dropped; this one must not, since the only devices listening include the
/// one being excluded. Survivors discover it the next time they make contact
/// and are refused, which already makes them drop this device — the same
/// self-healing path a stale peer takes.
///
/// Everyone has to be re-invited, which is the cost of locking one device out
/// immediately. It is only bearable because joining is cheap.
async fn rotate(shared: &Arc<Shared>, space: Option<&str>) -> Response {
    // Before the spaces lock, and before the space is even resolved: a
    // ticket that outlives a rotation is the failure this guards, and
    // cancelling one that turns out not to have needed it costs a re-show.
    cancel_open_invite(shared).await;
    let space = match space_named(shared, space).await {
        Ok(id) => id,
        Err(message) => return Response::error(message),
    };
    let me = shared.identity.id().to_string();
    let mut spaces = shared.spaces.lock().await;

    let Some(roster) = spaces.get(&space) else {
        return Response::error("that space is no longer held");
    };
    let label = spaces.label(&space).to_string();
    let devices: Vec<String> = roster
        .members()
        .filter(|m| m.endpoint_id != me)
        .map(|m| m.name.clone())
        .collect();

    // The space that was asked for, not whichever is default. Getting this
    // wrong destroyed the default space and left the named one intact — with
    // its keys, which is the security failure, since replacing a space is how
    // a device that is no longer trusted is locked out.
    if !spaces.replace(
        &space,
        Roster::found(shared.identity.secret(), &my_name(shared)),
    ) {
        return Response::error("that space is no longer held");
    }
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return Response::error(e.to_string());
    }
    // Rotating mints a space with a new id, so the old override names nothing.
    // Dropped rather than carried across: the new space has different members
    // and a setting somebody chose for the old one is a guess about the new.
    forget_policy(shared, &space);
    Response::Rotated {
        space: label,
        devices,
    }
}

/// A local name for a space this device has just joined.
///
/// Joining has to call it something, and the space itself carries no name —
/// labels are local, like device labels. Numbered rather than guessed from
/// the inviter, because a guess that collides is worse than a placeholder
/// somebody renames.
/// The first of `wanted` that is usable as a local name, else a counter.
///
/// "Usable" means non-empty, legal, and not already the name of a different
/// space here — labels qualify device names, so `work/laptop` cannot mean two
/// things. Falling through the whole list is normal: it is what a ticket
/// minted before labels travelled does, and what a second join from the same
/// space does.
fn pick_space_label<'a>(
    spaces: &Spaces,
    own: Option<&str>,
    wanted: impl IntoIterator<Item = Option<&'a str>>,
) -> String {
    for candidate in wanted.into_iter().flatten() {
        let candidate = candidate.trim();
        if candidate.is_empty() || candidate.contains('/') || candidate.contains(',') {
            continue;
        }
        // A name is taken only if some *other* space has it. Counting the
        // space's own current label as a clash is what renamed `work` to
        // `work-2` when a device rejoined a space it already held — the
        // collision was with itself (#76).
        match spaces.by_label(candidate) {
            None => return candidate.to_string(),
            Some(holder) if Some(holder.as_str()) == own => return candidate.to_string(),
            Some(_) => continue,
        }
    }
    next_space_label(spaces)
}

fn next_space_label(spaces: &Spaces) -> String {
    (2..)
        .map(|n| format!("space-{n}"))
        .find(|name| spaces.by_label(name).is_none())
        .expect("an unused name exists")
}

/// Which space a request means, by its local name.
///
/// `None` is the default, which is what every caller meant before spaces
/// could be named. An unknown name is an error rather than a silent fallback
/// to the default: acting on the wrong space is the failure worth refusing,
/// and it is exactly what a per-space button would otherwise do.
async fn space_named(shared: &Arc<Shared>, label: Option<&str>) -> Result<String, String> {
    let spaces = shared.spaces.lock().await;
    match label {
        None => Ok(spaces.default_id().to_string()),
        Some(label) => spaces.by_label(label).ok_or_else(|| {
            format!(
                "no space called '{label}'. Known: {}",
                space_labels(&spaces)
            )
        }),
    }
}

/// The spaces this device belongs to.
async fn list_spaces(shared: &Arc<Shared>) -> Response {
    let spaces = shared.spaces.lock().await;
    Response::Spaces {
        spaces: spaces
            .list(&shared.identity.id().to_string())
            .into_iter()
            .map(|s| voicecast_proto::SpaceRow {
                label: s.label,
                devices: s.devices,
                is_default: s.is_default,
                founded_here: s.founded_here,
            })
            .collect(),
    }
}

/// Found a new space from this device.
///
/// Additive: the spaces already held are untouched, and the new one becomes
/// the default so the invites that follow land in it.
async fn new_space(shared: &Arc<Shared>, label: &str) -> Response {
    let mut spaces = shared.spaces.lock().await;
    if spaces.by_label(label).is_some() {
        return Response::error(format!("there is already a space called '{label}'"));
    }
    let roster = Roster::found(shared.identity.secret(), &my_name(shared));
    let id = roster.space_id();
    spaces.insert(roster, label);
    if let Err(e) = spaces.set_label(&id, label) {
        return Response::error(e);
    }
    if let Err(e) = spaces.set_default(&id) {
        return Response::error(e);
    }
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return Response::error(e.to_string());
    }
    drop(spaces);
    list_spaces(shared).await
}

/// Drop one space, keeping the others.
///
/// Announced, like `leave`: the point is to be forgotten by the devices left
/// behind, and telling them is what makes that immediate rather than eventual.
async fn leave_space(shared: &Arc<Shared>, label: &str) -> Response {
    cancel_open_invite(shared).await;
    let mut spaces = shared.spaces.lock().await;
    let Some(id) = spaces.by_label(label) else {
        return Response::error(format!("no space called '{label}'"));
    };
    if let Err(message) = spaces.remove(&id) {
        return Response::error(message);
    }
    forget_policy(shared, &id);
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return Response::error(e.to_string());
    }
    drop(spaces);
    list_spaces(shared).await
}

/// Choose which space bare device names resolve in.
async fn default_space(shared: &Arc<Shared>, label: &str) -> Response {
    let mut spaces = shared.spaces.lock().await;
    let Some(id) = spaces.by_label(label) else {
        return Response::error(format!("no space called '{label}'"));
    };
    if let Err(message) = spaces.set_default(&id) {
        return Response::error(message);
    }
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return Response::error(e.to_string());
    }
    drop(spaces);
    list_spaces(shared).await
}

/// Rename a space locally.
async fn rename_space(shared: &Arc<Shared>, label: &str, to: &str) -> Response {
    let mut spaces = shared.spaces.lock().await;
    let Some(id) = spaces.by_label(label) else {
        return Response::error(format!("no space called '{label}'"));
    };
    if let Err(message) = spaces.set_label(&id, to) {
        return Response::error(message);
    }
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return Response::error(e.to_string());
    }
    drop(spaces);
    list_spaces(shared).await
}

/// Push a roster carrying our own tombstone to one peer.
async fn announce_departure(
    transport: &Arc<Transport>,
    peer_id: &str,
    space: &str,
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
            space: Some(space.to_string()),
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
    let spaces = shared.spaces.lock().await;
    let ids = spaces.ids();
    // Only worth labelling when there is a choice; one space would give a
    // column that always reads the same.
    let several = ids.len() > 1;

    // The default space first, so the devices a bare name reaches are at the
    // top rather than wherever the id happened to sort.
    let mut ordered = vec![spaces.default_id().to_string()];
    ordered.extend(ids.into_iter().filter(|id| id != spaces.default_id()));

    let mut devices = Vec::new();
    for id in ordered {
        let Some(roster) = spaces.get(&id) else {
            continue;
        };
        for m in roster.members() {
            devices.push(DeviceInfo {
                last_seen_secs: if m.endpoint_id == me {
                    Some(0)
                } else {
                    seen.get(&m.endpoint_id)
                        .map(|t| now_secs().saturating_sub(*t))
                },
                name: m.name.clone(),
                endpoint_id: m.endpoint_id.clone(),
                is_self: m.endpoint_id == me,
                space: several.then(|| spaces.label(&id).to_string()),
            });
        }
    }
    Response::Devices { devices }
}

/// Serve one peer connection.
async fn handle_peer<C: crate::transport::PeerConnection>(
    shared: &Arc<Shared>,
    conn: C,
) -> Result<()> {
    let remote = conn.remote();
    // Anything reaching us proves that peer is alive right now.
    mark_seen(shared, &remote.to_string()).await;
    while let Some((mut send, mut recv)) = conn.accept_bi().await {
        match read_msg(&mut recv).await? {
            PeerMessage::JoinRequest {
                endpoint_id,
                display_name,
                token,
            } => {
                // The record is signed for whoever is on the other end of
                // this connection, not for whoever the message names. They
                // are the same device every time a real client asks, and
                // where they differ is the whole attack: a ticket holder
                // enrolling a *third* key it does not hold, leaving a member
                // that revoking the device in front of you does not remove
                // (#52).
                let reply = if endpoint_id != remote.to_string() {
                    PeerMessage::JoinRefused {
                        reason: "the join request names a different device than the one \
                                 that sent it"
                            .into(),
                    }
                } else {
                    accept_join(shared, &remote.to_string(), &display_name, &token).await
                };
                write_msg(&mut send, &reply).await?;
            }
            PeerMessage::SpeakBegin {
                msg_id,
                priority,
                wait,
                voice,
                space,
                timeout_secs,
            } => {
                // Authorisation is the roster of the space the message was
                // sent in, and nothing else: an unpaired device cannot make
                // this one speak, and membership of one space grants nothing
                // in another.
                // The resolved id is kept, not just the yes/no: it is also
                // what selects this space's receiver policy further down.
                let in_space = space_for(shared, space, &remote).await;
                let allowed = match &in_space {
                    Some(id) => shared
                        .spaces
                        .lock()
                        .await
                        .get(id)
                        .is_some_and(|r| r.allows(&remote)),
                    None => false,
                };
                if !allowed {
                    write_msg(
                        &mut send,
                        &PeerMessage::Report {
                            status: Status::Rejected,
                            detail: Some("this device is not in that space".into()),
                        },
                    )
                    .await?;
                    continue;
                }
                // What arrives is whatever the peer chose to call a chunk.
                // The sender's chunking is a courtesy: it runs the text
                // through `voicecast_text` first, but a receiver that assumes
                // so is trusting a member not to be hostile or broken. One
                // chunk long enough held the speech thread for the life of
                // the process, and a stream that never ends grew this vector
                // until the phone killed the app (#53).
                let mut chunks: Vec<String> = Vec::new();
                let mut seen = 0usize;
                let mut too_long = false;
                loop {
                    match read_msg(&mut recv).await? {
                        PeerMessage::Chunk { text, .. } => {
                            if !accept_chunk(&mut chunks, &mut seen, text) {
                                too_long = true;
                                break;
                            }
                        }
                        PeerMessage::SpeakEnd => break,
                        other => anyhow::bail!("unexpected in message stream: {other:?}"),
                    }
                }
                if too_long {
                    // Said plainly, with the limit in it, because the sender
                    // is usually an agent that can shorten and try again.
                    write_msg(
                        &mut send,
                        &PeerMessage::Report {
                            status: Status::Rejected,
                            detail: Some(format!(
                                "message longer than {MAX_MESSAGE_CHARS} characters; \
                                 send it in parts"
                            )),
                        },
                    )
                    .await?;
                    continue;
                }
                // Waiting here is what lets a sender learn "spoken" rather
                // than "queued": only this device knows when the sound ended.
                // The sender's label, so the history says who it came from
                // rather than showing a public key.
                let from = {
                    let spaces = shared.spaces.lock().await;
                    let remote = remote.to_string();
                    spaces.ids().into_iter().find_map(|id| {
                        spaces
                            .get(&id)?
                            .members()
                            .find(|m| m.endpoint_id == remote)
                            .map(|m| m.name.clone())
                    })
                };
                let outgoing = Outgoing {
                    msg_id: msg_id.clone(),
                    chunks,
                    priority,
                    wait,
                    voice,
                    timeout: timeout_secs.map(std::time::Duration::from_secs),
                    from,
                    space: in_space,
                };
                let (status, _took, _detail) = speak_here(shared, &outgoing).await;
                write_msg(
                    &mut send,
                    &PeerMessage::Report {
                        status,
                        detail: None,
                    },
                )
                .await?;
            }
            PeerMessage::RosterSync {
                members,
                revoked,
                space,
            } => {
                // Only members may change our roster. Without this any device
                // that can reach us could inject entries — and, more visibly,
                // a device we just left would push us straight back into the
                // space it still thinks we are in.
                let Some(space) = space_for(shared, space, &remote).await else {
                    write_msg(
                        &mut send,
                        &PeerMessage::JoinRefused {
                            reason: "not a member of this space".into(),
                        },
                    )
                    .await?;
                    continue;
                };
                let member = shared
                    .spaces
                    .lock()
                    .await
                    .get(&space)
                    .is_some_and(|r| r.allows(&remote));
                if !member {
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
                merge_from_peer(shared, &space, members, revoked).await?;

                let mine = {
                    let spaces = shared.spaces.lock().await;
                    match spaces.get(&space) {
                        Some(roster) => PeerMessage::RosterSync {
                            members: roster.members().cloned().collect(),
                            revoked: roster.tombstones(),
                            space: Some(space.clone()),
                        },
                        None => continue,
                    }
                };
                // Best-effort: the peer may already be gone, and that is fine.
                let _ = write_msg(&mut send, &mine).await;
            }
            PeerMessage::Control { control } => {
                // Same rule as speech: only a device in a space with us may
                // silence us. Without this anyone reachable could.
                let allowed = match space_for(shared, None, &remote).await {
                    Some(id) => shared
                        .spaces
                        .lock()
                        .await
                        .get(&id)
                        .is_some_and(|r| r.allows(&remote)),
                    None => false,
                };
                let status = if allowed {
                    apply_control(shared, &control)
                } else {
                    Status::Rejected
                };
                write_msg(
                    &mut send,
                    &PeerMessage::Report {
                        status,
                        detail: None,
                    },
                )
                .await?;
            }
            PeerMessage::Hello { .. } => {}
            other => anyhow::bail!("unexpected message: {other:?}"),
        }
    }
    Ok(())
}

/// Which of this device's spaces a peer message belongs to.
///
/// A peer that names one is taken at its word, provided we hold that space.
/// One that does not — a build from before spaces, or simply an older
/// message — is placed by looking up where we already know it from, which is
/// unambiguous whenever the two devices share a single space.
async fn space_for(
    shared: &Arc<Shared>,
    named: Option<String>,
    remote: &iroh::EndpointId,
) -> Option<String> {
    let spaces = shared.spaces.lock().await;
    match named {
        // A peer that names a space is answered about that space or not at
        // all. The fallback below is for peers too old to name one, and
        // letting it fire for a space we no longer hold merged one space's
        // membership into another: a device that left `work` had every work
        // device merged into its `home` roster on the next presence check,
        // after which they could speak to it (#51).
        Some(id) => spaces.get(&id).is_some().then_some(id),
        None => spaces.space_of(&remote.to_string()),
    }
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
        // Named from the joiner's point of view, because that is who reads
        // it. "Run invite first" sounded like an instruction for this end.
        return PeerMessage::JoinRefused {
            reason: "the inviting device has no invite open — it may have expired, \
                     already been used, or the app was restarted. Show a new one there"
                .into(),
        };
    };
    if !ticket.is_valid() {
        *pending = None;
        Ticket::forget();
        return PeerMessage::JoinRefused {
            reason: "that invite has expired; show a new one on the inviting device".into(),
        };
    }
    if ticket.token != token {
        return PeerMessage::JoinRefused {
            reason: "that invite is not valid".into(),
        };
    }
    // Which space this invite was for, read before the ticket is consumed.
    let wanted = ticket.space.clone();

    // Single use: consumed here so a ticket seen over a shoulder, or left in
    // scrollback, cannot be replayed.
    *pending = None;
    Ticket::forget();
    drop(pending);

    let mut spaces = shared.spaces.lock().await;
    // The space the ticket named, not whichever happens to be default now.
    // Those differ the moment someone changes the default between showing an
    // invite and it being scanned, and the ticket is what the person pressing
    // the button was answering.
    let space_id = match wanted {
        Some(id) if spaces.get(&id).is_some() => id,
        // Falling back to the default here was the whole of #50: a ticket
        // shown before `rotate` named a space that no longer existed, so
        // whoever scanned it afterwards was admitted to the space rotating
        // had just created — the one the rotation existed to protect.
        Some(_) => {
            return PeerMessage::JoinRefused {
                reason: "the space that invite was for is no longer on this device. \
                         Show a new invite there"
                    .into(),
            };
        }
        None => spaces.default_id().to_string(),
    };
    let Some(roster) = spaces.get_mut(&space_id) else {
        return PeerMessage::JoinRefused {
            reason: "that space is no longer on this device".into(),
        };
    };
    let member = roster.invite(shared.identity.secret(), endpoint_id, name);
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return PeerMessage::JoinRefused {
            reason: format!("could not record membership: {e}"),
        };
    }
    let members: Vec<Member> = spaces
        .get(&space_id)
        .map(|r| r.members().cloned().collect())
        .unwrap_or_default();
    let label = spaces.label(&space_id).to_string();
    PeerMessage::JoinAccepted {
        member,
        members,
        space: Some(space_id),
        // The name as it stands now, which is what the joiner should adopt.
        // The ticket's copy was written when the invite was minted and is
        // stale if the space has been renamed since.
        label: Some(label),
    }
}

/// Note that a peer was reachable just now.
async fn mark_seen(shared: &Arc<Shared>, peer: &str) {
    shared
        .last_seen
        .lock()
        .await
        .insert(peer.to_string(), now_secs());
}

/// The first 16 characters of an endpoint id, for showing a person.
///
/// Counted in characters, not bytes. Ids reaching this are meant to be
/// base32 keys, and `roster::verify` now refuses any that are not, but
/// slicing a `str` at a byte offset panics on a multi-byte boundary and this
/// runs inline on the node's own IPC task — so being wrong once cost the
/// whole node rather than one bad line of output (#52).
fn short_id(id: &str) -> String {
    id.chars().take(16).collect()
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

#[cfg(test)]
mod tests {

    use super::{MAX_MESSAGE_CHARS, accept_chunk, name_objection};

    #[test]
    fn a_device_name_that_would_break_a_selector_is_refused() {
        // Each of these is addressable by nothing once set: `--to` splits on
        // the comma, reads the slash as a space qualifier, and answers `all`
        // and `here` before it ever looks a device up.
        for bad in ["a,b", "work/desk", "all", "ALL", "here", "Here", "  "] {
            assert!(
                name_objection(bad).is_some(),
                "{bad:?} must not be usable as a device name"
            );
        }
    }

    #[test]
    fn ordinary_device_names_are_accepted() {
        for good in [
            "desk",
            "Björn's iPad",
            "kitchen speaker",
            "here-ish",
            "all-in-one",
        ] {
            assert_eq!(name_objection(good), None, "{good:?} is a fine name");
        }
    }

    #[test]
    fn an_oversized_chunk_is_split_rather_than_spoken_whole() {
        // A peer's chunking is a courtesy. One chunk of 5,000 characters used
        // to go to the engine as a single utterance, which on the streaming
        // path filled Piper's output pipe and deadlocked the speech thread
        // for the life of the process (#53).
        let mut chunks = Vec::new();
        let mut seen = 0;
        let long = "word ".repeat(1_000);
        assert!(accept_chunk(&mut chunks, &mut seen, long));
        assert!(chunks.len() > 1, "it was split");
        for c in &chunks {
            assert!(
                c.chars().count() <= voicecast_text::MAX_CHUNK,
                "every piece is speakable: {} chars",
                c.chars().count()
            );
        }
    }

    #[test]
    fn an_ordinary_chunk_is_passed_through_untouched() {
        let mut chunks = Vec::new();
        let mut seen = 0;
        assert!(accept_chunk(&mut chunks, &mut seen, "Tea is ready.".into()));
        assert_eq!(chunks, vec!["Tea is ready."]);
    }

    #[test]
    fn a_stream_that_never_ends_is_refused_rather_than_buffered() {
        // The frame cap bounds one message; nothing bounded how many a member
        // could send before SpeakEnd, so the vector grew until the phone
        // killed the app.
        let mut chunks = Vec::new();
        let mut seen = 0;
        let block = "a".repeat(voicecast_text::MAX_CHUNK);
        let mut accepted = 0;
        for _ in 0..100_000 {
            if !accept_chunk(&mut chunks, &mut seen, block.clone()) {
                break;
            }
            accepted += 1;
        }
        assert!(
            seen > MAX_MESSAGE_CHARS,
            "the loop stopped because of the bound"
        );
        assert!(
            accepted < 100_000,
            "it refused rather than accepting everything"
        );
    }

    /// The message that exposed the bug now gets long enough to finish.
    ///
    /// 569 words measured at 147.6 seconds of audio through Piper's
    /// `en_US-lessac-medium`; the old flat 120 seconds cut the report short
    /// while the device was still speaking.
    #[test]
    fn a_long_message_is_given_longer_than_it_takes_to_say() {
        let estimate = estimated_wait(569, 1.0);
        assert!(
            estimate > std::time::Duration::from_secs_f32(147.6),
            "569 words estimated at {estimate:?}, which is less than the \
             audio it produces"
        );
        // The old constant, for the avoidance of doubt.
        assert!(estimate > std::time::Duration::from_secs(120));
    }

    /// A short message is not made to wait a long time for nothing.
    #[test]
    fn a_short_message_gets_the_floor_and_no_more() {
        assert_eq!(estimated_wait(1, 1.0), MIN_TIMEOUT);
        assert_eq!(estimated_wait(0, 1.0), MIN_TIMEOUT);
    }

    /// Longer text waits longer. The property the old constant lacked.
    #[test]
    fn the_estimate_grows_with_the_text() {
        let short = estimated_wait(100, 1.0);
        let long = estimated_wait(1_000, 1.0);
        assert!(long > short, "{long:?} should exceed {short:?}");
    }

    /// A device set to speak slowly is given proportionally longer.
    ///
    /// The rate is the receiver's own setting, which is the reason this is
    /// estimated on the device that will do the speaking.
    #[test]
    fn a_slower_device_waits_longer() {
        let normal = estimated_wait(1_000, 1.0);
        let half_speed = estimated_wait(1_000, 0.5);
        assert!(
            half_speed > normal,
            "{half_speed:?} should exceed {normal:?}"
        );
    }

    /// However much text arrives, a stuck engine cannot hold a caller for ever.
    #[test]
    fn the_estimate_is_capped() {
        assert_eq!(estimated_wait(usize::MAX, 1.0), MAX_TIMEOUT);
        assert_eq!(estimated_wait(10_000_000, 1.0), MAX_TIMEOUT);
    }

    /// A nonsense rate cannot divide by zero or produce a negative wait.
    #[test]
    fn an_absurd_rate_is_still_a_sane_wait() {
        for rate in [0.0, -1.0, f32::MIN_POSITIVE] {
            let estimate = estimated_wait(500, rate);
            assert!(
                estimate >= MIN_TIMEOUT && estimate <= MAX_TIMEOUT,
                "{estimate:?}"
            );
        }
    }
    use super::*;

    /// A socket name unique to this test run, so tests never collide.
    fn unique(label: &str) -> String {
        format!("voicecast-test-{}-{label}.sock", std::process::id())
    }

    /// A socket left behind by a node that died is reclaimed, not refused.
    ///
    /// The failure this covers is macOS-shaped but not macOS-only: anywhere
    /// the name is a file rather than an abstract address, a `kill -9` leaves
    /// it behind and every later node fails to bind.
    #[tokio::test]
    async fn reclaims_a_socket_a_dead_node_left() {
        let socket = unique("stale");

        // Exactly what a crash leaves: a bound name whose owner is gone and
        // which was never cleaned up on the way out.
        let mut abandoned = bind_ipc(&socket).await.expect("first bind");
        abandoned.do_not_reclaim_name_on_drop();
        drop(abandoned);

        let reclaimed = bind_ipc(&socket).await;
        assert!(reclaimed.is_ok(), "{:?}", reclaimed.err());
    }

    /// A node that is actually running is never displaced.
    ///
    /// The whole reason reclamation cannot simply overwrite on `AddrInUse`:
    /// doing so would let a second node quietly steal the socket from the
    /// first, and the CLI would reach whichever won.
    #[tokio::test]
    async fn refuses_to_displace_a_live_node() {
        let socket = unique("live");
        let _live = bind_ipc(&socket).await.expect("first bind");

        let second = bind_ipc(&socket).await;
        assert!(second.is_err(), "a second node must not take the socket");
        let message = second.unwrap_err().to_string();
        assert!(message.contains("already running"), "{message}");
    }

    /// The name travels; it should be used. Issue #36.
    ///
    /// Joining `work` used to produce a space called `space-2` on the joining
    /// device — the label rides on both the ticket and the acceptance, and
    /// `do_join` read neither. Since spaces are addressed by label, a locally
    /// invented name makes every `work/laptop` a guess.
    #[test]
    fn a_joined_space_keeps_the_name_it_arrived_with() {
        let spaces = Spaces::default();
        assert_eq!(
            pick_space_label(&spaces, None, [None, Some("work"), None]),
            "work"
        );
    }

    /// The joiner's own choice beats the inviter's name for it.
    ///
    /// The label is local — it is how *this* device writes `work/laptop` —
    /// so two people can reasonably disagree and the one joining decides.
    #[test]
    fn the_joiner_gets_the_last_word_on_the_name() {
        let spaces = Spaces::default();
        assert_eq!(
            pick_space_label(&spaces, None, [Some("theirs"), Some("ours"), None]),
            "theirs"
        );
    }

    /// A name already in use here falls through to the next candidate.
    ///
    /// Labels qualify device names, so two spaces called `work` would make
    /// `work/laptop` mean two things.
    #[test]
    fn a_name_already_taken_is_skipped() {
        use iroh_base::SecretKey;
        let mut spaces = Spaces::default();
        spaces.insert(
            Roster::found(&SecretKey::from_bytes(&[1; 32]), "me"),
            "work",
        );
        assert_eq!(
            pick_space_label(&spaces, None, [Some("work"), None, None]),
            "space-2"
        );
    }

    /// A ticket that names nothing still produces a usable name.
    ///
    /// That is what a ticket minted before labels travelled looks like, and
    /// it is not an error — the counter is the honest fallback.
    #[test]
    fn nothing_to_go_on_falls_back_to_the_counter() {
        let spaces = Spaces::default();
        assert_eq!(
            pick_space_label(&spaces, None, [None, None, None]),
            "space-2"
        );
    }

    /// A space rejoining under its own name keeps it.
    ///
    /// `by_label` answering "taken" for the space asking the question renamed
    /// `work` to `work-2` every time a device re-paired into a space it
    /// already held. The collision was with itself (#76).
    #[test]
    fn a_space_rejoining_under_its_own_name_keeps_it() {
        let mut spaces = Spaces::default();
        let secret = iroh_base::SecretKey::generate();
        let id = spaces.insert(Roster::found(&secret, "me"), "work");

        assert_eq!(
            pick_space_label(&spaces, Some(id.as_str()), [Some("work"), None, None]),
            "work",
            "its own name is not taken from itself"
        );
        assert_eq!(
            pick_space_label(&spaces, None, [Some("work"), None, None]),
            "space-2",
            "and it is still taken from anybody else"
        );
    }

    /// A label carrying a separator is refused rather than stored.
    ///
    /// `/` and `,` are how selectors are written, so a space called `a/b`
    /// would be unaddressable — and the name comes off the wire, from a
    /// device this one has just met.
    #[test]
    fn a_name_that_would_break_a_selector_is_not_taken() {
        let spaces = Spaces::default();
        assert_eq!(
            pick_space_label(&spaces, None, [Some("a/b"), None, None]),
            "space-2"
        );
        assert_eq!(
            pick_space_label(&spaces, None, [Some("a,b"), None, None]),
            "space-2"
        );
        assert_eq!(
            pick_space_label(&spaces, None, [Some("   "), None, None]),
            "space-2"
        );
    }

    /// The regression that adding a payload to `Here` invites.
    ///
    /// `--to here,laptop` on a machine called `laptop` produces two elements
    /// that both mean this device and disagree about what they shadowed.
    /// Comparing the whole value would make the machine say it twice.
    #[test]
    fn this_device_collapses_however_it_was_named() {
        let mut targets = Vec::new();
        push_target(Target::Here { shadowed: vec![] }, &mut targets);
        push_target(
            Target::Here {
                shadowed: vec!["peer-a".into()],
            },
            &mut targets,
        );
        assert_eq!(targets.len(), 1, "this device speaks once");
    }

    /// And the shadow survives the collapse, whichever order it arrives in.
    #[test]
    fn collapsing_keeps_what_either_element_shadowed() {
        for order in [false, true] {
            let mut targets = Vec::new();
            let (first, second) = if order {
                (vec![], vec!["peer-a".to_string()])
            } else {
                (vec!["peer-a".to_string()], vec![])
            };
            push_target(Target::Here { shadowed: first }, &mut targets);
            push_target(Target::Here { shadowed: second }, &mut targets);
            assert_eq!(
                targets,
                vec![Target::Here {
                    shadowed: vec!["peer-a".to_string()]
                }],
                "a clash seen by either element has to reach the report"
            );
        }
    }

    /// Merging must not report the same device twice.
    #[test]
    fn the_same_shadowed_device_is_not_listed_twice() {
        let mut targets = Vec::new();
        for _ in 0..2 {
            push_target(
                Target::Here {
                    shadowed: vec!["peer-a".into()],
                },
                &mut targets,
            );
        }
        assert_eq!(
            targets,
            vec![Target::Here {
                shadowed: vec!["peer-a".to_string()]
            }]
        );
    }

    /// Peers still dedup on the whole value, which is what they always did.
    #[test]
    fn the_same_peer_named_twice_is_one_target() {
        let mut targets = Vec::new();
        let peer = || Target::Peer {
            name: "laptop".into(),
            id: "abc".into(),
            space: "s".into(),
        };
        push_target(peer(), &mut targets);
        push_target(peer(), &mut targets);
        assert_eq!(targets.len(), 1);
    }

    /// Nothing shadowed adds no text at all, so the common report is unchanged.
    #[test]
    fn a_clean_send_says_nothing_extra() {
        assert_eq!(also_answers_to(&[]), None);
    }

    /// One shadowed device reads as one, and names it.
    #[test]
    fn a_shadowed_device_is_named_and_counted() {
        let note = also_answers_to(&["0123456789abcdef0123456789".to_string()])
            .expect("a shadowed device is worth saying");
        assert!(note.contains("1 other device"), "{note}");
        assert!(note.contains("answers"), "singular verb: {note}");
        assert!(note.contains("0123456789abcdef"), "names it: {note}");
        assert!(
            !note.contains("0123456789abcdef0"),
            "truncated to 16 like the device list: {note}"
        );
    }

    /// Two read as two. The first version of this dropped the count in the
    /// plural and said only "other devices".
    #[test]
    fn two_shadowed_devices_are_counted() {
        let note = also_answers_to(&["aaaa".to_string(), "bbbb".to_string()])
            .expect("two shadowed devices are worth saying");
        assert!(note.contains("2 other devices"), "{note}");
        assert!(note.contains("aaaa") && note.contains("bbbb"), "{note}");
    }
}

/// Issue #80: the protocol had no test above the unit level.
///
/// `handle_peer` took a concrete `iroh::endpoint::Connection`, so the only
/// way to reach its sixteen arms was to bind an endpoint and have a real
/// second device dial it. Every fix to the receiving side — including the
/// join check below, which is a security fix — was therefore verified by
/// reading. This drives the same code with a pair of in-memory pipes.
#[cfg(test)]
mod peer_tests {
    use super::*;
    use crate::transport::{PeerConnection, read_msg, write_msg};
    use tokio::io::DuplexStream;

    /// A connection that hands over one stream pair and then ends.
    ///
    /// One rather than a loop because `handle_peer` runs until the peer goes
    /// away: a fake that kept yielding streams would never return, and the
    /// test would hang rather than fail.
    struct OneExchange {
        peer: iroh::EndpointId,
        streams: std::sync::Mutex<Option<(DuplexStream, DuplexStream)>>,
    }

    impl PeerConnection for OneExchange {
        type Send = DuplexStream;
        type Recv = DuplexStream;

        fn remote(&self) -> iroh::EndpointId {
            self.peer
        }

        async fn accept_bi(&self) -> Option<(Self::Send, Self::Recv)> {
            self.streams.lock().expect("streams").take()
        }
    }

    /// A node on a scratch directory, and the shared state `handle_peer` takes.
    async fn node_for(label: &str) -> (Node, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("voicecast-peer-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        crate::identity::set_config_dir(dir.clone());

        let store = crate::identity::FileKeyStore::at(dir.join("identity.key"));
        let identity = crate::identity::Identity::load_or_create(&store).expect("identity");
        let transport = Transport::bind(identity.secret().clone(), None)
            .await
            .expect("transport");
        let engine = Arc::new(voicecast_engine::SilentEngine::new("no engine in a test"));
        let node = Node::new(engine, identity, transport, "Test".into())
            .await
            .expect("node");
        (node, dir)
    }

    /// Issue #52, driven rather than read.
    ///
    /// A join request is signed for whoever is on the other end of the
    /// connection, not for whoever the message names. Where those differ is
    /// the whole attack: a ticket holder enrolling a *third* key it does not
    /// hold, leaving a member that revoking the device in front of you does
    /// not remove. Until now the check compiled and nothing executed it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_join_naming_another_device_is_refused() {
        let (node, dir) = node_for("join").await;

        // Two pipes: one each way. `handle_peer` is handed the node's ends.
        let (node_recv, mut test_send) = tokio::io::duplex(64 * 1024);
        let (node_send, mut test_recv) = tokio::io::duplex(64 * 1024);

        let dialer = iroh::SecretKey::generate().public();
        let someone_else = iroh::SecretKey::generate().public();

        // Written before the handler runs, so the pipe already holds the
        // request and `handle_peer` reads it without a second task.
        write_msg(
            &mut test_send,
            &PeerMessage::JoinRequest {
                endpoint_id: someone_else.to_string(),
                display_name: "Impostor".into(),
                token: "irrelevant".into(),
            },
        )
        .await
        .expect("writing the request");

        let conn = OneExchange {
            peer: dialer,
            streams: std::sync::Mutex::new(Some((node_send, node_recv))),
        };
        handle_peer(&node.shared, conn).await.expect("handle_peer");

        match read_msg(&mut test_recv).await.expect("a reply") {
            PeerMessage::JoinRefused { reason } => assert!(
                reason.contains("different device"),
                "refused for the wrong reason: {reason}"
            ),
            other => {
                panic!("a join naming a device other than the dialer was not refused: {other:?}")
            }
        }

        node.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
