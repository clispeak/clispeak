//! The node: accepts local requests, speaks them, and relays them to peers.
//!
//! Two loops run side by side — a local IPC socket for the CLI, and an iroh
//! endpoint for other devices. The CLI is a thin client that hands over text
//! and exits; everything durable lives here.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clispeak_engine::SpeechEngine;
use clispeak_proto::{
    Control, DeviceInfo, Member, PeerMessage, Priority, Request, Response, Status, TargetResult,
};
use clispeak_text::chunk;
use interprocess::local_socket::{
    tokio::{Listener as TokioListener, Stream},
    // Anonymous: `connect` and `accept` are wanted, but the trait names
    // themselves would collide with the concrete types above.
    traits::tokio::{Listener, Stream as _},
};
use tokio::sync::Mutex;

use crate::history::{Entry, History, Saver};
use crate::ipc::{read_frame, socket_name, write_frame};
use crate::policy::{self, Policies};
use crate::queue::{Gating, Job, Speaker, words_in};
use crate::roster::RosterError;
use crate::spaces::Spaces;
use crate::transport::{Network, Wire, read_msg, write_msg};
use crate::{Identity, Roster, Ticket};

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
    // One place decides where the socket is *and* who may open it; this used
    // to spell out the name here and again in the CLI and the probe, which is
    // three chances to disagree. The access rules travel with it for the same
    // reason — set on the ordinary bind and forgotten on the reclaim below
    // would protect a normal start and not a recovery (#128).
    let name = || crate::ipc::socket_target(socket);
    let options = || crate::ipc::listener_for(socket);

    let refused = match options()?.create_tokio() {
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
        // Reached only when something took the name between `serve`'s probe
        // and this bind, or when `bind_ipc` is called without one. Both
        // possibilities are named because from here they are genuinely
        // indistinguishable — the token has been replaced by now, so nothing
        // holding the name can be identified any more (#128).
        anyhow::bail!(
            "the socket named {socket} is already held — by another node that \
             started a moment ago, or by another local process that took the name"
        );
    }

    eprintln!("removing the socket a previous node left at {socket}");
    options()?
        .try_overwrite(true)
        .create_tokio()
        .context("reclaiming the local socket")
}

/// Shared state both loops need.
/// What to do when the CLI asks for the window or for shutdown.
///
/// The node owns no UI, so the app installs these. Without an app — a headless
/// `clispeakd` — `show` has nothing to do and `quit` still works.
pub type WindowHook = Arc<dyn Fn() + Send + Sync>;

struct Shared {
    engine: Arc<dyn SpeechEngine>,
    identity: Identity,
    /// Where this node keeps its state.
    ///
    /// Carried rather than read from `identity::config_dir()` at each use.
    /// That global is a `OnceLock`: the first caller wins, so a second node
    /// in the same process silently gets the first one's directory and the
    /// two share an identity, a roster, a policy and an outstanding invite
    /// while believing they are strangers. Every test above the unit level
    /// needs two nodes at once, which is why #80 called this the one
    /// investment that unlocks the rest.
    ///
    /// The hosts are unaffected: `Node::new` still resolves the global and
    /// hands it here, so there is one directory in production and the
    /// difference exists only for callers that name their own.
    config_dir: PathBuf,
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
    /// Writes it, off whatever thread recorded something. Owns the path: no
    /// other caller writes the file, which is what keeps one writer.
    history_saver: Saver,
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
    /// Held as a trait object rather than a concrete `Transport`.
    ///
    /// A generic parameter would have reached `Node`, the app, the daemon and
    /// every function between them, to express something only a test cares
    /// about. See [`Network`].
    transport: Arc<dyn Network>,
}

/// Make this device's own roster entries agree with the name it advertises.
///
/// A device is the authority on its own name, and this is where that stops
/// being a claim: `device_name()` is what the last rename wrote down and what
/// every join request and every `--to` match uses, so any roster entry
/// disagreeing with it is stale by definition and is brought into line here.
/// Returns whether anything moved, so the caller knows to save.
///
/// **Every space, and on every start, including a migrating one.** It used to
/// be the current space only, and to be skipped entirely on the start that
/// migrated a legacy roster — so a device could hold a space calling it one
/// thing while its own settings screen, its join requests and every peer
/// called it another, with nothing to say which was right (#147). That is the
/// same "not just the default one" mistake `rename` already carries a comment
/// about, made once more in the one place a device gets a second chance to
/// notice.
///
/// The correction is announced when it has to make one. The disagreement has
/// been seen on a real phone and never reproduced; a line naming both strings
/// is what turns the next occurrence into evidence instead of a screenshot.
fn adopt_own_name(spaces: &mut Spaces, me: &str, name: &str) -> bool {
    let mut changed = false;
    for id in spaces.ids() {
        let label = spaces.label(&id).to_string();
        let Some(roster) = spaces.get_mut(&id) else {
            continue;
        };
        let was = roster.name_of(me).map(str::to_string);
        if roster.rename(me, name) {
            if let Some(was) = was {
                eprintln!(
                    "{label}: this device was recorded as {was:?}, but it is called \
                     {name:?} — corrected"
                );
            }
            changed = true;
        } else if roster.stamp_own_label(me) {
            changed = true;
        }
    }
    changed
}

impl Node {
    /// Start a node with the given engine, identity and transport.
    ///
    /// State goes wherever [`crate::identity::config_dir`] points, which is
    /// what every host wants. [`Node::new_in`] is the same thing with the
    /// directory named explicitly.
    pub async fn new(
        engine: Arc<dyn SpeechEngine>,
        identity: Identity,
        transport: impl Network,
        name: String,
    ) -> Result<Self> {
        let dir = crate::identity::config_dir().context("locating the config directory")?;
        Self::new_in(dir, engine, identity, transport, name).await
    }

