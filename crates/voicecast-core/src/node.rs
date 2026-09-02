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
use crate::policy::{self, Policy};
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
        Err(e) => return Err(e).context("binding the local socket"),
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
    name: String,
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
        let speaker = Speaker::new(
            Arc::clone(&engine),
            Arc::new(move |msg_id, status| {
                if let Some(shared) = notify.lock().expect("recorder lock").upgrade() {
                    remember_outcome(&shared, msg_id, status);
                }
            }),
        );
        let shared = Arc::new(Shared {
            engine,
            identity,
            name,
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

    /// Replace this space with a fresh one, locking every other device out.
    pub async fn rotate(&self) -> Response {
        rotate(&self.shared).await
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
                        Target::Here => shared.name.clone(),
                        Target::Peer { name, .. } => name,
                    })
                    .collect(),
            },
            Err(message) => Response::Error { message },
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
        Request::Invite => invite(shared).await,
        Request::Join { ticket } => join(shared, transport, &ticket).await,
        Request::Devices => devices(shared).await,
        Request::Rename { name } => rename(shared, &name).await,
        Request::Revoke { name } => revoke(shared, &name).await,
        Request::Leave => leave(shared, transport).await,
        Request::Rotate => rotate(shared).await,
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
        Request::History { limit } => history_response(shared, limit),
        Request::Replay { msg_id } => replay(shared, &msg_id),
        Request::ClearHistory => {
            let mut history = shared.history.lock().expect("history lock");
            history.clear();
            let _ = history.save(&shared.history_path);
            Response::Done
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
        return Response::Error {
            message: "nothing to say".into(),
        };
    }

    let targets = match resolve(shared, to.as_deref().unwrap_or("here")).await {
        Ok(targets) => targets,
        Err(message) => return Response::Error { message },
    };

    let outgoing = Outgoing {
        msg_id: new_msg_id(),
        chunks,
        priority,
        wait,
        voice,
        timeout: timeout_secs.map(std::time::Duration::from_secs),
        // Typed here, so this device is the origin.
        from: None,
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
}