    /// Start a node keeping its state in the directory given.
    ///
    /// The only difference from [`Node::new`] is where the state lives — and
    /// that difference is what lets two nodes run in one process, which every
    /// test above the unit level needs (#80).
    pub async fn new_in(
        config_dir: PathBuf,
        engine: Arc<dyn SpeechEngine>,
        identity: Identity,
        transport: impl Network,
        name: String,
    ) -> Result<Self> {
        // Apply a remembered voice before anything can be spoken with the
        // wrong one.
        if let Some((voice, rate)) = crate::identity::load_voice_settings_in(&config_dir) {
            let _ = engine.set_voice(&voice);
            let _ = engine.set_rate(rate);
        }

        let spaces_path = config_dir.join("spaces.cbor");
        let legacy = config_dir.join("roster.cbor");
        // Whether this device has been through the migration yet. Persisted
        // eagerly below so it happens exactly once, rather than being redone
        // from a roster file that nothing writes to any more.
        let migrating = !spaces_path.exists();
        let mut spaces = Spaces::load(&spaces_path, &legacy).context("loading spaces")?;
        // A device is always a member of its own space, even before anyone
        // else joins — otherwise it could not speak to itself.
        let founding = spaces.ids().is_empty();
        if founding {
            spaces.insert(Roster::found(identity.secret(), &name), "main");
        }
        // Two reasons to rewrite beyond that: the label changed, or it
        // predates the `renamed_at` stamp. Rosters written before that field
        // existed deserialize it as zero, and a merge comparing 0 > 0 keeps
        // the stale copy forever — so an unstamped entry is stamped once, on
        // the device that owns it.
        let corrected = adopt_own_name(&mut spaces, &identity.id().to_string(), &name);
        if founding || migrating || corrected {
            spaces.save(&spaces_path).context("saving spaces")?;
        }

        let history_path = config_dir.join("history.json");
        let history = std::sync::Mutex::new(History::load(&history_path));
        let history_saver = Saver::spawn(history_path);

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
            Arc::new(move |space: Option<&str>, priority: Priority| {
                let shared = gate.lock().expect("recorder lock").upgrade()?;
                let policy = shared.policy.lock().expect("policy lock");
                // The message's own priority, carried down from where it was
                // sent. It used to be `Priority::Normal` written out here, so
                // a `high` message passed the check at submit and was refused
                // by this one for being something it was not — which is the
                // whole of `high breaks through` never having worked (#246).
                //
                // Depth zero: the queue-depth rule drops a low-priority
                // message that would arrive too late to matter, and it has
                // already been applied once. Applying it again here, with
                // this message about to be spoken rather than waiting behind
                // anything, would be a different question with the same name.
                //
                // The two arguments beside each other are worth a moment. The
                // depth argument was reasoned about and got this paragraph;
                // the priority took a default and got nothing, which is
                // exactly how it stayed wrong for as long as it did.
                policy.verdict(space, priority, policy::local_minute(), 0)
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
            pending: Mutex::new(Ticket::recall_in(&config_dir)),
            speaker,
            history,
            history_saver,
            last_seen: Mutex::new(std::collections::HashMap::new()),
            policy: std::sync::Mutex::new(policy::load_in(&config_dir)),
            config_dir,
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
        // Before the transport, because this is the one place waiting for a
        // history write is the right thing to do: nothing is racing it and
        // the alternative is losing the last outcome recorded.
        self.shared.history_saver.flush();
        self.transport.close().await;
    }

    /// Install what `clispeak show` and `clispeak quit` should do.
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
        hand_to_saver(&self.shared, &history);
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
                        let _ = sync_roster(&node.shared, conn.as_ref(), &space).await;
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
                        if let Err(e) = handle_peer(&shared, conn.as_ref()).await {
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
        let config_dir = self.shared.config_dir.clone();

        // Before anything else: is there a second copy of this device's state
        // on this machine? Two directories can hold two rosters while sharing
        // the one identity in the keyring, and then whichever node starts
        // first decides which address book every peer sees. A device that
        // answers with the wrong history does not look broken — it looks like
        // one that has forgotten who it knows, and that is expensive to chase
        // (#198, decision 108).
        //
        // Refusing is the point. The alternative is starting successfully and
        // being subtly wrong, which is the failure this project keeps paying
        // for. The message names both directories because the fix is to
        // decide which one is real, and only the person can do that.
        //
        // **This refused a clean install once**, and decision 110 downgraded
        // it to a warning until the reason was understood rather than leaving
        // a refusal nobody could account for in the startup path. The reason
        // is now understood and it was not two directories at all: Flatpak
        // bind-mounts the host's config into the sandbox, so one directory had
        // two paths and this check reported the device as conflicting with
        // itself. `conflicting_identity` now asks the filesystem whether two
        // paths are the same place instead of comparing the strings, and the
        // refusal comes back with it (decision 111).
        if let Some(other) = crate::identity::conflicting_identity(&config_dir) {
            anyhow::bail!(
                "two copies of this device's state are on this machine, and they \
                 share one identity:\n  {}\n  {}\nBoth name the same keyring \
                 entry, so both are this device — but each has its own roster, \
                 so whichever starts first decides which devices this one \
                 remembers. Delete the directory you do not want, or set \
                 CLISPEAK_CONFIG_DIR to name the one you do. Nothing was \
                 started",
                config_dir.display(),
                other.display(),
            );
        }

        // Ask what holds the socket *before* writing a new token, because
        // writing one destroys the only thing that could identify a node
        // already running: `install_token` replaces the file, and the running
        // node still holds the old secret. Probing afterwards can only ever
        // report a stranger (#128).
        match crate::ipc::who_is_listening(&socket_name(), &config_dir).await {
            crate::ipc::Listening::Nothing => {}
            crate::ipc::Listening::Node => {
                anyhow::bail!(
                    "another clispeak node is already running on {}. Only one can \
                     hold this device's identity at a time — quit the other one",
                    socket_name()
                );
            }
            // Not "another node". It answered and could not prove it holds
            // this machine's token, so it is not a node this user started —
            // and saying so is the difference between looking for a second
            // app to quit and looking for whatever has the name.
            crate::ipc::Listening::Stranger(why) => {
                anyhow::bail!(
                    "something else on this machine holds the socket named {:?}, and \
                     it is not a clispeak node this user started: {why}. The socket \
                     name is unprotected on this platform, so any local process can \
                     take it first — set CLISPEAK_SOCKET to a different name to work \
                     around it. Nothing was sent to whatever holds it",
                    socket_name()
                );
            }
        }

        // Before the listener exists, so there is never a socket accepting
        // connections with no secret to check them against.
        let token =
            crate::ipc::install_token(&config_dir).context("writing the local socket token")?;

        // And a copy where a *host* CLI will look, when this node is not
        // running on the host. A Flatpak's configuration lives inside the
        // sandbox while the tool it installs into `~/.local/bin` reads
        // `~/.config`, so without this the app works, the socket is right,
        // and every command fails the handshake with an error blaming a
        // squatter for holding the name.
        //
        // Said out loud on failure rather than ignored. The whole failure is
        // one unwritten file, and the symptom points somewhere else entirely
        // — which is worth more than a tidy startup.
        match crate::ipc::publish_token_for_host(&token) {
            None => {}
            Some(Ok(path)) => eprintln!("token for the host CLI: {}", path.display()),
            Some(Err(e)) => {
                eprintln!(
                    "could not put a token where the host `clispeak` will find it: {e}\n\
                     The app will work and the command line tool will not: every call \n\
                     will fail saying something else holds the socket, which is not \n\
                     what happened. A Flatpak needs --filesystem=xdg-config/clispeak:create."
                );
            }
        }

        let listener = bind_ipc(&socket_name()).await?;

        let socket = socket_name();
        eprintln!("listening as {}", self.transport.id());
        // Not "listening on <socket>": that reads as a path, and the previous
        // wording sent people to `ls` a file that is at a different place on
        // macOS and does not exist at all on Linux. Issue #43.
        // The place, not just the name. This used to say "a name, not a
        // path" and then warn if the value looked like one, because the
        // platform decided where it landed and there was often no file
        // anywhere (#43). Now the node owns the location, so it can simply
        // say where it is — and a name with a separator is refused at bind
        // rather than warned about, which is what made the note unreachable.
        match crate::ipc::socket_dir() {
            Some(dir) => eprintln!("socket: {}", dir.join(&socket).display()),
            None => eprintln!("socket name: {socket:?} (a named pipe)"),
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
                        if let Err(e) = handle_cli(&shared, &transport, stream, &token).await {
                            // A liveness probe is not a failure worth printing;
                            // anything else is, including a caller that offered
                            // the wrong token.
                            if !e.is::<crate::ipc::Probe>() {
                                eprintln!("cli: {e:#}");
                            }
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
                                if let Err(e) = handle_peer(&shared, conn.as_ref()).await {
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
async fn handle_cli(
    shared: &Arc<Shared>,
    transport: &Arc<dyn Network>,
    mut s: Stream,
    token: &crate::ipc::Token,
) -> Result<()> {
    // Before the request is even read. `Request` is everything this device
    // can be told to do — speak, invite, join, rotate, revoke, read the
    // history, quit — and until now any local user could send one, because
    // the socket is a name with no permissions on Linux and a file in a
    // world-writable directory on macOS (#54).
    crate::ipc::accept_handshake(&mut s, token).await?;

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
            hand_to_saver(shared, &history);
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
        fallback: shared.engine.tier() == clispeak_engine::Tier::Fallback,
        queued: shared.speaker.depth(),
        // The node's own version, which is what a person means when they ask
        // "is this up to date" — not the CLI's, which can be a different
        // build entirely on a machine where the app has not been restarted.
        version: crate::version().to_string(),
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
async fn speak(shared: &Arc<Shared>, transport: &Arc<dyn Network>, ask: SpeakRequest) -> Response {
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
                         device itself: clispeak rename <new>"
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
            // A member's name is chosen on that device and this is the one
            // suggestion list that goes inside an error message, which the
            // CLI now prints with its line breaks intact. Escaped here, where
            // it is interpolated, and not on the way into the roster — see
            // decision 38 for why the roster keeps what the peer actually
            // said (#135).
            let name = clispeak_text::plain(&m.name);
            names.push(if several {
                format!("{}/{name}", spaces.label(id))
            } else {
                name
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
    transport: &Arc<dyn Network>,
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
    transport: &Arc<dyn Network>,
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
    shared.speaker.submit(Job {
        msg_id: msg_id.clone(),
        chunks,
        voice,
        space: space.map(str::to_string),
        // Carried rather than reduced to "is this urgent". The queue reads
        // the priority for its ordering and the speak-time policy check reads
        // it for its verdict, and there is now one of it (#246).
        gating: Gating::Sent(p),
        done,
    });
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
    transport: &Arc<dyn Network>,
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
                Target::Here { shadowed } => {
                    let (status, note) = apply_control(&shared, &control);
                    TargetResult {
                        device: my_name(&shared),
                        endpoint_id: shared.identity.id().to_string(),
                        status,
                        took_ms: None,
                        detail: both(also_answers_to(&shadowed), note),
                    }
                }
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
/// Apply a control here, and say what it actually did.
///
/// Every arm but the first used to return a fixed status describing the
/// *intent*: `stop` and `skip` said `cancelled` with nothing to cancel,
/// `pause` said `queued` with nothing queued, and `resume` said `speaking` on
/// a device making no sound. An agent reading that would record work that
/// never happened (#116).
///
/// The reasoning was already here, in the one arm that had it — `stop --id`
/// distinguishes `Cancelled` from `Dropped` "rather than reporting a
/// cancellation that never was". This is that sentence applied to the other
/// four.
///
/// `pause` and `resume` have no honest word in a vocabulary written for
/// messages, so they keep their nearest status and carry the truth in the
/// detail, which the CLI already prints in parentheses. Adding a `Status`
/// variant would be clearer and would break the wire for any older peer, so
/// it is Patrick's call and not made here.
fn apply_control(shared: &Arc<Shared>, control: &Control) -> (Status, Option<String>) {
    match control {
        Control::Stop { msg_id: Some(id) } => {
            // Distinguished so `stop --id` on a message that already finished
            // says so, rather than reporting a cancellation that never was.
            if shared.speaker.stop_message(id) {
                (Status::Cancelled, None)
            } else {
                (Status::Dropped, Some("no such message here".into()))
            }
        }
        Control::Stop { msg_id: None } => match shared.speaker.clear() {
            0 => (
                Status::Dropped,
                Some("nothing was playing or waiting".into()),
            ),
            1 => (Status::Cancelled, None),
            n => (Status::Cancelled, Some(format!("{n} messages"))),
        },
        Control::Skip => {
            if shared.speaker.skip() {
                (Status::Cancelled, None)
            } else {
                (Status::Dropped, Some("nothing was being spoken".into()))
            }
        }
        Control::Pause => {
            if shared.speaker.pause() {
                (Status::Queued, Some("held mid-message".into()))
            } else {
                (Status::Queued, Some("held; nothing was playing".into()))
            }
        }
        Control::Resume => {
            if shared.speaker.unpause() {
                (Status::Speaking, None)
            } else {
                (
                    Status::Speaking,
                    Some("resumed; nothing was waiting".into()),
                )
            }
        }
    }
}

/// Both notes, when both apply, so neither is silently dropped.
fn both(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(format!("{a}; {b}")),
        (a, b) => a.or(b),
    }
}

/// Ask a peer to do it.
async fn send_control(
    transport: &Arc<dyn Network>,
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
    send.finish();
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
        let mut rows: Vec<clispeak_proto::SpacePolicy> = policies
            .spaces
            .iter()
            .filter(|(id, _)| held.get(id).is_some())
            .map(|(id, over)| clispeak_proto::SpacePolicy {
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
    let _ = policy::save_in(&shared.config_dir, &p);
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
        if let Err(e) = policy::save_in(&shared.config_dir, &p) {
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
        if let Err(e) = policy::save_in(&shared.config_dir, &p) {
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
            .map(|e| clispeak_proto::HistoryEntry {
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
/// It is skipped in both places, which it was not for a while. Bypassing the
/// check at submit stopped being sufficient the moment a second check was
/// added at the point of speaking, and this paragraph went on describing
/// behaviour the code had quietly lost (#248).
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
    shared.speaker.submit(Job {
        msg_id: entry.msg_id.clone(),
        chunks,
        voice: None,
        // A replay is this device speaking its own history, not the
        // space's message arriving again.
        space: None,
        // Skipping the submit-time check was never enough on its own. Every
        // job also passes a check at the moment of speaking (#77), which
        // refused replays on a muted device and so undid the bypass the
        // comment above still described (#248).
        gating: Gating::Asked,
        done: None,
    });
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
    hand_to_saver(shared, &history);
}

/// Fill in how a message ended, once it has.
fn remember_outcome(shared: &Arc<Shared>, msg_id: &str, status: Status) {
    let mut history = shared.history.lock().expect("history lock");
    history.set_status(msg_id, status);
    hand_to_saver(shared, &history);
}

/// Serialise while the lock is held, and let the saver do the disk.
///
/// The lock has to be held to serialise a consistent snapshot; it must not be
/// held across the write, which is where `sync_all` sits. Splitting it this
/// way is the whole of #78 — see [`Saver`].
fn hand_to_saver(shared: &Arc<Shared>, history: &History) {
    match history.to_bytes() {
        Ok(bytes) => shared.history_saver.put(bytes),
        Err(e) => eprintln!("could not serialise history: {e}"),
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
/// never reaches anyone — which is what `clispeak rename` had to warn about.
async fn sync_roster(shared: &Arc<Shared>, conn: &dyn Wire, space: &str) -> Result<()> {
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
    send.finish();

    match read_msg(&mut recv).await? {
        PeerMessage::RosterSync {
            members, revoked, ..
        } => {
            merge_from_peer(shared, space, members, revoked).await?;
        }
        // The peer answered as though we are not a member: it left, or it
        // removed us. Either way there is no point showing a device that
        // will never answer, so it is dropped **here and nowhere else**.
        //
        // It used to be revoked, and the comment here said that was "safe by
        // construction: a peer can only ever make us forget *itself*". That
        // is true of one message and false of the system. A revoke mints a
        // tombstone, tombstones travel through `merge`, and they apply on
        // every device that receives one — so a single refused sync removed
        // that device from the whole space. When two devices refused each
        // other in the same minute the space came apart in thirty seconds,
        // taking the founder with it (#166).
        //
        // `forget` is the honest response to a *guess*. A real departure
        // already announces itself: `leave` sends a roster carrying its own
        // tombstone, which is a decision and is meant to propagate. This path
        // exists only for the case where that announcement went missing, and
        // it still heals it — the next sync with any other member brings the
        // real tombstone. What it no longer does is act on one refusal as
        // though it were a decision every device should adopt.
        PeerMessage::JoinRefused { .. } => {
            let peer = conn.remote().to_string();
            let mut spaces = shared.spaces.lock().await;
            // The space this sync was about, not whichever one this peer
            // happens to share with us first. Being dropped from one space
            // says nothing about any other, and `space_of` answers with the
            // default before anything else — so a peer leaving a second
            // space was removed from the one it was still a member of (#51).
            if let Some(roster) = spaces.get_mut(space)
                && roster.allows(&conn.remote())
                && roster.forget(&peer)
            {
                spaces.save(&shared.spaces_path)?;
                eprintln!(
                    "{peer} answered as though we are not in that space; \
                     forgotten here only. It comes back on the next sync with \
                     any other member, and if it really left, its own \
                     tombstone arrives the same way"
                );
            }
        }
        other => anyhow::bail!("unexpected reply to roster sync: {other:?}"),
    }
    mark_seen(shared, &conn.remote().to_string()).await;
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
    let offered = members.len();
    let (theirs, rejected) = Roster::from_parts(members, revoked);
    report_rejected("roster sync", offered, &rejected);
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
    transport: &Arc<dyn Network>,
    peer_id: &str,
    space: &str,
    outgoing: &Outgoing,
) -> Result<(Status, Option<String>)> {
    let peer = peer_id.parse().context("bad endpoint id in roster")?;
    let conn = transport.connect(peer).await?;

    // Piggyback a roster exchange: this is the moment both sides are known to
    // be reachable, so it costs one extra stream and keeps names and
    // membership converging without any background chatter.
    if let Err(e) = sync_roster(shared, conn.as_ref(), space).await {
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
    ticket.remember_in(&shared.config_dir);
    *shared.pending.lock().await = Some(ticket);
    Response::Invite { url, expires_in }
}

/// Join a space using someone else's ticket.
async fn join(
    shared: &Arc<Shared>,
    transport: &Arc<dyn Network>,
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

/// Say what a roster exchange threw away.
///
/// Silence here is what made decision 83 cost a day: three records arrived,
/// three were discarded, and the only thing said afterwards was that the
/// join had worked. A refusal is cheap to print and it names the record and
/// the reason, so the next mismatch is one line of output rather than a
/// day (#145).
fn report_rejected(what: &str, offered: usize, rejected: &[RosterError]) {
    if rejected.is_empty() {
        return;
    }
    eprintln!(
        "{what}: {} of {offered} membership records were refused",
        rejected.len()
    );
    for e in rejected {
        eprintln!("  {e}");
    }
}

/// Why a join the other device accepted still failed here.
///
/// Written for whoever reads it, which is why it names the count on both
/// sides and offers the fix rather than the diagnosis: a run of refused
/// signatures is what two devices on different builds look like from this
/// end, and it is indistinguishable from anything else without knowing that.
fn join_verification_failed(offered: usize, rejected: &[RosterError]) -> String {
    let Some(first) = rejected.first() else {
        return "the other device accepted this join but sent no membership record for \
                this device, so there is nothing here to be a member with. Show a new \
                invite there and try again"
            .to_string();
    };
    let kept = offered.saturating_sub(rejected.len());
    let mut why = format!(
        "the other device accepted this join, but this device's own membership record \
         did not verify here: {} of {offered} records were refused and {kept} kept. \
         First: {first}",
        rejected.len()
    );
    if rejected
        .iter()
        .all(|e| matches!(e, RosterError::BadSignature(_)))
    {
        why.push_str(
            ". Every one failed on its signature, which is what two devices running \
             different builds look like from here: put both on the same version and \
             pair again",
        );
    }
    why
}

async fn do_join(
    shared: &Arc<Shared>,
    transport: &Arc<dyn Network>,
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
            let all: Vec<Member> = members.into_iter().chain(std::iter::once(member)).collect();
            let offered = all.len();
            let (mut joined, rejected) = Roster::from_parts(all, Vec::new());
            report_rejected("join", offered, &rejected);
            // A join that kept no record of *us* is not a join, whatever the
            // other end said. This is the check that was missing when the
            // signing domain separator changed under decision 83: both
            // devices agreed the join had succeeded, and the roster it
            // produced was empty (#145).
            let me = shared.identity.id().to_string();
            if !joined.holds(&me) {
                anyhow::bail!("{}", join_verification_failed(offered, &rejected));
            }
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
        // Free-form text written by the other device, and it ends up inside
        // an error the CLI now prints with its line breaks intact — so it is
        // escaped here, where it is interpolated, rather than there, where
        // our own prose would be escaped with it (#135).
        PeerMessage::JoinRefused { reason } => {
            anyhow::bail!("{}", clispeak_text::plain(&reason))
        }
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
    if let Err(e) = crate::identity::set_device_name_in(&shared.config_dir, name) {
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

    let target = {
        let Some(roster) = spaces.get(&space) else {
            return Response::error(format!("no device named '{name}' in this space"));
        };
        match pick_device(
            roster
                .members()
                .map(|m| (m.name.as_str(), m.endpoint_id.as_str())),
            name,
        ) {
            Ok(id) => id,
            Err(message) => return Response::error(message),
        }
    };
    if target == me {
        return Response::error("that is this device — use `clispeak leave` instead");
    }

    if let Some(roster) = spaces.get_mut(&space) {
        roster.revoke(&target);
    }
    if let Err(e) = spaces.save(&shared.spaces_path) {
        return Response::error(e.to_string());
    }
    Response::Revoked {
        name: name.to_string(),
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
async fn leave(
    shared: &Arc<Shared>,
    transport: &Arc<dyn Network>,
    space: Option<&str>,
) -> Response {
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
    if text.chars().count() > clispeak_text::MAX_CHUNK {
        chunks.extend(clispeak_text::chunk(&text));
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
    // Before the message below quotes it back. A name is peer-visible text
    // that ends up in other devices' rosters and in errors printed to an
    // agent, and the CLI no longer escapes a finished error message (#135).
    if name.chars().any(char::is_control) {
        return Some("a device name cannot contain control characters".into());
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
    Ticket::forget_in(&shared.config_dir);
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
        // The last two come from the other device — a live label in the join
        // reply, and the ticket's copy — so this is where a peer's string
        // becomes one of this device's own labels, and the only place that
        // happens without going through `set_label`. A control character in
        // one would end up quoted back inside errors the CLI prints with
        // their line breaks intact (#135).
        if candidate.is_empty()
            || candidate.contains('/')
            || candidate.contains(',')
            || candidate.chars().any(char::is_control)
        {
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
            .map(|s| clispeak_proto::SpaceRow {
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
    transport: &Arc<dyn Network>,
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
    send.finish();
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
    let mut revoked = Vec::new();
    for id in ordered {
        let Some(roster) = spaces.get(&id) else {
            continue;
        };
        // Gathered alongside the members rather than instead of them. A
        // revoked device is not a member and must not appear in a listing
        // as though it were — but leaving it out of the *response* is what
        // made a space dissolving itself invisible from every tool (#166).
        for (endpoint_id, at) in roster.tombstones() {
            revoked.push(clispeak_proto::RevokedInfo {
                // A member record beside a tombstone is the contradiction
                // that sat in three rosters for three days. It should be
                // impossible now; it is reported rather than assumed away.
                still_a_member: roster.holds(&endpoint_id),
                endpoint_id,
                revoked_at: at,
                space: several.then(|| spaces.label(&id).to_string()),
            });
        }
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
    Response::Devices { devices, revoked }
}

/// Serve one peer connection.
async fn handle_peer(shared: &Arc<Shared>, conn: &dyn Wire) -> Result<()> {
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
                // through `clispeak_text` first, but a receiver that assumes
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
                let (status, detail) = if allowed {
                    apply_control(shared, &control)
                } else {
                    (Status::Rejected, None)
                };
                write_msg(&mut send, &PeerMessage::Report { status, detail }).await?;
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
        Ticket::forget_in(&shared.config_dir);
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
    Ticket::forget_in(&shared.config_dir);
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
/// Which device a `revoke` selector means, or why it means none.
///
/// Pure, because the interesting behaviour is entirely in the choosing and a
/// `Roster` inside a `Shared` inside a `Node` is a great deal of scaffolding
/// to stand up in order to assert a `match`.
///
/// **A name that matches two devices is refused, not guessed.**
/// `Roster::by_name` returns the first current member, so `revoke` used to be
/// a coin toss that reported success — and the case is ordinary rather than
/// exotic: re-pairing a phone after a rebuild leaves the old entry beside the
/// new one, both answering to the same label. Removing the device you had
/// just paired, and being told it worked, is the failure this project exists
/// to avoid.
///
/// **An endpoint id is accepted, and that matters as much as the refusal.**
/// The other half of a name clash is usually the *dead* device, and a dead
/// device cannot be renamed — so an escape hatch that says "rename one of
/// them" would leave a stale member that nothing can remove (#39).
fn pick_device<'a>(
    members: impl Iterator<Item = (&'a str, &'a str)>,
    selector: &str,
) -> Result<String, String> {
    let mut by_name = Vec::new();
    let mut by_id = Vec::new();
    for (name, id) in members {
        if name == selector {
            by_name.push(id);
        }
        // A prefix, because `clispeak devices` prints a short id and that is
        // what someone reading it will type back.
        if id.starts_with(selector) {
            by_id.push(id);
        }
    }

    // A name wins over an id prefix: a name is what a person meant to type,
    // and a selector cannot be both without somebody naming a device after a
    // key.
    match (by_name.as_slice(), by_id.as_slice()) {
        ([only], _) => Ok((*only).to_string()),
        (several @ [_, _, ..], _) => {
            // Across lines, which it could not be until #135: the CLI escaped
            // control characters in the whole of anything a node sent,
            // newlines included, so this arrived as one line with a literal
            // `\n` in it. One id per line because the next thing to do with
            // one is paste it back.
            let ids = several
                .iter()
                .map(|id| format!("\n  {}", short_id(id)))
                .collect::<String>();
            Err(format!(
                "more than one device is called '{selector}' in this space{ids}\n\
                 Revoke by id instead: clispeak revoke <id>"
            ))
        }
        ([], [only]) => Ok((*only).to_string()),
        ([], several @ [_, _, ..]) => Err(format!(
            "'{selector}' is a prefix of {} device ids here. Give more of it.",
            several.len()
        )),
        ([], []) => Err(format!("no device named '{selector}' in this space")),
    }
}

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

    use super::{MAX_MESSAGE_CHARS, accept_chunk, adopt_own_name, name_objection};

    /// Issue #147: a device that disagrees with itself about its own name.
    ///
    /// Seen on a phone whose settings screen said one thing and whose space
    /// list said another, seconds apart, with the laptop agreeing with the
    /// settings screen. The cause was never reproduced; what is fixed here is
    /// that the disagreement no longer survives a restart, in any space, and
    /// no longer passes unremarked.
    #[test]
    fn this_device_is_called_the_same_thing_in_every_space() {
        use crate::spaces::Spaces;
        use iroh_base::SecretKey;

        let secret = SecretKey::from_bytes(&[7; 32]);
        let host = SecretKey::from_bytes(&[8; 32]);
        let me = secret.public().to_string();

        let mut spaces = Spaces::default();
        spaces.insert(crate::Roster::found(&secret, "Android phone"), "main");
        // A second space, joined rather than founded — which is the ordinary
        // way a device ends up in two, and the one the old code left alone
        // because it reconciled the current space and nothing else.
        let mut joined = crate::Roster::found(&host, "Laptop");
        joined.invite(&host, &me, "Android phone");
        spaces.insert(joined, "work");
        assert_eq!(spaces.ids().len(), 2, "two spaces, one device");

        assert!(
            adopt_own_name(&mut spaces, &me, "Phone"),
            "a disagreement is a change"
        );
        for id in spaces.ids() {
            assert_eq!(
                spaces.get(&id).and_then(|r| r.name_of(&me)),
                Some("Phone"),
                "every space, not just the default one"
            );
        }

        assert!(
            !adopt_own_name(&mut spaces, &me, "Phone"),
            "agreeing already is not a change, and must not force a save"
        );
    }

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
                c.chars().count() <= clispeak_text::MAX_CHUNK,
                "every piece is speakable: {} chars",
                c.chars().count()
            );
        }
    }

    #[test]
    fn revoke_refuses_a_name_that_means_two_devices() {
        let members = || {
            [
                ("Phone", "aaaa1111aaaa1111aaaa"),
                ("Phone", "bbbb2222bbbb2222bbbb"),
                ("Laptop", "cccc3333cccc3333cccc"),
            ]
            .into_iter()
        };

        // The case that made this worth fixing: a phone re-paired after a
        // rebuild, sitting beside the entry it replaced. `by_name` returned
        // whichever came first and reported success (#39).
        let e = super::pick_device(members(), "Phone").expect_err("should refuse");
        assert!(e.contains("more than one"), "{e}");
        assert!(
            e.contains("aaaa1111aaaa1111") && e.contains("bbbb2222bbbb2222"),
            "it must name both candidates: {e}"
        );
        // It says it across lines now, which is what #135 was: the CLI
        // escaped control characters in the whole of anything a node sent,
        // so this arrived as one line with a literal backslash-n in it.
        assert_eq!(e.lines().count(), 4, "an id per line, then the fix: {e}");

        assert_eq!(
            super::pick_device(members(), "Laptop").expect("unambiguous"),
            "cccc3333cccc3333cccc"
        );

        // An id is accepted, whole or as the prefix `devices` prints — the
        // only way to remove the dead half of a name clash, because a dead
        // device cannot be renamed.
        assert_eq!(
            super::pick_device(members(), "bbbb2222bbbb2222").expect("by short id"),
            "bbbb2222bbbb2222bbbb"
        );
        assert_eq!(
            super::pick_device(members(), "aaaa1111aaaa1111aaaa").expect("by whole id"),
            "aaaa1111aaaa1111aaaa"
        );

        // A prefix that cannot separate them says so rather than choosing.
        let e = super::pick_device([("A", "ff11"), ("B", "ff22")].into_iter(), "ff")
            .expect_err("ambiguous prefix");
        assert!(e.contains("prefix"), "{e}");

        assert!(super::pick_device(members(), "nobody").is_err());
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
        let block = "a".repeat(clispeak_text::MAX_CHUNK);
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
        format!("clispeak-test-{}-{label}.sock", std::process::id())
    }

    /// A squatter is not a node, and is not reported as one.
    ///
    /// The half of #128 that needed no redesign. Any local user can take the
    /// socket name — the abstract namespace has no owner on Linux, and on
    /// macOS `interprocess` uses the world-writable temporary directory — and
    /// both callers treated "something answered" as "a node is running". So a
    /// squatter made the node refuse to start with *"another node is already
    /// running"*, which was false, and made the app decline to start its own.
    ///
    /// The handshake could always tell them apart. It was never asked.
    #[tokio::test]
    async fn something_that_cannot_prove_the_token_is_not_our_node() {
        use crate::ipc::{Listening, install_token, who_is_listening};

        let socket = unique("squatter");
        let dir = std::env::temp_dir().join(format!("clispeak-squat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        install_token(&dir).expect("a token to check against");

        // Nothing is listening yet.
        assert_eq!(
            who_is_listening(&socket, &dir).await,
            Listening::Nothing,
            "an unheld name is free"
        );

        // Exactly what a squatter is: something holding the name that has
        // never seen the token. A bare listener answers connections and can
        // produce no proof.
        let _squatter = interprocess::local_socket::ListenerOptions::new()
            // The same resolver the node uses, so the squatter takes
            // the place a node would actually look rather than a name that
            // merely resembles it.
            //
            // Deliberately *not* `ipc::listener_for`: a squatter is somebody
            // else's process and gets no help from our access rules. Using
            // them here would be a test of the node against itself.
            .name(crate::ipc::socket_target(&socket).expect("name"))
            .create_tokio()
            .expect("taking the name first");

        match who_is_listening(&socket, &dir).await {
            Listening::Stranger(why) => assert!(
                !why.is_empty(),
                "a refusal has to carry the reason it refused"
            ),
            other => panic!("a squatter must not read as a node: {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
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
        assert!(message.contains("already held"), "{message}");
        // And it no longer asserts *what* holds it. By the time `bind_ipc`
        // runs, `serve` has replaced the token, so a node that started a
        // moment ago and a local process that took the name are genuinely
        // indistinguishable from here — `who_is_listening` is what tells
        // them apart, before the token is replaced (#128).
        assert!(
            message.contains("another local process"),
            "the message must not claim to know which: {message}"
        );
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
/// Two real nodes, in one process, talking to each other.
///
/// **What changed to make this possible.** Every one of these paths reached a
/// concrete `iroh::endpoint::Connection`, obtainable only by binding an
/// endpoint and having a real device dial it — so the protocol could not be
/// driven by a test at all, and every fix to it said "verified by reading"
/// (#80). Two things were in the way and both are gone: the connection is now
/// a [`Wire`], and a node carries its own config directory instead of reading
/// a process-wide `OnceLock` that gave the second node the first one's
/// identity, roster and outstanding invite.
///
/// The tests below drive the real `serve_peers`, the real `do_join`, the real
/// `send_to_peer` and the real `sync_roster`. What they replace is a
/// `Transport`, and nothing else.
#[cfg(test)]
mod peer_tests {
    use super::*;
    use crate::loopback::Switchboard;
    use crate::transport::{Network, read_msg, write_msg};
    use clispeak_engine::{EngineError, Tier, Voice};

    /// A scratch directory that removes itself, however the test ends.
    ///
    /// On drop rather than at the end of the body: a failing assertion is a
    /// panic, and the tidy-up line after it never runs. The old version left
    /// a directory behind for every failure, which is a slow leak in `/tmp`
    /// that nothing reports.
    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// An engine that speaks into a `Vec`, so a test can read what arrived.
    ///
    /// `SilentEngine` refuses everything, which is right for a device with no
    /// speech and useless for asking whether a message crossed. The
    /// distinction matters: a receiver that reports `no_engine` has still
    /// authorised, queued and accepted the message, so a test using it cannot
    /// tell delivery from refusal.
    #[derive(Default)]
    struct Recorder {
        said: std::sync::Mutex<Vec<String>>,
    }

    impl Recorder {
        fn heard(&self) -> Vec<String> {
            self.said.lock().expect("said").clone()
        }
    }

    impl SpeechEngine for Recorder {
        fn ready(&self) -> Result<(), EngineError> {
            Ok(())
        }

        fn speak(&self, chunk: &str) -> Result<(), EngineError> {
            self.said.lock().expect("said").push(chunk.to_string());
            Ok(())
        }

        fn voices(&self) -> Vec<Voice> {
            vec![Voice {
                id: "test".into(),
                name: "Test".into(),
            }]
        }

        fn stop(&self) {}

        fn tier(&self) -> Tier {
            Tier::Full
        }
    }

    /// A device: its own directory, its own identity, its own place on the
    /// board, and a background task serving peers.
    struct Device {
        node: Arc<Node>,
        engine: Arc<Recorder>,
        serving: tokio::task::JoinHandle<()>,
        _scratch: Scratch,
    }

    impl Device {
        /// This device's public key, as the roster spells it.
        fn id(&self) -> String {
            self.node.id()
        }

        /// Everything this device's engine was asked to say.
        fn heard(&self) -> Vec<String> {
            self.engine.heard()
        }

        /// The membership of this device's current space, as names.
        async fn roster_names(&self) -> Vec<String> {
            let spaces = self.node.shared.spaces.lock().await;
            let mut names: Vec<String> =
                spaces.current().members().map(|m| m.name.clone()).collect();
            names.sort();
            names
        }

        async fn shut_down(self) {
            self.serving.abort();
            self.node.close().await;
        }
    }

    /// A counter, so two devices in one test never share a directory.
    fn unique_dir(label: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::env::temp_dir().join(format!("clispeak-{label}-{}-{n}", std::process::id()))
    }

    /// Build a device and start serving peers on it.
    async fn device(board: &Switchboard, name: &str) -> Device {
        let dir = unique_dir(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");

        let store = crate::identity::FileKeyStore::at(dir.join("identity.key"));
        let identity = crate::identity::Identity::load_or_create(&store).expect("identity");
        let net = board.endpoint(identity.id());
        let engine = Arc::new(Recorder::default());
        let node = Arc::new(
            Node::new_in(
                dir.clone(),
                Arc::clone(&engine) as Arc<dyn SpeechEngine>,
                identity,
                net,
                name.to_string(),
            )
            .await
            .expect("node"),
        );

        let serving = {
            let node = Arc::clone(&node);
            tokio::spawn(async move {
                let _ = node.serve_peers().await;
            })
        };

        Device {
            node,
            engine,
            serving,
            _scratch: Scratch(dir),
        }
    }

    /// Pair two devices the way a person does: one invites, the other joins.
    ///
    /// Driven through the public calls rather than by assembling a roster, so
    /// what is under test is the join, not a reconstruction of it.
    async fn pair(host: &Device, joiner: &Device) {
        pair_in(host, joiner, None).await;
    }

    /// The same, into a space the host names.
    async fn pair_in(host: &Device, joiner: &Device, space: Option<&str>) {
        let url = match host.node.invite(space).await {
            Response::Invite { url, .. } => url,
            other => panic!("invite did not produce one: {other:?}"),
        };
        match joiner.node.join(&url, None).await {
            Response::Joined { .. } => {}
            other => panic!("join failed: {other:?}"),
        }
    }

    /// The headline: two devices pair, and both hold the other.
    ///
    /// The old version of this test faked the joiner — it wrote a
    /// `JoinRequest` down a pipe with a freshly generated key, called
    /// `handle_peer` directly, and then ran `Roster::adopt` by hand over the
    /// reply. That checked the host's half twice and the joiner's half never,
    /// because both sides of the comparison were the test's own code.
    ///
    /// This runs `do_join` on a second real node. A break between the two
    /// halves is now a failure here rather than something a person meets
    /// while pairing a phone.
    #[tokio::test(flavor = "multi_thread")]
    async fn two_devices_pair_and_each_holds_the_other() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;
        let phone = device(&board, "Phone").await;

        pair(&laptop, &phone).await;

        assert_eq!(
            laptop.roster_names().await,
            vec!["Laptop".to_string(), "Phone".to_string()],
            "the inviting device has to record the member it just vouched for"
        );
        assert_eq!(
            phone.roster_names().await,
            vec!["Laptop".to_string(), "Phone".to_string()],
            "a join that leaves the joiner with an empty roster is not a join (#145)"
        );

        laptop.shut_down().await;
        phone.shut_down().await;
    }

    /// A message crosses, and the far device is the one that says it.
    ///
    /// Nothing above the unit level had ever asserted this. Every spoken
    /// message in the project's history was verified by a person hearing it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_message_reaches_the_other_device_and_is_spoken_there() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;
        let phone = device(&board, "Phone").await;
        pair(&laptop, &phone).await;

        let sent = laptop
            .node
            .speak(
                "the build finished".into(),
                Priority::Normal,
                Some("Phone".into()),
            )
            .await;
        // `Queued` rather than `Spoken`: the far device answers when it has
        // taken the message, not when it has finished saying it. Asserting
        // `Spoken` here would be asserting on a race, which is what `settle`
        // below is for.
        match &sent {
            Response::Report { targets, .. } => {
                assert_eq!(targets.len(), 1, "one addressee, one result: {sent:?}");
                assert!(
                    matches!(targets[0].status, Status::Queued | Status::Spoken),
                    "the paired device refused the message: {sent:?}"
                );
                assert_eq!(targets[0].device, "Phone");
            }
            other => panic!("sending to a paired device should report on it: {other:?}"),
        }

        settle(|| !phone.heard().is_empty()).await;
        assert_eq!(
            phone.heard(),
            vec!["the build finished".to_string()],
            "the receiving device speaks what was sent"
        );
        assert!(
            laptop.heard().is_empty(),
            "a message addressed to one device must not also be said here"
        );

        laptop.shut_down().await;
        phone.shut_down().await;
    }

    /// An unpaired device cannot make this one speak.
    ///
    /// Authorisation is the roster of the space the message was sent in, and
    /// the check existed and had never been executed. Driven from a bare
    /// endpoint rather than a second node, because a stranger is exactly a
    /// device that never joined.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stranger_cannot_make_this_device_speak() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;

        let stranger_key = iroh::SecretKey::generate();
        let stranger = board.endpoint(stranger_key.public());
        let conn = stranger
            .connect(laptop.node.transport.id())
            .await
            .expect("dialling");
        let (mut send, mut recv) = conn.open_bi().await.expect("a stream");

        write_msg(
            &mut send,
            &PeerMessage::SpeakBegin {
                msg_id: "m1".into(),
                priority: Priority::Normal,
                wait: false,
                voice: None,
                space: None,
                timeout_secs: None,
            },
        )
        .await
        .expect("writing the header");

        // The rest is offered and may not be taken, and that is the correct
        // behaviour rather than a tolerance: a device that is not a member is
        // refused on the header alone, and the handler moves to the next
        // stream without ever reading the payload. Draining an unauthorised
        // peer's message first would be the bug — it is unbounded input from
        // someone with no standing to send it.
        //
        // So these writes race a stream the node has already finished with.
        // Insisting they succeed is what made this test pass on a laptop and
        // fail on CI: locally the pipe buffer swallowed them before the far
        // end was dropped, and on a slower machine it did not. The reply is
        // the assertion; the writes are not.
        let _ = write_msg(
            &mut send,
            &PeerMessage::Chunk {
                seq: 0,
                text: "say something".into(),
            },
        )
        .await;
        let _ = write_msg(&mut send, &PeerMessage::SpeakEnd).await;

        match read_msg(&mut recv).await.expect("a reply") {
            PeerMessage::Report { status, .. } => assert!(
                !matches!(status, Status::Spoken | Status::Queued),
                "a stranger's message was accepted: {status:?}"
            ),
            other => panic!("unexpected reply: {other:?}"),
        }
        assert!(
            laptop.heard().is_empty(),
            "nothing a stranger sent may be spoken"
        );

        laptop.shut_down().await;
    }

    /// Issue #52, driven rather than read.
    ///
    /// A join request is signed for whoever is on the other end of the
    /// connection, not for whoever the message names. Where those differ is
    /// the whole attack: a ticket holder enrolling a *third* key it does not
    /// hold, leaving a member that revoking the device in front of you does
    /// not remove.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_join_naming_another_device_is_refused() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;

        let dialer_key = iroh::SecretKey::generate();
        let dialer = board.endpoint(dialer_key.public());
        let someone_else = iroh::SecretKey::generate().public();

        let conn = dialer
            .connect(laptop.node.transport.id())
            .await
            .expect("dialling");
        let (mut send, mut recv) = conn.open_bi().await.expect("a stream");
        write_msg(
            &mut send,
            &PeerMessage::JoinRequest {
                endpoint_id: someone_else.to_string(),
                display_name: "Impostor".into(),
                token: "irrelevant".into(),
            },
        )
        .await
        .expect("writing the request");

        match read_msg(&mut recv).await.expect("a reply") {
            PeerMessage::JoinRefused { reason } => assert!(
                reason.contains("different device"),
                "refused for the wrong reason: {reason}"
            ),
            other => {
                panic!("a join naming a device other than the dialer was not refused: {other:?}")
            }
        }

        laptop.shut_down().await;
    }

    /// Issue #62, and the bug a person found on real hardware in September.
    ///
    /// A rename has to reach the other device. It travels on the roster sync
    /// that piggybacks the next message, so this sends one and then asks the
    /// far device what it calls this one.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_rename_reaches_the_other_device() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;
        let phone = device(&board, "Phone").await;
        pair(&laptop, &phone).await;

        match laptop.node.rename("Workstation").await {
            Response::Renamed { .. } => {}
            other => panic!("rename failed: {other:?}"),
        }

        // Contact is what carries it: the sync rides the next message rather
        // than a background heartbeat, which is the design and is also why
        // the rename appeared not to propagate until something else happened.
        let _ = laptop
            .node
            .speak("anything".into(), Priority::Normal, Some("Phone".into()))
            .await;

        settle(|| !phone.heard().is_empty()).await;
        assert_eq!(
            phone.roster_names().await,
            vec!["Phone".to_string(), "Workstation".to_string()],
            "the far device still calls this one by its old name"
        );

        laptop.shut_down().await;
        phone.shut_down().await;
    }

    /// Issue #166: a refused sync must cost one device, not the space.
    ///
    /// A peer answering as though we are not a member used to mint a
    /// tombstone, and tombstones travel — so a single refusal removed that
    /// device everywhere, and two devices refusing each other in the same
    /// minute took the whole space apart in thirty seconds, founder included.
    /// The fix was to forget rather than revoke. This is that, driven: a
    /// stranger claiming not to know us leaves the rest of the space intact.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_peer_that_denies_us_does_not_dissolve_the_space() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;
        let phone = device(&board, "Phone").await;
        let tablet = device(&board, "Tablet").await;
        pair(&laptop, &phone).await;
        pair(&laptop, &tablet).await;

        assert_eq!(
            laptop.roster_names().await.len(),
            3,
            "all three should be in one space before the interesting part"
        );

        // The phone leaves, which is the honest version of "does not know
        // us": it announces its departure and stops answering.
        match phone.node.leave(None).await {
            Response::Left { .. } => {}
            other => panic!("leave failed: {other:?}"),
        }
        phone.shut_down().await;

        // The laptop reaches for everyone. The phone is gone; the tablet is
        // not, and is what must survive.
        // `all`, not `None`. A bare send goes to this device only — there is
        // deliberately no selector meaning "every device everywhere", and
        // `all` is scoped to one space.
        let sent = laptop
            .node
            .speak("still here".into(), Priority::Normal, Some("all".into()))
            .await;
        assert!(
            matches!(sent, Response::Report { .. }),
            "a send to the whole space should report per device: {sent:?}"
        );
        settle(|| !tablet.heard().is_empty()).await;

        assert!(
            laptop.roster_names().await.contains(&"Tablet".to_string()),
            "one departing device must not take the others with it (#166)"
        );
        assert_eq!(
            tablet.heard(),
            vec!["still here".to_string()],
            "the device that never left has to still receive"
        );

        laptop.shut_down().await;
        tablet.shut_down().await;
    }

    // --------------------------------------------------- refusing a join
    //
    // Every branch of `accept_join` that says no. #80 listed these as driven
    // by nothing, and two of them are fixes for closed issues whose code has
    // never been executed by a test — #50 in particular, where an invite
    // outliving a rotation could admit someone to the very space the rotation
    // existed to protect.

    /// Send a join request over a raw wire and return whatever comes back.
    ///
    /// A bare endpoint rather than a second node, because these are all cases
    /// where the *asking* device is doing something a well-behaved joiner
    /// would not.
    async fn ask_to_join(board: &Switchboard, host: &Device, token: &str) -> PeerMessage {
        let key = iroh::SecretKey::generate();
        let caller = board.endpoint(key.public());
        let conn = caller
            .connect(host.node.transport.id())
            .await
            .expect("dialling");
        let (mut send, mut recv) = conn.open_bi().await.expect("a stream");
        write_msg(
            &mut send,
            &PeerMessage::JoinRequest {
                endpoint_id: key.public().to_string(),
                display_name: "Caller".into(),
                token: token.into(),
            },
        )
        .await
        .expect("writing the request");
        read_msg(&mut recv).await.expect("a reply")
    }

    /// The reason must be written for the joiner, who is the one reading it.
    fn refusal(reply: PeerMessage) -> String {
        match reply {
            PeerMessage::JoinRefused { reason } => reason,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_join_with_no_invite_open_is_refused() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;

        let reason = refusal(ask_to_join(&board, &laptop, "anything").await);
        assert!(
            reason.contains("inviting device"),
            "the reason is read on the *joining* device, so it has to point \
             at the other one: {reason}"
        );
        assert_eq!(
            laptop.roster_names().await,
            vec!["Laptop".to_string()],
            "nobody was admitted"
        );

        laptop.shut_down().await;
    }

    /// A wrong token must not burn the invite.
    ///
    /// It is a single-use secret, and consuming it on a failed guess would
    /// let anyone who can reach the socket cancel a pairing that is in
    /// progress — a denial of service with no credential at all.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_wrong_token_is_refused_and_leaves_the_invite_usable() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;
        let phone = device(&board, "Phone").await;

        let url = match laptop.node.invite(None).await {
            Response::Invite { url, .. } => url,
            other => panic!("invite did not produce one: {other:?}"),
        };

        let reason = refusal(ask_to_join(&board, &laptop, "not-the-token").await);
        assert!(
            reason.contains("not valid"),
            "a wrong token is refused: {reason}"
        );

        // The real joiner still gets in, which is the whole point.
        match phone.node.join(&url, None).await {
            Response::Joined { .. } => {}
            other => panic!("a wrong guess consumed the invite: {other:?}"),
        }
        assert!(
            laptop.roster_names().await.contains(&"Phone".to_string()),
            "the invited device has to be admitted after a failed guess"
        );

        laptop.shut_down().await;
        phone.shut_down().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_expired_invite_is_refused_and_thrown_away() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;

        // Placed directly: minting one and waiting five minutes is not a
        // test, and the branch under examination is what happens when the
        // clock has already passed `expires_at`.
        let stale = Ticket {
            endpoint_id: laptop.id(),
            token: "stale".into(),
            expires_at: now_secs().saturating_sub(1),
            space: None,
            label: None,
        };
        *laptop.node.shared.pending.lock().await = Some(stale);

        let reason = refusal(ask_to_join(&board, &laptop, "stale").await);
        assert!(
            reason.contains("expired"),
            "say it expired rather than that it is invalid — those send the \
             reader to different places: {reason}"
        );
        assert!(
            laptop.node.shared.pending.lock().await.is_none(),
            "a dead ticket is cleared rather than left to be re-read on every \
             later attempt"
        );

        laptop.shut_down().await;
    }

    /// Issue #50, driven for the first time.
    ///
    /// A ticket names the space it was minted for. When that space is gone,
    /// falling back to the default admits the joiner to *a* space — and after
    /// a rotation the default is the new space the rotation existed to
    /// protect, so the device being locked out walks straight back in.
    ///
    /// Driven directly rather than through `rotate`, and that is worth saying:
    /// `rotate` and `leave_space` both cancel any open invite first, so this
    /// branch is a second line of defence behind that one. Testing it means
    /// putting the node in the state the first line is there to prevent.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_invite_for_a_space_that_is_gone_admits_nobody() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;

        let orphan = Ticket {
            endpoint_id: laptop.id(),
            token: "orphan".into(),
            expires_at: now_secs() + 300,
            space: Some("a-space-this-device-does-not-hold".into()),
            label: Some("work".into()),
        };
        *laptop.node.shared.pending.lock().await = Some(orphan);

        let reason = refusal(ask_to_join(&board, &laptop, "orphan").await);
        assert!(
            reason.contains("no longer on this device"),
            "the joiner needs to know the invite is stale rather than wrong: {reason}"
        );
        assert_eq!(
            laptop.roster_names().await,
            vec!["Laptop".to_string()],
            "an invite for a space that is gone must admit nobody anywhere — \
             falling back to the default is the whole of #50"
        );

        laptop.shut_down().await;
    }

    /// Single use: a ticket seen over a shoulder, or left in scrollback,
    /// cannot be replayed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_used_invite_cannot_be_used_again() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;
        let phone = device(&board, "Phone").await;

        let url = match laptop.node.invite(None).await {
            Response::Invite { url, .. } => url,
            other => panic!("invite did not produce one: {other:?}"),
        };
        let token = Ticket::parse(&url).expect("a minted ticket").token;
        match phone.node.join(&url, None).await {
            Response::Joined { .. } => {}
            other => panic!("the first join should succeed: {other:?}"),
        }

        // The same token, from a device that was never invited.
        let reason = refusal(ask_to_join(&board, &laptop, &token).await);
        assert!(
            reason.contains("no invite open") || reason.contains("inviting device"),
            "a replayed ticket is refused because the invite is spent: {reason}"
        );
        assert_eq!(
            laptop.roster_names().await,
            vec!["Laptop".to_string(), "Phone".to_string()],
            "the replay admitted a third device"
        );

        laptop.shut_down().await;
        phone.shut_down().await;
    }

    /// Issue #51, driven for the first time.
    ///
    /// A peer that names a space is answered about that space or not at all.
    /// The fallback is for peers too old to name one, and letting it fire for
    /// a space we no longer hold merged one space's membership into another:
    /// a device that left `work` had every work device land in its `home`
    /// roster, after which they could speak to it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_message_naming_a_space_we_have_left_is_refused() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;
        let desk = device(&board, "Desk").await;

        // The desk is in both spaces, which is what makes the fallback
        // dangerous: with it, a message about the space we left would be
        // answered about the space we kept.
        pair(&laptop, &desk).await;
        laptop.node.new_space("work").await;
        pair_in(&laptop, &desk, Some("work")).await;
        laptop.node.default_space("main").await;

        let work_id = {
            let spaces = laptop.node.shared.spaces.lock().await;
            spaces.by_label("work").expect("the work space")
        };
        match laptop.node.leave_space("work").await {
            Response::Spaces { .. } => {}
            other => panic!("leave_space failed: {other:?}"),
        }

        let conn = desk
            .node
            .transport
            .connect(laptop.node.transport.id())
            .await
            .expect("dialling");
        let (mut send, mut recv) = conn.open_bi().await.expect("a stream");
        write_msg(
            &mut send,
            &PeerMessage::SpeakBegin {
                msg_id: "m1".into(),
                priority: Priority::Normal,
                wait: false,
                voice: None,
                space: Some(work_id),
                timeout_secs: None,
            },
        )
        .await
        .expect("writing the header");
        // The whole message, not just the header — and this is not tidiness.
        //
        // A refused header is answered immediately and the rest is never
        // read, so sending only the header passes. But if the refusal ever
        // stops happening, the node accepts and then waits for chunks that
        // never arrive while this waits for a reply that never comes: the
        // test *hangs* instead of failing, and a hang in CI is a timeout with
        // no name on it. Confirmed by reintroducing the fallback: with only
        // the header it hung, and with the whole message it fails and says
        // which status it got.
        //
        // Write errors are ignored for the reason the stranger test explains:
        // being refused mid-message is correct, and insisting on the writes
        // makes the test depend on losing that race.
        let _ = write_msg(
            &mut send,
            &PeerMessage::Chunk {
                seq: 0,
                text: "work business".into(),
            },
        )
        .await;
        let _ = write_msg(&mut send, &PeerMessage::SpeakEnd).await;

        match read_msg(&mut recv).await.expect("a reply") {
            PeerMessage::Report { status, detail } => {
                assert!(
                    !matches!(status, Status::Spoken | Status::Queued),
                    "a message about a space this device left was accepted: \
                     {status:?} {detail:?}"
                );
            }
            other => panic!("unexpected reply: {other:?}"),
        }
        assert!(
            laptop.heard().is_empty(),
            "nothing about a space we left may be spoken"
        );
        assert_eq!(
            laptop.roster_names().await,
            vec!["Desk".to_string(), "Laptop".to_string()],
            "the space we kept must be exactly as it was"
        );

        laptop.shut_down().await;
        desk.shut_down().await;
    }

    // ------------------------------------------------------------ resolve
    //
    // `resolve` is the most intricate function in the crate and #80 recorded
    // it as having no tests at all — only `push_target` and `also_answers_to`,
    // the two helpers it calls. Every selector rule below is a rule somebody
    // wrote down in a comment and nothing executed.
    //
    // These build real spaces by pairing real devices rather than assembling
    // a `Spaces` by hand, so what is under test is the resolver over state the
    // rest of the system actually produces.

    /// The device names a selector resolved to, in order, for readable
    /// assertions. `Here` is reported under this device's own label.
    async fn resolved(of: &Device, selector: &str) -> Result<Vec<String>, String> {
        let mine = of.node.name();
        resolve(&of.node.shared, selector).await.map(|targets| {
            targets
                .into_iter()
                .map(|t| match t {
                    Target::Here { .. } => mine.clone(),
                    Target::Peer { name, .. } => name,
                })
                .collect()
        })
    }

    /// A laptop in two spaces: `main` with a phone, `work` with a desk.
    async fn two_spaces(board: &Switchboard) -> (Device, Device, Device) {
        let laptop = device(board, "Laptop").await;
        let phone = device(board, "Phone").await;
        let desk = device(board, "Desk").await;

        pair(&laptop, &phone).await;
        match laptop.node.new_space("work").await {
            Response::Spaces { .. } => {}
            other => panic!("new_space failed: {other:?}"),
        }
        pair_in(&laptop, &desk, Some("work")).await;
        // Creating a space makes it the default, so say which one this
        // fixture means rather than inheriting whichever was made last. Every
        // assertion below about a *bare* selector is an assertion about the
        // default, and leaving that implicit is how a test ends up describing
        // the fixture instead of the rule.
        match laptop.node.default_space("main").await {
            Response::Spaces { .. } => {}
            other => panic!("default_space failed: {other:?}"),
        }
        (laptop, phone, desk)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn here_and_a_bare_name_reach_what_they_say() {
        let board = Switchboard::new();
        let (laptop, phone, _desk) = two_spaces(&board).await;

        assert_eq!(resolved(&laptop, "here").await, Ok(vec!["Laptop".into()]));
        assert_eq!(resolved(&laptop, "Phone").await, Ok(vec!["Phone".into()]));

        laptop.shut_down().await;
        phone.shut_down().await;
        _desk.shut_down().await;
    }

    /// `all` is scoped to one space, and there is deliberately no selector
    /// meaning "every device everywhere" — a work message arriving on the
    /// family tablet is what separate spaces exist to prevent.
    #[tokio::test(flavor = "multi_thread")]
    async fn all_is_one_space_and_never_every_space() {
        let board = Switchboard::new();
        let (laptop, phone, desk) = two_spaces(&board).await;

        let mut here = resolved(&laptop, "all").await.expect("all resolves");
        here.sort();
        assert_eq!(
            here,
            vec!["Laptop".to_string(), "Phone".to_string()],
            "bare `all` is the default space only"
        );

        let mut work = resolved(&laptop, "work/all").await.expect("work/all");
        work.sort();
        assert_eq!(
            work,
            vec!["Desk".to_string(), "Laptop".to_string()],
            "a qualified `all` must include this device, or `work/all` reaches \
             nothing on a machine whose default has moved on"
        );

        laptop.shut_down().await;
        phone.shut_down().await;
        desk.shut_down().await;
    }

    /// `--to all,pixel` must not make the phone say it twice.
    #[tokio::test(flavor = "multi_thread")]
    async fn duplicates_collapse() {
        let board = Switchboard::new();
        let (laptop, phone, desk) = two_spaces(&board).await;

        let got = resolved(&laptop, "all,Phone,here,Phone").await.expect("ok");
        assert_eq!(
            got.len(),
            2,
            "one device per addressee however many times it was named: {got:?}"
        );

        laptop.shut_down().await;
        phone.shut_down().await;
        desk.shut_down().await;
    }

    /// A name only in a non-default space still resolves, unqualified.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_name_unique_outside_the_default_space_still_resolves() {
        let board = Switchboard::new();
        let (laptop, phone, desk) = two_spaces(&board).await;

        assert_eq!(resolved(&laptop, "Desk").await, Ok(vec!["Desk".into()]));
        assert_eq!(
            resolved(&laptop, "work/Desk").await,
            Ok(vec!["Desk".into()]),
            "qualifying a name that already resolved must not change it"
        );

        laptop.shut_down().await;
        phone.shut_down().await;
        desk.shut_down().await;
    }

    /// An unknown name is an error naming every name that *is* known.
    ///
    /// Partial delivery from a typo is the failure worth preventing: reaching
    /// two devices out of three looks like it worked.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unknown_name_lists_the_known_ones() {
        let board = Switchboard::new();
        let (laptop, phone, desk) = two_spaces(&board).await;

        let err = resolved(&laptop, "Pixel").await.expect_err("should refuse");
        assert!(
            err.contains("Pixel"),
            "the error must name what failed: {err}"
        );
        assert!(
            err.contains("Phone") && err.contains("Desk"),
            "an agent needs the alternatives to correct itself: {err}"
        );

        // And nothing partial: one bad element refuses the whole selector.
        assert!(
            resolved(&laptop, "Phone,Pixel").await.is_err(),
            "a typo beside a good name must not deliver to the good one"
        );

        laptop.shut_down().await;
        phone.shut_down().await;
        desk.shut_down().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unknown_space_lists_the_known_ones() {
        let board = Switchboard::new();
        let (laptop, phone, desk) = two_spaces(&board).await;

        let err = resolved(&laptop, "home/Desk")
            .await
            .expect_err("should refuse");
        assert!(err.contains("home"), "name the space that failed: {err}");
        assert!(err.contains("work"), "and the ones that exist: {err}");

        laptop.shut_down().await;
        phone.shut_down().await;
        desk.shut_down().await;
    }

    /// Issue #39, the half that had no test.
    ///
    /// Two devices sharing a name *inside one space* were told to "Qualify it:
    /// work/twin  or  work/twin" — the same command twice, and the one that
    /// had just failed. An agent following that suggestion loops forever, and
    /// neither device is addressable by any selector this resolver accepts.
    #[tokio::test(flavor = "multi_thread")]
    async fn two_devices_with_one_name_in_one_space_are_not_told_to_qualify() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;
        let twin_a = device(&board, "Twin").await;
        let twin_b = device(&board, "Twin").await;
        pair(&laptop, &twin_a).await;
        pair(&laptop, &twin_b).await;

        let err = resolved(&laptop, "Twin").await.expect_err("ambiguous");
        assert!(
            err.contains("same space"),
            "the message has to say why qualifying cannot help: {err}"
        );
        assert!(
            !err.contains('/'),
            "suggesting a qualified selector here is advice that cannot work, \
             and an agent will loop on it: {err}"
        );
        assert!(
            err.contains("rename"),
            "the only way out is renaming one of them, so say so: {err}"
        );

        laptop.shut_down().await;
        twin_a.shut_down().await;
        twin_b.shut_down().await;
    }

    /// A name in two *different* spaces can be separated, and is.
    ///
    /// The default space has to hold no `TV` for this to arise at all: a name
    /// present in the default wins outright and is never ambiguous, which is
    /// the next test. So the ambiguity is between two *other* spaces, which is
    /// exactly the case qualifying was invented for.
    #[tokio::test(flavor = "multi_thread")]
    async fn one_name_in_two_spaces_asks_to_be_qualified() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;
        let phone = device(&board, "Phone").await;
        let home_tv = device(&board, "TV").await;
        let work_tv = device(&board, "TV").await;

        pair(&laptop, &phone).await;
        laptop.node.new_space("work").await;
        pair_in(&laptop, &work_tv, Some("work")).await;
        laptop.node.new_space("home").await;
        pair_in(&laptop, &home_tv, Some("home")).await;
        laptop.node.default_space("main").await;

        let err = resolved(&laptop, "TV").await.expect_err("ambiguous");
        assert!(
            err.contains('/'),
            "here qualifying *does* separate them, so it must be offered: {err}"
        );
        assert_eq!(
            resolved(&laptop, "work/TV").await,
            Ok(vec!["TV".into()]),
            "and the advice it gave has to actually work"
        );

        laptop.shut_down().await;
        phone.shut_down().await;
        home_tv.shut_down().await;
        work_tv.shut_down().await;
    }

    /// The default space wins outright, so setting a default decides
    /// something.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_default_space_wins_a_shared_name_without_an_error() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;
        let home_tv = device(&board, "TV").await;
        let work_tv = device(&board, "TV").await;

        pair(&laptop, &home_tv).await;
        laptop.node.new_space("work").await;
        pair_in(&laptop, &work_tv, Some("work")).await;

        // With `work` as the default, a bare `TV` is no longer ambiguous.
        match laptop.node.default_space("work").await {
            Response::Spaces { .. } => {}
            other => panic!("default_space failed: {other:?}"),
        }
        let got = resolved(&laptop, "TV").await.expect("no longer ambiguous");
        assert_eq!(got, vec!["TV".to_string()]);
        assert_eq!(
            resolve(&laptop.node.shared, "TV").await.unwrap().len(),
            1,
            "the default space decides it rather than reaching both"
        );

        laptop.shut_down().await;
        home_tv.shut_down().await;
        work_tv.shut_down().await;
    }

    /// Issue #39, the other half: this device's own label wins, and the peers
    /// it beat are named rather than silently dropped.
    ///
    /// A one-row report saying "spoken" reads as a clean send whether or not a
    /// second machine answered to the name, which is why the shadowed ids are
    /// carried out of the resolver at all.
    #[tokio::test(flavor = "multi_thread")]
    async fn our_own_name_wins_but_the_peers_it_beat_are_reported() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;
        let impostor = device(&board, "Laptop").await;
        pair(&laptop, &impostor).await;

        let targets = resolve(&laptop.node.shared, "Laptop")
            .await
            .expect("our own name always resolves");
        match targets.as_slice() {
            [Target::Here { shadowed }] => {
                assert_eq!(
                    shadowed.len(),
                    1,
                    "the peer answering to the same name has to be carried out \
                     of the resolver, or the send reads as clean"
                );
                assert!(
                    also_answers_to(shadowed).is_some_and(|s| s.contains("not sent to")),
                    "and it has to reach the report in words"
                );
            }
            other => panic!("our own name should resolve to this device alone: {other:?}"),
        }

        // `here` is unambiguous by construction and reports nothing.
        match resolve(&laptop.node.shared, "here")
            .await
            .unwrap()
            .as_slice()
        {
            [Target::Here { shadowed }] => assert!(
                shadowed.is_empty(),
                "`here` names no one else, so it shadows no one"
            ),
            other => panic!("here should be this device: {other:?}"),
        }

        laptop.shut_down().await;
        impostor.shut_down().await;
    }

    /// A quiet window that certainly contains this moment, whatever time the
    /// suite runs at.
    ///
    /// `00:00-23:59` is the obvious spelling and it is wrong for one minute a
    /// day: the window is half-open, so 23:59 itself falls outside it. Two
    /// hours from now covers the run however long it takes, and wraps
    /// correctly past midnight because a window whose end is before its start
    /// is read as crossing midnight.
    fn quiet_from_now() -> (String, String) {
        let start = policy::local_minute();
        (
            policy::format_time(start),
            policy::format_time((start + 120) % 1440),
        )
    }

    /// #246. `high` is the entire mechanism for reaching someone through a
    /// quiet window, `status` advertises it, and it never worked.
    ///
    /// Policy is checked twice — once when the message is accepted and again
    /// when it is about to be spoken (#77) — and the second check was handed
    /// a hardcoded `Priority::Normal`. So a high message passed the first
    /// gate, queued, and was refused by the second one for being something it
    /// was not.
    ///
    /// This has to go through the queue to fail. `Policy::verdict` is correct
    /// and its own tests pass, because they call it directly with the
    /// priority the caller meant. What was wrong was the call site, and
    /// nothing exercised it.
    #[tokio::test(flavor = "multi_thread")]
    async fn high_breaks_through_a_quiet_window_at_the_moment_of_speaking() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;
        let (from, to) = quiet_from_now();
        laptop
            .node
            .set_quiet(Some(from), Some(to), true, None)
            .await;

        laptop
            .node
            .speak("the roof is on fire".into(), Priority::High, None)
            .await;
        settle(|| !laptop.heard().is_empty()).await;
        assert_eq!(
            laptop.heard(),
            vec!["the roof is on fire".to_string()],
            "high breaks through, so it has to actually come out of the speaker"
        );

        laptop.shut_down().await;
    }

    /// The other half: the window still applies to everything else.
    ///
    /// Without this, "let it through" and "let everything through" look
    /// identical from the test suite, and the fix for #246 could have been
    /// deleting the second check entirely — which would put #77 back.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_normal_message_is_still_refused_inside_the_window() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;
        let (from, to) = quiet_from_now();
        laptop
            .node
            .set_quiet(Some(from), Some(to), true, None)
            .await;

        let response = laptop
            .node
            .speak("nothing important".into(), Priority::Normal, None)
            .await;
        match response {
            Response::Report { targets, .. } => assert_eq!(
                targets.first().map(|t| t.status.clone()),
                Some(Status::QuietHours),
                "a normal message inside the window is refused, and told why"
            ),
            other => panic!("speaking here should report: {other:?}"),
        }
        assert!(
            laptop.heard().is_empty(),
            "and nothing comes out of the speaker"
        );

        laptop.shut_down().await;
    }

    /// Pressing play on a muted device has to speak.
    ///
    /// `replay` skips the submit-time policy check deliberately, and says so:
    /// mute and quiet hours exist to stop a device making noise *unasked*,
    /// and pressing play is the ask. Then #77 added a second check at the
    /// moment of speaking, which every job passes through — so the bypass
    /// stopped working and the comment describing it went on being read as
    /// true. Found while fixing #246, in the same three lines.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_replay_speaks_on_a_muted_device() {
        let board = Switchboard::new();
        let laptop = device(&board, "Laptop").await;
        laptop.node.set_mute(true, None).await;

        // Muted, so this is recorded and not spoken — which is the whole
        // reason someone would go back to the history and press play.
        let msg_id = match laptop
            .node
            .speak("read this back to me".into(), Priority::Normal, None)
            .await
        {
            Response::Report { msg_id, .. } => msg_id,
            other => panic!("speaking here should report: {other:?}"),
        };
        assert!(laptop.heard().is_empty(), "a muted device says nothing");

        laptop.node.replay(&msg_id);
        settle(|| !laptop.heard().is_empty()).await;
        assert_eq!(
            laptop.heard(),
            vec!["read this back to me".to_string()],
            "the person asked for this one directly"
        );

        laptop.shut_down().await;
    }

    /// Wait for a condition, or give up loudly.
    ///
    /// Delivery is asynchronous — the sender is answered when the message is
    /// accepted, not when it has been said — so a test that asserts
    /// immediately is asserting on a race. A fixed `sleep` would be the other
    /// way to lose: too short is flaky, too long is a suite nobody runs.
    ///
    /// Two seconds is far past anything in-memory needs and is still a bound,
    /// so a genuine hang fails rather than hanging the suite.
    async fn settle(mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if done() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("waited two seconds and it never happened");
    }
}