/// One resolved destination for a message.
#[derive(Clone, PartialEq, Eq)]
enum Target {
    /// This device.
    Here,
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
    let push = |t: Target, targets: &mut Vec<Target>| {
        if !targets.contains(&t) {
            targets.push(t);
        }
    };
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
                push(Target::Here, &mut targets);
            }
            for k in known.iter().filter(|k| k.space == space) {
                push(peer(k), &mut targets);
            }
            continue;
        }

        if scope.is_none() && (name.eq_ignore_ascii_case("here") || name == shared.name) {
            push(Target::Here, &mut targets);
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
                    device_names(&spaces, &shared.name)
                ));
            }
            [only] => push(peer(only), &mut targets),
            several => {
                let where_ = several
                    .iter()
                    .map(|k| spaces.label(&k.space))
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
                Target::Here => {
                    let (status, took_ms, detail) = speak_here(&shared, &outgoing).await;
                    TargetResult {
                        device: shared.name.clone(),
                        status,
                        took_ms,
                        detail,
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
        Ok(status) => TargetResult {
            device: name.to_string(),
            took_ms: outgoing.wait.then(|| started.elapsed().as_millis() as u64),
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
fn enqueue(
    shared: &Arc<Shared>,
    msg_id: String,
    chunks: Vec<String>,
    p: Priority,
    voice: Option<String>,
) -> Response {
    enqueue_inner(shared, msg_id, chunks, p, voice, None)
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
    done: Option<tokio::sync::oneshot::Sender<Status>>,
) -> Response {
    // Policy comes first. A muted device has no business reporting a broken
    // engine: the sender needs to hear the reason that actually applies, and
    // "muted" is both truer and more actionable than "no engine".
    let refusal = {
        let policy = shared.policy.lock().expect("policy lock");
        policy.verdict(p, policy::local_minute(), shared.speaker.depth())
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
    shared.speaker.submit(
        Job {
            msg_id: msg_id.clone(),
            chunks,
            voice,
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
            from: from.clone().unwrap_or_else(|| shared.name.clone()),
            at: now_secs(),
            status: Status::Queued,
            priority: p,
            space: None,
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
        return match enqueue(shared, msg_id.clone(), chunks.clone(), p, voice.clone()) {
            Response::Accepted { .. } => (Status::Queued, None, None),
            Response::Error { message } => (settle(Status::NoEngine), None, Some(message)),
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
        Some(tx),
    ) {
        Response::Accepted { .. } => {}
        Response::Error { message } => return (settle(Status::NoEngine), None, Some(message)),
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
        Ok(Ok(status)) => (status, Some(started.elapsed().as_millis() as u64), None),
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
        Err(message) => return Response::Error { message },
    };

    let mut set = tokio::task::JoinSet::new();
    for (index, target) in targets.into_iter().enumerate() {
        let shared = Arc::clone(shared);
        let transport = Arc::clone(transport);
        let control = control.clone();
        set.spawn(async move {
            let result = match target {
                Target::Here => TargetResult {
                    device: shared.name.clone(),
                    status: apply_control(&shared, &control),
                    took_ms: None,
                    detail: None,
                },
                Target::Peer { name, id, space } => {
                    match send_control(&transport, &id, &space, &control).await {
                        Ok(status) => TargetResult {
                            device: name,
                            status,
                            took_ms: None,
                            detail: None,
                        },
                        Err(e) => TargetResult {
                            device: name,
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
) -> Result<Status> {
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
        PeerMessage::Report { status } => Ok(status),
        other => anyhow::bail!("unexpected reply: {other:?}"),
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
        return Response::Error {
            message: format!("no message {msg_id} in the history"),
        };
    };
    if let Err(e) = shared.engine.ready() {
        return Response::Error {
            message: e.to_string(),
        };
    }
    let chunks = chunk(&entry.text);
    if chunks.is_empty() {
        return Response::Error {
            message: "that message has no text".into(),
        };
    }
    shared.speaker.submit(
        Job {
            msg_id: entry.msg_id.clone(),
            chunks,
            voice: None,
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
            // Only the space that peer was in: being dropped from one space
            // says nothing about any other.
            if let Some(id) = spaces.space_of(&peer)
                && let Some(roster) = spaces.get_mut(&id)
                && roster.allows(&conn.remote_id())
            {
                roster.revoke(&peer);
                spaces.save(&shared.spaces_path)?;
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
    roster.rename(&shared.identity.id().to_string(), &shared.name);
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
) -> Result<Status> {
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
    // Written down as well as held, so restarting the app mid-pairing does
    // not silently invalidate a code someone is looking at.
    ticket.remember();
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
        PeerMessage::JoinAccepted {
            member,
            members,
            space: _,
        } => {
            let mut spaces = shared.spaces.lock().await;
            // Replace rather than merge: joining a space means adopting its
            // membership, not blending it with whatever we had before.
            let all = members.into_iter().chain(std::iter::once(member));
            let joined = Roster::from_parts(all.collect(), Vec::new());
            let count = joined.members().count();
            spaces.replace_current(joined);
            spaces.save(&shared.spaces_path)?;
            Ok(count)
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
    let mut spaces = shared.spaces.lock().await;
    spaces
        .current_mut()
        .rename(&shared.identity.id().to_string(), name);
    if let Err(e) = spaces.save(&shared.spaces_path) {
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
    let mut spaces = shared.spaces.lock().await;

    let Some(target) = spaces
        .current()
        .by_name(name)
        .map(|m| m.endpoint_id.clone())
    else {
        return Response::Error {
            message: format!("no device named '{name}' in this space"),
        };
    };
    if target == me {
        return Response::Error {
            message: "that is this device — use `voicecast leave` instead".into(),
        };
    }

    spaces.current_mut().revoke(&target);
    if let Err(e) = spaces.save(&shared.spaces_path) {
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
    let (farewell, peers, space) = {
        let spaces = shared.spaces.lock().await;
        let roster = spaces.current();
        let space = spaces.default_id().to_string();
        let mut goodbye = roster.clone();
        goodbye.revoke(&me);
        let peers: Vec<String> = roster
            .members()
            .filter(|m| m.endpoint_id != me)
            .map(|m| m.endpoint_id.clone())
            .collect();
        (goodbye, peers, space)
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
    spaces.replace_current(Roster::leave(shared.identity.secret(), &shared.name));
    if let Err(e) = spaces.save(&shared.spaces_path) {
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

/// Replace this space with a fresh one founded here.
///
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
async fn rotate(shared: &Arc<Shared>) -> Response {
    let me = shared.identity.id().to_string();
    let mut spaces = shared.spaces.lock().await;

    let devices: Vec<String> = spaces
        .current()
        .members()
        .filter(|m| m.endpoint_id != me)
        .map(|m| m.name.clone())
        .collect();

    spaces.replace_current(Roster::found(shared.identity.secret(), &shared.name));
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return Response::Error {
            message: e.to_string(),
        };
    }
    Response::Rotated { devices }
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
        return Response::Error {
            message: format!("there is already a space called '{label}'"),
        };
    }
    let roster = Roster::found(shared.identity.secret(), &shared.name);
    let id = roster.space_id();
    spaces.insert(roster, label);
    if let Err(e) = spaces.set_label(&id, label) {
        return Response::Error { message: e };
    }
    if let Err(e) = spaces.set_default(&id) {
        return Response::Error { message: e };
    }
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return Response::Error {
            message: e.to_string(),
        };
    }
    drop(spaces);
    list_spaces(shared).await
}

/// Drop one space, keeping the others.
///
/// Announced, like `leave`: the point is to be forgotten by the devices left
/// behind, and telling them is what makes that immediate rather than eventual.
async fn leave_space(shared: &Arc<Shared>, label: &str) -> Response {
    let mut spaces = shared.spaces.lock().await;
    let Some(id) = spaces.by_label(label) else {
        return Response::Error {
            message: format!("no space called '{label}'"),
        };
    };
    if let Err(message) = spaces.remove(&id) {
        return Response::Error { message };
    }
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return Response::Error {
            message: e.to_string(),
        };
    }
    drop(spaces);
    list_spaces(shared).await
}

/// Choose which space bare device names resolve in.
async fn default_space(shared: &Arc<Shared>, label: &str) -> Response {
    let mut spaces = shared.spaces.lock().await;
    let Some(id) = spaces.by_label(label) else {
        return Response::Error {
            message: format!("no space called '{label}'"),
        };
    };
    if let Err(message) = spaces.set_default(&id) {
        return Response::Error { message };
    }
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return Response::Error {
            message: e.to_string(),
        };
    }
    drop(spaces);
    list_spaces(shared).await
}

/// Rename a space locally.
async fn rename_space(shared: &Arc<Shared>, label: &str, to: &str) -> Response {
    let mut spaces = shared.spaces.lock().await;
    let Some(id) = spaces.by_label(label) else {
        return Response::Error {
            message: format!("no space called '{label}'"),
        };
    };
    if let Err(message) = spaces.set_label(&id, to) {
        return Response::Error { message };
    }
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return Response::Error {
            message: e.to_string(),
        };
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
                voice,
                space,
                timeout_secs,
            } => {
                // Authorisation is the roster of the space the message was
                // sent in, and nothing else: an unpaired device cannot make
                // this one speak, and membership of one space grants nothing
                // in another.
                let allowed = match space_for(shared, space, &remote).await {
                    Some(id) => shared
                        .spaces
                        .lock()
                        .await
                        .get(&id)
                        .is_some_and(|r| r.allows(&remote)),
                    None => false,
                };
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
                };
                let (status, _took, _detail) = speak_here(shared, &outgoing).await;
                write_msg(&mut send, &PeerMessage::Report { status }).await?;
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
                write_msg(&mut send, &PeerMessage::Report { status }).await?;
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
    if let Some(id) = named
        && spaces.get(&id).is_some()
    {
        return Some(id);
    }
    spaces.space_of(&remote.to_string())
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
    // Single use: consumed here so a ticket seen over a shoulder, or left in
    // scrollback, cannot be replayed.
    *pending = None;
    Ticket::forget();
    drop(pending);

    let mut spaces = shared.spaces.lock().await;
    let member = spaces
        .current_mut()
        .invite(shared.identity.secret(), endpoint_id, name);
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return PeerMessage::JoinRefused {
            reason: format!("could not record membership: {e}"),
        };
    }
    let members: Vec<Member> = spaces.current().members().cloned().collect();
    let space_id = spaces.current().space_id();
    PeerMessage::JoinAccepted {
        member,
        members,
        space: Some(space_id),
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
}
