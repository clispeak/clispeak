//! The voicecast node, wrapped as an app.
//!
//! Deliberately thin. Everything durable — transport, roster, queue, playback
//! — lives in `voicecast-core`, exactly as it does for the CLI. This crate
//! owns the window, picks the platform engine, and exposes a handful of
//! commands the UI calls.
//!
//! On mobile the app *is* the node: there is no `voicecastd` on a phone, so
//! the long-running process has to be the app itself.

use std::sync::Arc;

use serde::Serialize;
use tauri::{Manager, State};
// Menus and trays are desktop-only in Tauri; a phone has neither.
#[cfg(desktop)]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use voicecast_core::{Identity, Node, Transport};
use voicecast_engine::SpeechEngine;
use voicecast_proto::{DeviceInfo, Priority, Response};

/// What the UI shows in its header.
#[derive(Serialize)]
pub struct NodeStatus {
    /// This device's local label.
    pub name: String,
    /// Its public key.
    pub device_id: String,
    /// Engine currently in use.
    pub engine: String,
    /// Whether that engine is a stand-in for the intended one.
    pub fallback: bool,
    /// Why the engine cannot speak, when it cannot.
    ///
    /// The pill alone cannot carry this: "unavailable" says a device is
    /// silent without saying what to do about it, and the sentence that
    /// names the fault was reaching only whoever sent a message.
    pub reason: Option<String>,
    /// Whether the node is still coming up. Transient, and worth saying so.
    pub starting: bool,
    /// Why the node is not running at all.
    ///
    /// Distinct from [`reason`], which is about an engine inside a node that
    /// *is* running. This one means there is no node: a locked keyring, an
    /// unwritable config directory, or another node already holding the
    /// socket. It used to go to stderr and the window said "starting…" until
    /// it was closed. Issue #72.
    ///
    /// [`reason`]: NodeStatus::reason
    pub failed: Option<String>,
}

impl Default for NodeStatus {
    fn default() -> Self {
        Self {
            name: String::new(),
            device_id: String::new(),
            engine: "unknown".into(),
            fallback: true,
            reason: None,
            starting: false,
            failed: None,
        }
    }
}

/// Where the command-line tool should live on the host.
///
/// `~/.local/bin` on every desktop: it needs no privileges, which is what
/// keeps installing the app a drag-and-drop rather than a password prompt.
///
/// On Linux it is on the default PATH. On macOS it is **not** — the default
/// there comes from `/etc/paths`, which lists `/usr/local/bin` and nothing
/// under a home directory. Writing to `/usr/local/bin` needs an administrator,
/// so the app installs where it can and reports whether the result is
/// actually reachable; see [`cli_on_path`].
fn cli_destination() -> Option<std::path::PathBuf> {
    Some(
        directories::BaseDirs::new()?
            .home_dir()
            .join(".local/bin")
            .join(CLI_NAME),
    )
}

/// The command-line tool's file name, which carries an extension on Windows.
///
/// Without it the install writes a file called `voicecast` that Windows will
/// not execute: present on disk, in a directory that is on the PATH, and
/// still `command not found`. That is the macOS failure this module already
/// guards against, arriving through a different mechanism — and it would be
/// found by an agent rather than by whoever installed the app.
const CLI_NAME: &str = if cfg!(windows) {
    "voicecast.exe"
} else {
    "voicecast"
};

/// The copy of the CLI the app carries, if this build carries one.
///
/// Two packages do. A Flatpak keeps it in the sandbox at `/app/libexec`,
/// unreachable from the host. A macOS `.app` keeps it in `Contents/MacOS`
/// beside the app binary, where it works but is not on anyone's PATH.
/// Either way the copy has to be put somewhere a shell will find it.
///
/// A plain `cargo run`, or a distribution package that installed the tool
/// itself, carries nothing and needs nothing.
fn bundled_cli() -> Option<std::path::PathBuf> {
    let flatpak = std::path::PathBuf::from("/app/libexec/voicecast");
    if flatpak.exists() {
        return Some(flatpak);
    }
    if cfg!(target_os = "macos") {
        // `Contents/MacOS/voicecast-app` sits beside `Contents/MacOS/voicecast`,
        // which is where Tauri puts a bundled sidecar binary.
        let exe = std::env::current_exe().ok()?;
        let beside = exe.parent()?.join("voicecast");
        if beside.exists() && beside != exe {
            return Some(beside);
        }
    }
    None
}

/// Whether the command-line tool is available to agents on this machine.
#[tauri::command]
fn cli_status() -> Option<String> {
    bundled_cli()?;
    let dest = cli_destination()?;
    dest.exists().then(|| dest.display().to_string())
}

/// Whether offering to install the CLI makes sense here.
#[tauri::command]
fn cli_installable() -> bool {
    bundled_cli().is_some() && cli_status().is_none()
}

/// Where the CLI is expected to be, whether or not it is there yet.
///
/// Reported so the interface can say what went wrong when the automatic
/// install failed, rather than leaving a missing `voicecast` unexplained.
#[tauri::command]
fn cli_expected_path() -> Option<String> {
    bundled_cli()?;
    cli_destination().map(|p| p.display().to_string())
}

/// Whether the directory the CLI is installed into is on the PATH.
///
/// The install can succeed and still leave `voicecast` unusable, which on
/// macOS is the normal case rather than an edge one: `~/.local/bin` is not on
/// the default PATH there. An agent would report "command not found" and
/// nothing would explain why, so the app says so itself.
///
/// Read from this process's own PATH, which for an app launched from Finder
/// is the bare system default — deliberately the pessimistic reading, since
/// that is the environment a GUI-launched agent would inherit.
#[tauri::command]
fn cli_on_path() -> bool {
    let Some(dest) = cli_destination() else {
        return false;
    };
    let Some(dir) = dest.parent() else {
        return false;
    };
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|p| p == dir))
        .unwrap_or(false)
}

/// Shell start-up files that might already put the CLI's directory on PATH.
///
/// Checked before writing anything, so a line someone wrote themselves is
/// never duplicated — including the one this app may have written before.
#[cfg(target_os = "macos")]
const PROFILES: &[&str] = &[
    ".zshenv",
    ".zprofile",
    ".zshrc",
    ".profile",
    ".bash_profile",
    ".bashrc",
];

/// Where the line goes when nothing supplies it already.
///
/// `.zprofile` because zsh is the login shell on every macOS since Catalina,
/// and because it is read *after* `/etc/zprofile` runs `path_helper` — which
/// rebuilds PATH from `/etc/paths` and would otherwise reorder the entry away.
#[cfg(target_os = "macos")]
const PROFILE: &str = ".zprofile";

/// Put the CLI's directory on the PATH for future shells.
///
/// Installing the tool is not enough on macOS. The default PATH is built from
/// `/etc/paths`, which names no home directory, so `~/.local/bin/voicecast`
/// exists and no shell can see it — and an agent, which is the whole reason
/// the tool is installed, reports `command not found` with nothing to explain
/// it. The alternative destination that *is* on the default PATH,
/// `/usr/local/bin`, is root-owned and would turn installing this app into a
/// password prompt.
///
/// Linux is left alone: `~/.local/bin` is on the default PATH there already.
///
/// Only ever appends, and only once. It cannot affect a shell that is already
/// open, so [`cli_on_path`] still reports the truth for this session.
#[cfg(target_os = "macos")]
fn ensure_on_path(home: &std::path::Path, dir: &std::path::Path) -> Result<bool, String> {
    // Matching on the text rather than on a marker comment: what matters is
    // whether a shell will end up with this directory on its PATH, however
    // it was written, not whether this app is the one that wrote it.
    let needle = dir.strip_prefix(home).map_or_else(
        |_| dir.display().to_string(),
        |rest| format!("/{}", rest.display()),
    );
    for profile in PROFILES {
        if let Ok(text) = std::fs::read_to_string(home.join(profile))
            && text.contains(&needle)
        {
            return Ok(false);
        }
    }

    let path = home.join(PROFILE);
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!(
        "\n# Added by voicecast, so the `voicecast` command is on the PATH for\n\
         # shells and for the agents that call it.\n\
         export PATH=\"$HOME{needle}:$PATH\"\n"
    ));
    std::fs::write(&path, text).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(true)
}

/// Whether the host copy of the CLI needs writing.
///
/// Compared by content rather than a version string: the two must speak the
/// same protocol, and "different bytes" is exactly the question. Reading a
/// few megabytes once at startup is cheaper than diagnosing a mismatch.
/// Desktop only, like its caller. Android and iOS have no host to install
/// onto and no shell to run it from — and without this the function is
/// compiled there, called nowhere, and dead code is an error under the
/// workspace's `-D warnings` (#69).
#[cfg(desktop)]
fn cli_needs_install() -> bool {
    let Some(dest) = cli_destination() else {
        return false;
    };
    // Nothing to install from: not a package that carries the tool.
    let Some(source) = bundled_cli() else {
        return false;
    };
    let Ok(bundled) = std::fs::read(source) else {
        return false;
    };
    match std::fs::read(&dest) {
        Ok(installed) => bundled != installed,
        // Absent, or unreadable and therefore not usable as it stands.
        Err(_) => true,
    }
}

/// Put the command-line tool on the host PATH and keep it there in step.
///
/// Runs on every launch, and installs rather than only refreshing: the tool
/// is how an agent reaches this node, so an app that merely offers it leaves
/// `voicecast` missing from the PATH until someone finds the button.
///
/// A Flatpak update replaces the app but cannot touch a file already copied
/// to the host, so the same pass rewrites a stale copy. Without that the two
/// drift apart and the CLI ends up talking a protocol the node no longer
/// speaks — a failure that looks like a bug rather than a stale install.
/// Desktop only, like its caller. Android and iOS have no host to install
/// onto and no shell to run it from — and without this the function is
/// compiled there, called nowhere, and dead code is an error under the
/// workspace's `-D warnings` (#69).
#[cfg(desktop)]
fn install_cli_on_host() {
    if cli_needs_install() {
        match install_cli() {
            Ok(path) => eprintln!("installed the command-line tool at {path}"),
            Err(e) => eprintln!("could not install the command-line tool: {e}"),
        }
    }

    // Separate from the copy above, and not conditional on it: the tool can
    // already be installed and up to date while still being unreachable,
    // which is what happens on every launch after the first.
    #[cfg(target_os = "macos")]
    if bundled_cli().is_some()
        && let Some(dir) = cli_destination()
            .as_deref()
            .and_then(std::path::Path::parent)
    {
        let home = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf());
        match home
            .ok_or_else(|| "no home directory".to_string())
            .and_then(|home| ensure_on_path(&home, dir))
        {
            Ok(true) => eprintln!(
                "added {} to the PATH in ~/{PROFILE}; open a new terminal to pick it up",
                dir.display()
            ),
            Ok(false) => {}
            Err(e) => eprintln!("could not put {} on the PATH: {e}", dir.display()),
        }
    }
}

/// Copy the bundled CLI onto the host PATH.
///
/// Deliberately *not* exported as a Flatpak command: entering the sandbox
/// costs around 86ms per invocation against the tool's own 3ms, and it is
/// called repeatedly by an agent. A plain binary on the host keeps that fast,
/// and the two still reach each other over an abstract socket.
#[tauri::command]
fn install_cli() -> Result<String, String> {
    let source =
        bundled_cli().ok_or("this build does not carry a copy of the command-line tool")?;
    let dest = cli_destination().ok_or("no home directory")?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    std::fs::copy(source, &dest).map_err(|e| format!("could not install: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }
    Ok(dest.display().to_string())
}

/// Whether this device will speak, and when.
#[derive(Serialize)]
pub struct PolicyView {
    /// Silenced indefinitely.
    pub muted: bool,
    /// Quiet window start as `HH:MM`, if one is set.
    pub from: Option<String>,
    /// Quiet window end as `HH:MM`, if one is set.
    pub to: Option<String>,
    /// Whether `high` may break through the window.
    pub high_breaks_through: bool,
    /// Per-space restrictions layered on top of the above.
    ///
    /// Only spaces that restrict something appear, so the interface can ask
    /// "does this space differ" of the list itself.
    pub spaces: Vec<SpacePolicyView>,
}

/// One space's extra restrictions, named as the person sees it.
#[derive(Serialize)]
pub struct SpacePolicyView {
    /// This device's own name for the space.
    pub label: String,
    /// Silenced indefinitely, for this space alone.
    pub muted: bool,
    /// Quiet window start as `HH:MM`, if this space sets one.
    pub from: Option<String>,
    /// Quiet window end as `HH:MM`, if this space sets one.
    pub to: Option<String>,
    /// Whether `high` may break through *this space's* window.
    pub high_breaks_through: bool,
}

impl TryFrom<Response> for PolicyView {
    type Error = String;

    fn try_from(r: Response) -> Result<Self, String> {
        match r {
            Response::Policy {
                muted,
                quiet_from,
                quiet_to,
                high_breaks_through,
                spaces,
            } => Ok(Self {
                muted,
                from: quiet_from,
                to: quiet_to,
                high_breaks_through,
                spaces: spaces
                    .into_iter()
                    .map(|s| SpacePolicyView {
                        label: s.label,
                        muted: s.muted,
                        from: s.quiet_from,
                        to: s.quiet_to,
                        high_breaks_through: s.high_breaks_through,
                    })
                    .collect(),
            }),
            // The old comment here said reporting "not muted" would be "a lie
            // the interface then shows as settled state" — and then returned
            // exactly that, because there was nowhere to put a failure. A
            // corrupt or unreadable policy file therefore drew a device that
            // was unmuted with no quiet window, which is a claim about
            // settings nobody could read (#73).
            Response::Error { message, .. } => Err(message),
            other => Err(format!("unexpected reply asking for the policy: {other:?}")),
        }
    }
}

/// This device's speaking policy, and any per-space overrides.
#[tauri::command]
async fn policy(state: State<'_, AppState>) -> Result<PolicyView, String> {
    state.node.policy().await.try_into()
}

/// Silence this device, or one space on it, or let it speak again.
#[tauri::command]
async fn set_mute(
    state: State<'_, AppState>,
    muted: bool,
    space: Option<String>,
) -> Result<PolicyView, String> {
    // Every reply, error included, goes through one conversion now, so a
    // failure here cannot become a view of settings nobody could read (#73).
    state
        .node
        .set_mute(muted, space.as_deref())
        .await
        .try_into()
}

/// Set or clear a daily quiet window, device-wide or for one space.
#[tauri::command]
async fn set_quiet(
    state: State<'_, AppState>,
    from: Option<String>,
    to: Option<String>,
    high_breaks_through: bool,
    space: Option<String>,
) -> Result<PolicyView, String> {
    state
        .node
        .set_quiet(from, to, high_breaks_through, space.as_deref())
        .await
        .try_into()
}

/// How this device's voice is configured.
#[derive(Serialize)]
pub struct VoiceConfig {
    /// Every voice this engine offers.
    pub available: Vec<VoiceOption>,
    /// The one in use.
    pub current: Option<String>,
    /// Speaking rate, where 1.0 is normal.
    pub rate: f32,
}

/// One selectable voice.
#[derive(Serialize)]
pub struct VoiceOption {
    /// Stable id, used when selecting.
    pub id: String,
    /// What to show a person.
    pub name: String,
}

/// An invite, ready to hand to another device.
#[derive(Serialize)]
pub struct Invite {
    /// The `voicecast://join/...` string.
    pub url: String,
    /// The same invite as a scannable SVG, or empty if it could not be drawn.
    pub qr: String,
    /// Seconds until it stops being accepted.
    pub expires_in: u64,
}

/// Handle to the running node, shared with every command.
pub struct AppState {
    node: Arc<Node>,
}

/// What became of the node this app wraps.
///
/// Managed before the node exists, so the interface can be told which of the
/// three states it is in. [`AppState`] is registered only once a node is
/// running, so every command that needs one fails with "state not managed"
/// until then — which the interface could only read as "still starting", and
/// so read for ever when the node had actually failed. Issue #72.
enum Startup {
    /// The node is still being built. Genuinely transient.
    Starting,
    /// It is up.
    Running(Arc<Node>),
    /// It is not, and this is why. A reason nobody can see is not a reason:
    /// the failure went to stderr, which for an app launched from Finder is
    /// nowhere at all.
    Failed(String),
}

/// Holds [`Startup`] so the interface can ask what became of the node.
pub struct StartupState(std::sync::Mutex<Startup>);

impl StartupState {
    fn set(&self, next: Startup) {
        *self.0.lock().expect("startup lock") = next;
    }
}

#[tauri::command]
fn node_status(state: State<'_, StartupState>) -> NodeStatus {
    status_of(&state.0.lock().expect("startup lock"))
}

/// Turn a startup state into what the interface should show.
///
/// Split from the command so the three branches can be tested; a
/// `tauri::State` cannot be built outside a running app, which is part of why
/// "starting for ever" was never caught.
fn status_of(startup: &Startup) -> NodeStatus {
    let node = match startup {
        // Still the honest answer, and now it is an answer rather than the
        // absence of one.
        Startup::Starting => {
            return NodeStatus {
                starting: true,
                ..NodeStatus::default()
            };
        }
        Startup::Failed(why) => {
            return NodeStatus {
                engine: "unavailable".into(),
                fallback: true,
                failed: Some(why.clone()),
                ..NodeStatus::default()
            };
        }
        Startup::Running(node) => Arc::clone(node),
    };

    match replies::status(node.status()) {
        Some(s) => NodeStatus {
            name: node.name().to_string(),
            device_id: s.device_id,
            engine: s.engine,
            fallback: s.fallback,
            reason: s.reason,
            ..NodeStatus::default()
        },
        None => NodeStatus {
            name: node.name().to_string(),
            device_id: node.id(),
            engine: "unknown".into(),
            fallback: true,
            ..NodeStatus::default()
        },
    }
}

#[tauri::command]
async fn list_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, String> {
    replies::devices(state.node.devices().await)
}

/// The agent skill, compiled in so it can never drift from this build.
const SKILL: &str = include_str!("../../../skills/voicecast/SKILL.md");

/// Whether this build runs inside a Flatpak.
///
/// The macOS bundle and a plain build are not sandboxed and can write
/// wherever the user says; a Flatpak can only write what its manifest grants.
fn sandboxed() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

/// Where Claude Code looks for skills. Only a default.
fn skill_default() -> Option<std::path::PathBuf> {
    Some(
        directories::BaseDirs::new()?
            .home_dir()
            .join(".claude/skills/voicecast/SKILL.md"),
    )
}

/// Where a previous install put it, so it can be kept in step.
fn skill_record() -> Option<std::path::PathBuf> {
    voicecast_core::config_dir()
        .ok()
        .map(|d| d.join("skill-destination"))
}

/// Whether this build may write to `path`.
///
/// Inside a Flatpak an unshared write **appears to succeed and never reaches
/// the host** — the app's home holds an overlay, not the real directory. So a
/// sandboxed app that offered to install anywhere would report success and
/// install nothing, which is the one outcome worse than refusing.
///
/// Only the default location is granted in the manifest. Anywhere else has to
/// go through the command-line tool, which runs as the user with no sandbox.
fn skill_writable(path: &std::path::Path) -> bool {
    if !sandboxed() {
        return true;
    }
    let Some(dirs) = directories::BaseDirs::new() else {
        return false;
    };
    path.starts_with(dirs.home_dir().join(".claude"))
}

/// What an agent skill install looks like from here.
#[derive(Serialize)]
pub struct SkillStatus {
    /// Where it would go, or where it went.
    pub path: String,
    /// Where it would go if nothing had been chosen — Claude Code's own
    /// directory. Sent so the interface can tell whether the path in front of
    /// the user is a choice or the default, which is the difference between
    /// offering to reset it and offering nothing.
    ///
    /// Nullable rather than required, and the reason is a failure mode rather
    /// than a shape. `skill_default` is `None` only where there is no home
    /// directory, and the first version reached for `?` — which would have
    /// returned no status at all and made the whole panel vanish, on exactly
    /// the machine where a user with a recorded path most needs to see it.
    /// A section that disappears rather than explaining itself is the bug
    /// this app has already shipped once (#109).
    pub default_path: Option<String>,
    /// "absent", "current" or "stale".
    pub state: String,
    /// Whether this build can write anywhere, or only the default.
    pub sandboxed: bool,
}

/// Compare an installed copy against this build, by content.
fn skill_state(path: &std::path::Path) -> &'static str {
    match std::fs::read_to_string(path) {
        Ok(text) if text == SKILL => "current",
        Ok(_) => "stale",
        Err(_) => "absent",
    }
}

/// Where the skill is or would be, and whether it is current.
#[tauri::command]
fn skill_status() -> Option<SkillStatus> {
    let path = skill_record()
        .and_then(|r| std::fs::read_to_string(r).ok())
        .map(|p| std::path::PathBuf::from(p.trim()))
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(skill_default)?;
    Some(SkillStatus {
        state: skill_state(&path).into(),
        path: path.display().to_string(),
        default_path: skill_default().map(|p| p.display().to_string()),
        sandboxed: sandboxed(),
    })
}

/// Write the skill where an agent will find it.
///
/// Refuses rather than pretending when the sandbox would swallow the write,
/// and says which command does work.
#[tauri::command]
fn install_skill(path: Option<String>) -> Result<String, String> {
    let destination = match path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => std::path::PathBuf::from(shellexpand_home(p)),
        None => skill_default().ok_or("no home directory on this system")?,
    };

    if !skill_writable(&destination) {
        return Err(format!(
            "this app is sandboxed and can only write to the default location. \
             Run this instead:  voicecast skill --install --path {}",
            destination.display()
        ));
    }

    if let Some(dir) = destination.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    std::fs::write(&destination, SKILL)
        .map_err(|e| format!("could not write {}: {e}", destination.display()))?;

    // Remembered so a later launch can keep it in step, and so the interface
    // reports on the copy that actually exists rather than the default.
    if let Some(record) = skill_record() {
        if let Some(dir) = record.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(record, destination.display().to_string());
    }
    Ok(destination.display().to_string())
}

/// Forget a chosen skill location, so the default applies again.
///
/// Only the *record* is removed. A file the user asked to be written
/// somewhere is theirs, and deleting it is a different act from changing
/// where the next install goes — one this button did not offer to do. So the
/// old copy stays, and the caller is handed its path to say so rather than
/// leaving a stale skill somewhere nobody is told about.
///
/// Resetting the field alone would not have worked: `skill_status` reads the
/// record, so the next poll would put the chosen path straight back and the
/// button would look broken for a reason nothing on screen explained.
#[tauri::command]
fn reset_skill_path() -> Result<Option<String>, String> {
    let default = skill_default().ok_or("no home directory on this system")?;
    let Some(record) = skill_record() else {
        return Ok(None);
    };
    let previous = std::fs::read_to_string(&record)
        .ok()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty() && std::path::Path::new(p) != default);
    match std::fs::remove_file(&record) {
        Ok(()) => Ok(previous),
        // Nothing recorded is already the state being asked for.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("could not forget the chosen path: {e}")),
    }
}

/// Expand a leading `~`, which people type and `PathBuf` does not understand.
fn shellexpand_home(path: &str) -> String {
    let Some(rest) = path.strip_prefix("~/") else {
        return path.to_string();
    };
    match directories::BaseDirs::new() {
        Some(dirs) => dirs.home_dir().join(rest).display().to_string(),
        None => path.to_string(),
    }
}

/// Keep an installed skill in step with the app.
///
/// Only ever rewrites a copy this app installed and recorded — it never
/// creates one, because putting a file into somebody's agent configuration is
/// something they should have asked for.
/// Desktop only, like its caller. Android and iOS have no host to install
/// onto and no shell to run it from — and without this the function is
/// compiled there, called nowhere, and dead code is an error under the
/// workspace's `-D warnings` (#69).
#[cfg(desktop)]
fn refresh_installed_skill() {
    let Some(record) = skill_record() else { return };
    let Ok(text) = std::fs::read_to_string(&record) else {
        return;
    };
    let path = std::path::PathBuf::from(text.trim());
    if path.as_os_str().is_empty() || skill_state(&path) != "stale" || !skill_writable(&path) {
        return;
    }
    match std::fs::write(&path, SKILL) {
        Ok(()) => eprintln!("updated the agent skill at {}", path.display()),
        Err(e) => eprintln!("could not update the agent skill: {e}"),
    }
}

/// What this device is saying right now.
#[derive(Serialize)]
pub struct NowPlaying {
    /// The message being spoken, if any.
    pub msg_id: Option<String>,
    /// Its text, so the controls say what they would stop.
    pub text: Option<String>,
    /// Which device it came from.
    pub from: Option<String>,
    /// Whether speech is held rather than finished.
    pub paused: bool,
    /// How many messages are waiting behind it.
    pub waiting: usize,
}

/// What this device is saying, for the playback controls.
#[tauri::command]
fn now_playing(state: State<'_, AppState>) -> NowPlaying {
    let (speaking, pending, paused) =
        replies::queue(state.node.queue_state()).unwrap_or((None, Vec::new(), false));
    // The text comes from the history rather than the queue: the queue holds
    // chunks mid-flight, and the history holds the message as it was sent.
    let entry = speaking.as_deref().and_then(|id| state.node.message(id));
    NowPlaying {
        msg_id: speaking,
        text: entry.as_ref().map(|e| e.text.clone()),
        from: entry.map(|e| e.from),
        paused,
        waiting: pending.len(),
    }
}

/// Hold what is being spoken here.
#[tauri::command]
fn pause_speech(state: State<'_, AppState>) {
    state.node.pause();
}

/// Start speaking here again.
#[tauri::command]
fn resume_speech(state: State<'_, AppState>) {
    state.node.unpause();
}

/// Stop what is being spoken here and drop the queue behind it.
#[tauri::command]
fn stop_speech(state: State<'_, AppState>) {
    state.node.stop();
}

/// Abandon the current message and move to the next.
#[tauri::command]
fn skip_speech(state: State<'_, AppState>) {
    state.node.skip();
}

/// Recent messages this device was asked to speak, spoken or not.
#[tauri::command]
fn history(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<voicecast_proto::HistoryEntry>, String> {
    replies::history(state.node.history(limit))
}

/// Speak a message from the history again.
///
/// Plays through mute and quiet hours: pressing play is the consent those
/// settings exist to require.
#[tauri::command]
fn replay(state: State<'_, AppState>, msg_id: String) -> Result<(), String> {
    replies::accepted(state.node.replay(&msg_id))
}

/// Forget the history.
#[tauri::command]
fn clear_history(state: State<'_, AppState>) -> Result<(), String> {
    replies::done(state.node.clear_history())
}

/// The spaces this device belongs to.
#[tauri::command]
async fn list_spaces(state: State<'_, AppState>) -> Result<Vec<voicecast_proto::SpaceRow>, String> {
    replies::spaces(state.node.spaces().await)
}

/// Found a new space from this device, and make it the default.
#[tauri::command]
async fn new_space(state: State<'_, AppState>, label: String) -> Result<(), String> {
    replies::spaces_changed(state.node.new_space(&label).await)
}

/// Drop one space, keeping the others.
///
/// Distinct from `leave_space`, which leaves the space this device is
/// currently using; this one names which of several to drop.
#[tauri::command]
async fn drop_space(state: State<'_, AppState>, label: String) -> Result<(), String> {
    replies::spaces_changed(state.node.leave_space(&label).await)
}

/// Choose which space bare device names resolve in.
#[tauri::command]
async fn default_space(state: State<'_, AppState>, label: String) -> Result<(), String> {
    replies::spaces_changed(state.node.default_space(&label).await)
}

/// Rename a space locally.
#[tauri::command]
async fn rename_space(state: State<'_, AppState>, label: String, to: String) -> Result<(), String> {
    replies::spaces_changed(state.node.rename_space(&label, &to).await)
}

/// Replace the default space, locking every other device out at once.
///
/// Returns the devices that were in it, so the interface can say who has to
/// be re-invited rather than leaving the user to remember.
#[tauri::command]
async fn rotate_space(
    state: State<'_, AppState>,
    space: Option<String>,
) -> Result<Vec<String>, String> {
    replies::rotated(state.node.rotate(space.as_deref()).await)
}

#[tauri::command]
async fn make_invite(state: State<'_, AppState>, space: Option<String>) -> Result<Invite, String> {
    replies::invite(state.node.invite(space.as_deref()).await)
}

/// What an invite would join, read before anything is committed to.
///
/// The destination is written into the ticket by whoever minted it, so the
/// joining device cannot choose it — the only honest thing to offer is to
/// read it out first. Local and side-effect free: no device is contacted and
/// the single-use token is not spent.
#[derive(Serialize)]
pub struct InvitePreview {
    /// The inviter's name for the space, absent on a ticket that predates
    /// labels travelling.
    label: Option<String>,
    /// Seconds until it stops being accepted.
    expires_in: u64,
    /// The inviting device's key, shortened, for comparing against its screen.
    from: String,
}

#[tauri::command]
fn preview_invite(state: State<'_, AppState>, ticket: String) -> Result<InvitePreview, String> {
    replies::preview(state.node.preview(&ticket))
}

#[tauri::command]
async fn join_space(
    state: State<'_, AppState>,
    ticket: String,
    label: Option<String>,
) -> Result<Joined, String> {
    replies::joined(state.node.join(&ticket, label).await)
}

/// The result of a join, including what the space ended up being called.
///
/// The name is returned because it can differ from the one asked for — a
/// clash with a space already held here falls back — and a person told "you
/// joined work" who then cannot find `work` has been misled.
#[derive(Serialize)]
pub struct Joined {
    members: usize,
    space: String,
}

/// Whether this device will keep receiving while asleep.
///
/// `true` on desktop, which has no equivalent restriction — the UI only warns
/// where the warning means something.
#[tauri::command]
fn battery_ok() -> bool {
    #[cfg(target_os = "android")]
    {
        voicecast_engine::is_battery_exempt()
    }
    #[cfg(not(target_os = "android"))]
    {
        true
    }
}

/// Ask Android to stop optimising this app.
#[tauri::command]
fn request_battery_exemption() -> bool {
    #[cfg(target_os = "android")]
    {
        voicecast_engine::request_battery_exemption()
    }
    #[cfg(not(target_os = "android"))]
    {
        true
    }
}

#[tauri::command]
async fn rename_device(state: State<'_, AppState>, name: String) -> Result<String, String> {
    replies::renamed(state.node.rename(&name).await)
}

/// An invite opened from a QR scan, if one is waiting.
///
/// Polled by the interface rather than pushed, because the scan can land
/// before the webview exists.
#[tauri::command]
fn pending_invite() -> Option<String> {
    #[cfg(target_os = "android")]
    {
        voicecast_engine::take_pending_invite()
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

#[tauri::command]
fn voice_config(state: State<'_, AppState>) -> VoiceConfig {
    let engine = state.node.engine();
    VoiceConfig {
        available: engine
            .voices()
            .into_iter()
            .map(|v| VoiceOption {
                id: v.id,
                name: v.name,
            })
            .collect(),
        current: engine.current_voice(),
        rate: engine.rate(),
    }
}

#[tauri::command]
fn set_voice(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let engine = state.node.engine();
    engine.set_voice(&id).map_err(|e| e.to_string())?;
    // Remembered immediately: a preference that needs a clean shutdown to
    // stick is one that will sometimes not.
    let _ = voicecast_core::save_voice_settings(&id, engine.rate());
    Ok(())
}

#[tauri::command]
fn set_rate(state: State<'_, AppState>, rate: f32) -> Result<(), String> {
    let engine = state.node.engine();
    engine.set_rate(rate).map_err(|e| e.to_string())?;
    if let Some(voice) = engine.current_voice() {
        let _ = voicecast_core::save_voice_settings(&voice, rate);
    }
    Ok(())
}

#[tauri::command]
async fn revoke_device(
    state: State<'_, AppState>,
    name: String,
    space: Option<String>,
) -> Result<String, String> {
    replies::renamed(state.node.revoke(&name, space.as_deref()).await)
}

#[tauri::command]
async fn leave_space(state: State<'_, AppState>, space: Option<String>) -> Result<String, String> {
    replies::left(state.node.leave(space.as_deref()).await)
}

#[tauri::command]
async fn speak(state: State<'_, AppState>, text: String) -> Result<(), String> {
    // Validated here rather than in the UI so the app and the CLI reject
    // exactly the same things.
    if let Err(rejection) = voicecast_text::validate(&text) {
        return Err(rejection.to_string());
    }
    replies::spoke(state.node.speak(text, Priority::Normal, None).await)
}

/// Whether a report counts as the message being taken, and why if not.
///
/// The app speaks on this device only, so there is one target — but the
/// answer is written over all of them, because "some device refused" is the
/// question, not "the first one did".
///
/// A refusal is surfaced rather than swallowed. Muting is a decision worth
/// reporting back: pressing Speak on a muted device and being told nothing
/// looks like the button is broken.
fn spoken_or_why(targets: &[voicecast_proto::TargetResult]) -> Result<(), String> {
    use voicecast_proto::Status;
    let heard = |s: &Status| matches!(s, Status::Spoken | Status::Queued | Status::Speaking);
    if targets.iter().any(|t| heard(&t.status)) {
        return Ok(());
    }
    let why = targets
        .iter()
        .map(|t| match (&t.status, t.detail.as_deref()) {
            (Status::Muted, _) => "this device is muted".to_string(),
            (Status::QuietHours, _) => "quiet hours are on".to_string(),
            (Status::NoEngine, Some(d)) => d.to_string(),
            (Status::NoEngine, None) => "no working speech engine".to_string(),
            (s, Some(d)) => format!("{s:?}: {d}"),
            (s, None) => format!("{s:?}"),
        })
        .collect::<Vec<_>>()
        .join("; ");
    Err(if why.is_empty() {
        "nothing was spoken".into()
    } else {
        why
    })
}

/// One place where a reply's shape is turned into what a command returns.
///
/// Every command used to carry its own `match` on [`Response`], each ending
/// `other => Err(describe(other))`. That catch-all is right — an unhandled
/// reply should say so — but it turns a compile-time question into a runtime
/// one, and it has been wrong three times: the leave reply changed shape and
/// the app showed `unexpected response: Left { … }`; `speak` began reporting
/// per device and the app announced a failure *after speaking the message*;
/// a device that refused produced no message at all. Each compiled, each was
/// green, each was found by a person pressing a button (#46).
///
/// Pulling the matches out here does not make the node's reply a compile-time
/// promise — that is issue #79's seam, and a bigger change. What it does is
/// make the pairing something a test can execute. `command_tests` below drives
/// a real node through *these* functions, so a reply that changes shape fails
/// the build rather than waiting for someone to press the button.
mod replies {
    use super::{DeviceInfo, Invite, InvitePreview, Joined, describe, spoken_or_why};
    use voicecast_proto::Response;

    /// The fields `node_status` reads. `None` when the reply was not a status
    /// at all, which the caller draws as an unknown engine rather than an
    /// error — the status panel has to render something.
    pub(super) struct Status {
        pub device_id: String,
        pub engine: String,
        pub fallback: bool,
        pub reason: Option<String>,
    }

    pub(super) fn status(r: Response) -> Option<Status> {
        match r {
            Response::Status {
                device_id,
                engine,
                fallback,
                engine_reason,
                ..
            } => Some(Status {
                device_id,
                engine,
                fallback,
                reason: engine_reason,
            }),
            _ => None,
        }
    }

    /// What is being said and what is behind it. `None` for the same reason
    /// as [`status`]: the playback controls always draw.
    #[allow(clippy::type_complexity)]
    pub(super) fn queue(r: Response) -> Option<(Option<String>, Vec<String>, bool)> {
        match r {
            Response::Queue {
                speaking,
                pending,
                paused,
            } => Some((speaking, pending, paused)),
            _ => None,
        }
    }

    pub(super) fn devices(r: Response) -> Result<Vec<DeviceInfo>, String> {
        match r {
            Response::Devices { devices } => Ok(devices),
            other => Err(describe(other)),
        }
    }

    pub(super) fn history(r: Response) -> Result<Vec<voicecast_proto::HistoryEntry>, String> {
        match r {
            Response::History { entries } => Ok(entries),
            other => Err(describe(other)),
        }
    }

    pub(super) fn accepted(r: Response) -> Result<(), String> {
        match r {
            Response::Accepted { .. } => Ok(()),
            other => Err(describe(other)),
        }
    }

    pub(super) fn done(r: Response) -> Result<(), String> {
        match r {
            Response::Done => Ok(()),
            other => Err(describe(other)),
        }
    }

    pub(super) fn spaces(r: Response) -> Result<Vec<voicecast_proto::SpaceRow>, String> {
        match r {
            Response::Spaces { spaces } => Ok(spaces),
            other => Err(describe(other)),
        }
    }

    /// The four commands that change the set of spaces and show the result by
    /// refreshing rather than by reading the reply.
    pub(super) fn spaces_changed(r: Response) -> Result<(), String> {
        spaces(r).map(|_| ())
    }

    pub(super) fn rotated(r: Response) -> Result<Vec<String>, String> {
        match r {
            Response::Rotated { devices, .. } => Ok(devices),
            other => Err(describe(other)),
        }
    }

    pub(super) fn invite(r: Response) -> Result<Invite, String> {
        match r {
            Response::Invite { url, expires_in } => {
                // A missing QR is a degraded invite, not a failed one: the
                // code can still be copied.
                let qr = voicecast_core::qr_svg(&url).unwrap_or_default();
                Ok(Invite {
                    url,
                    qr,
                    expires_in,
                })
            }
            other => Err(describe(other)),
        }
    }

    pub(super) fn preview(r: Response) -> Result<InvitePreview, String> {
        match r {
            Response::Preview {
                label,
                expires_in,
                endpoint_id,
            } => Ok(InvitePreview {
                label,
                expires_in,
                from: endpoint_id.chars().take(16).collect(),
            }),
            other => Err(describe(other)),
        }
    }

    pub(super) fn joined(r: Response) -> Result<Joined, String> {
        match r {
            Response::Joined { members, space } => Ok(Joined { members, space }),
            other => Err(describe(other)),
        }
    }

    pub(super) fn renamed(r: Response) -> Result<String, String> {
        match r {
            Response::Renamed { name } => Ok(name),
            other => Err(describe(other)),
        }
    }

    pub(super) fn left(r: Response) -> Result<String, String> {
        match r {
            Response::Left {
                space,
                unreached,
                refounded,
            } => {
                let mut said = format!("left {space}");
                if refounded {
                    said.push_str(" — it was the only one, so an empty space took its place");
                }
                if unreached > 0 {
                    said.push_str(&format!("; {unreached} device(s) not reached yet"));
                }
                Ok(said)
            }
            other => Err(describe(other)),
        }
    }

    /// `speak` reports per device, as the CLI has always seen it. This matched
    /// only `Accepted` and `Finished`, so the node queued the message, spoke
    /// it, and the interface then said the send had failed — showing the debug
    /// formatting of a successful report as an error. Work done, failure
    /// announced.
    pub(super) fn spoke(r: Response) -> Result<(), String> {
        match r {
            Response::Report { targets, .. } => spoken_or_why(&targets),
            Response::Accepted { .. } | Response::Finished { .. } => Ok(()),
            other => Err(describe(other)),
        }
    }
}

/// Turn an unexpected response into something worth showing a person.
fn describe(r: Response) -> String {
    match r {
        Response::Error { message, .. } => message,
        other => format!("unexpected response: {other:?}"),
    }
}

/// How hostnames get resolved on this platform.
///
/// Android reads its DNS configuration through JNI, which needs an
/// initialised `ndk_context`. Tauri does not provide one, and the lookup
/// panics rather than failing — killing the task that starts the node. Using
/// a fixed public resolver avoids the JNI path entirely and is what iroh
/// falls back to anyway when it cannot read the system config.
fn dns_resolver() -> Option<iroh::dns::DnsResolver> {
    #[cfg(target_os = "android")]
    {
        // Cloudflare. A carrier's resolver would be preferable, but reaching
        // it costs a JNI context we do not have.
        Some(iroh::dns::DnsResolver::with_nameserver(
            "1.1.1.1:53".parse().expect("valid nameserver address"),
        ))
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

/// The speech engine for this platform.
///
/// Falls back to [`SilentEngine`] rather than refusing to start: a device that
/// cannot speak is still a useful member of a space — it can be joined,
/// renamed, and told about — and it reports `no_engine` honestly instead of
/// swallowing messages.
fn speech_engine() -> Arc<dyn SpeechEngine> {
    // iOS named rather than left to fall through. It is unix and it is not
    // Android, so it took the arm below and would have called
    // `PiperEngine::discover()` on a platform that cannot spawn a child
    // process at all. Nobody wrote that; it was inherited from phrasing the
    // exclusion as "unix, and not Android", so the code disagreed with a
    // decision its owner had already made out loud (#126).
    //
    // The reason matters as much as the arm. "No speech engine is installed"
    // sends a reader to install something; this platform has nothing to
    // install yet, and saying so is the difference between a fixable fault
    // and a missing feature. When there is an iOS build worth shipping this
    // becomes `AVSpeechSynthesizer`, which is a real engine rather than a
    // process to spawn.
    #[cfg(target_os = "ios")]
    {
        // `AVSpeechSynthesizer`, which is what iOS speech is — not a
        // stand-in, so its tier is `Full`.
        return match voicecast_engine::IosEngine::new() {
            Ok(engine) => Arc::new(engine),
            // Its own reason, not a stand-in's. The failure this is most
            // likely to carry is the audio session refusing playback, which
            // means the phone would have spoken silently — worth saying
            // rather than replacing with "no engine".
            Err(e) => {
                eprintln!("the iOS speech engine did not start: {e}");
                Arc::new(voicecast_engine::SilentEngine::new(e.reason()))
            }
        };
    }

    #[cfg(all(unix, not(any(target_os = "android", target_os = "ios"))))]
    {
        // Best available, in order. Piper is what this platform should sound
        // like; espeak is a floor where the host happens to provide one, which
        // on Linux is common and on macOS is essentially never.
        let piper = match voicecast_engine::PiperEngine::discover() {
            Ok(engine) => return Arc::new(engine),
            Err(e) => e,
        };
        eprintln!("piper unavailable: {piper}");
        match voicecast_engine::EspeakEngine::new() {
            Ok(engine) => Arc::new(engine),
            Err(e) => {
                eprintln!("espeak-ng unavailable: {e}");
                // Piper's reason, not a stand-in — the same rule the Windows
                // branch below already follows. This said "no speech engine is
                // installed on this device", which is a lie on any Mac whose
                // Piper is present and merely broken: a missing dylib, a
                // signature the OS refuses, an Intel binary on arm64. It sent
                // whoever read it to install what they already had, while the
                // sentence that named the real fault went to stderr, which for
                // an app launched from Finder is nowhere at all.
                //
                // Rediscovering rather than Silent: discovery ran once and
                // the answer was kept for the life of the process, so the
                // first-run path was "install Piper, be told Piper is not
                // installed" by a node whose statement had stopped being true
                // (#84). This keeps looking.
                Arc::new(voicecast_engine::Rediscovering::new(
                    piper.reason(),
                    Box::new(|| {
                        voicecast_engine::PiperEngine::discover()
                            .ok()
                            .map(|e| Arc::new(e) as Arc<dyn SpeechEngine>)
                    }),
                ))
            }
        }
    }
    #[cfg(target_os = "android")]
    {
        match voicecast_engine::AndroidEngine::new() {
            Ok(engine) => Arc::new(engine),
            Err(e) => {
                eprintln!("android speech unavailable: {e}");
                Arc::new(voicecast_engine::SilentEngine::new(format!(
                    "this device's speech engine could not start: {e}"
                )))
            }
        }
    }
    // Windows. Piper, the same engine as every other desktop — and when it
    // does not start, the reason it gave rather than a stand-in. There is no
    // floor engine here, so that reason is the only thing standing between
    // the sender and an unexplained silence; see decision 22.
    #[cfg(not(unix))]
    {
        match voicecast_engine::PiperEngine::discover() {
            Ok(engine) => Arc::new(engine),
            Err(e) => {
                eprintln!("piper unavailable: {e}");
                Arc::new(voicecast_engine::SilentEngine::new(e.reason()))
            }
        }
    }
}

/// This device's key store.
///
/// Desktop shares the keyring-backed store with `voicecastd`, so the app and
/// the daemon are the same device rather than two devices arguing over one
/// roster. Mobile has no keyring backend, so the key lives in app-private
/// storage instead.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn key_store() -> anyhow::Result<Box<dyn voicecast_core::KeyStore>> {
    Ok(Box::new(voicecast_keystore::DesktopKeyStore::new()?))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn key_store() -> anyhow::Result<Box<dyn voicecast_core::KeyStore>> {
    Ok(Box::new(voicecast_core::FileKeyStore::default_location()?))
}

/// Build the node this app wraps.
async fn start_node() -> anyhow::Result<Node> {
    // Before the call, because the call is where this process can stop dead.
    // Reading the key waits on a keychain dialog on macOS after every update,
    // and until now said nothing while it did — leaving a running app that
    // had bound no socket and explained nothing. Stderr is a poor channel for
    // an app launched from Finder, but `open -a` and a terminal launch both
    // show it, which is how anybody debugging this starts.
    // Asked before the key store, and long before the transport. Reaching
    // this point with another node already running used to mean a second
    // endpoint online under the same secret key and a window that looked
    // healthy — the failure only surfaced at `serve()`, by which time the
    // damage was done. Desktop only: a phone has no socket and no CLI.
    #[cfg(desktop)]
    if voicecast_core::ipc::node_is_listening().await {
        anyhow::bail!(
            "another voicecast node is already running on this machine. \
             Only one can hold this device's identity at a time — quit the \
             other one, or use the window it already has"
        );
    }

    eprintln!("opening the key store…");
    let store = key_store()?;
    let identity = Identity::load_or_create(store.as_ref())?;
    let name = voicecast_core::device_name();

    let engine = speech_engine();
    let transport = Transport::bind(identity.secret().clone(), dns_resolver()).await?;
    Node::new(engine, identity, transport, name).await
}

/// iOS needs a rustls crypto provider chosen before anything builds an
/// HTTPS client.
///
/// iroh reaches for one during startup and reqwest panics rather than
/// erroring — "No rustls crypto provider is configured" — so the app died
/// before its first frame. Every other target resolves a default through
/// feature unification; iOS does not, and nothing said so until it ran.
/// `ring` rather than `aws-lc-rs` because `ring` is already in the tree, so
/// this selects a provider rather than adding one.
///
/// Ignoring the result is deliberate: the call fails only if a provider is
/// already installed, which is the state being asked for.
#[cfg(target_os = "ios")]
fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Start the app.
///
/// # Panics
/// If Tauri itself cannot start, since there is nothing to show without it.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Before the builder, because the node starts inside `setup` and iroh
    // builds an HTTPS client on its way up.
    #[cfg(target_os = "ios")]
    install_crypto_provider();

    tauri::Builder::default()
        .setup(|app| {
            // Where this device's state lives has to be settled first:
            // everything below reads it.
            //
            // Mobile has no XDG config directory, so the core has to be told
            // about app-private storage. Desktop has one, and `voicecastd`
            // was already using it — overriding it here gave a single device
            // two rosters, two histories and two mute settings, sharing only
            // the identity that lives in the keyring.
            #[cfg(mobile)]
            match app.path().app_data_dir() {
                Ok(dir) => voicecast_core::set_config_dir(dir),
                Err(e) => eprintln!("no app data directory: {e}"),
            }

            // Bring across anything an older build left in the app's own
            // directory. Said out loud rather than done quietly: it moves
            // the file that holds every device pairing.
            #[cfg(desktop)]
            if let Ok(old) = app.path().app_data_dir() {
                match voicecast_core::migrate_from(&old) {
                    Ok(moved) if !moved.is_empty() => {
                        eprintln!("moved {} into the config directory", moved.join(", "));
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("could not move state out of the app directory: {e}"),
                }
            }

            // The CLI lives on the host, not in the sandbox. Put it there on
            // first launch, and rewrite it when a Flatpak update has moved
            // the app on without it.
            #[cfg(desktop)]
            install_cli_on_host();

            // Only refreshes a copy this app was asked to install.
            #[cfg(desktop)]
            refresh_installed_skill();

            // A missing tray must not stop the app. Some environments have no
            // StatusNotifier host, and the GNOME Flatpak runtime ships no
            // appindicator library at all — the node is still perfectly
            // useful without an icon, and `voicecast show` can reach it.
            //
            // Wrapped in catch_unwind because the failure is a *panic* inside
            // the tray crate's dynamic loading, not an error it returns, so
            // there is nothing to propagate.
            #[cfg(desktop)]
            {
                let handle = app.handle().clone();
                let built =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build_tray(&handle)));
                match built {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => eprintln!("no tray icon: {e}"),
                    Err(_) => eprintln!("no tray icon: this system has no app indicator library"),
                }
            }

            // Registered before anything can fail, so the interface has
            // somewhere to read an answer from even when there is no node.
            app.manage(StartupState(std::sync::Mutex::new(Startup::Starting)));

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match start_node().await {
                    Ok(node) => {
                        let node = Arc::new(node);
                        node.start_presence_checks();
                        handle.manage(AppState {
                            node: Arc::clone(&node),
                        });
                        handle
                            .state::<StartupState>()
                            .set(Startup::Running(Arc::clone(&node)));

                        // Give the CLI a way to reach a window it cannot see.
                        // This matters most where the tray icon is collapsed
                        // or absent: without it, closing the window leaves a
                        // running node with no way back and no way out.
                        // Desktop only — a phone has no CLI to reach it from.
                        #[cfg(desktop)]
                        {
                            let show_handle = handle.clone();
                            let quit_handle = handle.clone();
                            node.set_window_hooks(
                                Arc::new(move || reveal(&show_handle)),
                                Arc::new(move || quit_handle.exit(0)),
                            )
                            .await;
                        }
                        // On desktop the app is the node the CLI talks to:
                        // install it, open it, and `voicecast --to phone ...`
                        // works with no separate daemon. A phone has no CLI,
                        // so binding a local socket there would serve nobody
                        // and may not work on Android at all.
                        let outcome = if cfg!(any(target_os = "android", target_os = "ios")) {
                            node.serve_peers().await
                        } else {
                            node.serve().await
                        };
                        if let Err(e) = outcome {
                            let why = format!("{e:#}");
                            eprintln!("node stopped: {why}");
                            // Taken off the network, not merely reported. The
                            // transport is already bound by this point, so a
                            // node that cannot claim the socket was leaving a
                            // second endpoint online under this device's
                            // secret key — peers reaching whichever the relay
                            // saw last, while the CLI reached the other one.
                            // Issue #72.
                            node.close().await;
                            handle.state::<StartupState>().set(Startup::Failed(why));
                        }
                    }
                    Err(e) => {
                        let why = format!("{e:#}");
                        eprintln!("could not start node: {why}");
                        handle.state::<StartupState>().set(Startup::Failed(why));
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            node_status,
            list_devices,
            make_invite,
            preview_invite,
            join_space,
            rename_device,
            battery_ok,
            request_battery_exemption,
            revoke_device,
            leave_space,
            pending_invite,
            cli_status,
            cli_installable,
            cli_expected_path,
            cli_on_path,
            install_cli,
            skill_status,
            install_skill,
            reset_skill_path,
            now_playing,
            pause_speech,
            resume_speech,
            stop_speech,
            skip_speech,
            history,
            replay,
            clear_history,
            list_spaces,
            new_space,
            drop_space,
            default_space,
            rename_space,
            rotate_space,
            policy,
            set_mute,
            set_quiet,
            voice_config,
            set_voice,
            set_rate,
            speak
        ])
        .on_window_event(|_window, _event| {
            #[cfg(desktop)]
            // Closing the window hides it rather than quitting: the node has
            // to keep running for the CLI and for peers to reach this device.
            // Quitting is a deliberate act from the tray menu.
            if let tauri::WindowEvent::CloseRequested { api, .. } = _event {
                api.prevent_close();
                let _ = _window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while running voicecast")
        .run(|_app, _event| {
            // Closing the window hides it, so on macOS the app is still
            // running with no window and clicking the Dock icon is the
            // obvious way back. Without this the click does nothing at all:
            // `Reopen` is delivered and was never handled, and the tray menu
            // was the only route to a window the user had just asked for.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                reveal(_app);
            }
        });
}

/// A tray icon so the user can see the node is running, and stop it.
///
/// A visible icon beats a hidden daemon: it says the thing is alive and gives
/// an obvious way to quit. On Linux this needs the desktop shell to provide a
/// StatusNotifierItem host, which not every compositor does — the app still
/// works without one, it is just harder to notice.
///
/// Worth knowing when it seems not to work: `libayatana-appindicator`
/// registers under its unique D-Bus connection name plus an object path,
/// not the `org.freedesktop.StatusNotifierItem-PID-N` well-known name, so
/// grepping for the latter finds nothing even on success. Ask the watcher
/// for `RegisteredStatusNotifierItems` instead. Some bars — Omarchy's
/// included — then collapse third-party icons behind a drawer.
#[cfg(desktop)]
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show voicecast", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit voicecast", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    // Embedded rather than taken from the window icon: libayatana-appindicator
    // refuses to register a tray item with no icon, and it fails silently —
    // the process simply never claims a StatusNotifierItem name.
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))?;

    TrayIconBuilder::with_id("voicecast")
        .icon(icon)
        .tooltip("voicecast")
        .menu(&menu)
        // The menu belongs on the secondary button. A left click is how a
        // person asks for the window, and with the menu moved off it and
        // nothing put in its place, a left click did nothing at all — the
        // window was reachable only through a menu you had to know to
        // right-click for.
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                // On release, not on press: a click that is still down may
                // yet become a drag, and acting on the press raises the
                // window while the pointer is being moved away.
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                reveal(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => reveal(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

/// Bring the window back from the tray.
#[cfg(desktop)]
fn reveal(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// A throwaway home directory, named so parallel tests cannot collide.
    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "voicecast-path-{}-{label}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("scratch home");
        dir
    }

    /// The line is added when no profile puts the directory on the PATH.
    #[test]
    fn writes_a_line_when_nothing_supplies_it() {
        let home = scratch("fresh");
        let added = ensure_on_path(&home, &home.join(".local/bin")).expect("ensure");
        assert!(added);

        let written = std::fs::read_to_string(home.join(PROFILE)).expect("profile");
        assert!(
            written.contains(r#"export PATH="$HOME/.local/bin:$PATH""#),
            "{written}"
        );

        // Running again must not append a second copy: the first line is now
        // itself the thing that satisfies the check.
        let again = ensure_on_path(&home, &home.join(".local/bin")).expect("ensure");
        assert!(!again);
        let after = std::fs::read_to_string(home.join(PROFILE)).expect("profile");
        assert_eq!(written, after);

        std::fs::remove_dir_all(&home).ok();
    }

    /// A line the user wrote themselves, in any profile, is left alone.
    #[test]
    fn leaves_an_existing_line_alone() {
        let home = scratch("existing");
        std::fs::write(
            home.join(".zshrc"),
            "export PATH=\"$HOME/.local/bin:$PATH\"\n",
        )
        .expect("zshrc");

        let added = ensure_on_path(&home, &home.join(".local/bin")).expect("ensure");
        assert!(!added);
        assert!(!Path::new(&home.join(PROFILE)).exists());

        std::fs::remove_dir_all(&home).ok();
    }

    /// An existing profile keeps its contents, and does not lose a newline.
    #[test]
    fn appends_without_clobbering() {
        let home = scratch("append");
        std::fs::write(home.join(PROFILE), "export EDITOR=vim").expect("profile");

        assert!(ensure_on_path(&home, &home.join(".local/bin")).expect("ensure"));

        let written = std::fs::read_to_string(home.join(PROFILE)).expect("profile");
        assert!(written.starts_with("export EDITOR=vim\n"), "{written}");
        assert!(written.contains("$HOME/.local/bin"), "{written}");

        std::fs::remove_dir_all(&home).ok();
    }
}

/// Reading a report, which is platform-independent and so tested everywhere.
///
/// Its own module because the one above is `#[cfg(all(test, target_os =
/// "macos"))]` — tests dropped in there compile on every target and run on
/// one, which is indistinguishable from passing.
#[cfg(test)]
mod report_tests {
    use super::PolicyView;
    use voicecast_proto::Response;

    #[test]
    fn a_policy_that_could_not_be_read_is_an_error_not_a_blank_one() {
        // The comment on this conversion used to say that reporting "not
        // muted" would be "a lie the interface then shows as settled state",
        // and then returned exactly that. A corrupt policy file drew a device
        // that was unmuted with no quiet window (#73).
        let refused = Response::error("policy.json is not valid JSON");
        match PolicyView::try_from(refused) {
            Err(why) => assert_eq!(why, "policy.json is not valid JSON"),
            Ok(_) => panic!("a policy that could not be read must not convert"),
        }
    }

    #[test]
    fn an_unexpected_reply_is_also_an_error() {
        // A reply this build has never seen must not be read as "nothing
        // configured" either — that is the same lie by another route.
        assert!(PolicyView::try_from(Response::Done).is_err());
    }

    #[test]
    fn a_real_policy_still_converts() {
        let ok = Response::Policy {
            muted: true,
            quiet_from: Some("22:00".into()),
            quiet_to: Some("07:00".into()),
            high_breaks_through: true,
            spaces: Vec::new(),
        };
        let Ok(view) = PolicyView::try_from(ok) else {
            panic!("a real policy converts")
        };
        assert!(view.muted);
        assert_eq!(view.from.as_deref(), Some("22:00"));
    }

    use super::*;

    /// The bug this closes: a successful report read as a failure.
    ///
    /// The node queued the message and spoke it, and the interface showed the
    /// debug formatting of that very report as an error toast. Work done,
    /// failure announced.
    #[test]
    fn a_queued_message_is_success() {
        use voicecast_proto::{Status, TargetResult};
        let report = [TargetResult {
            device: "Phone".into(),
            endpoint_id: "97514a80e9425dd3".into(),
            status: Status::Queued,
            took_ms: None,
            detail: None,
        }];
        assert!(spoken_or_why(&report).is_ok());
    }

    /// A muted device says so rather than failing silently or lying.
    #[test]
    fn a_muted_device_says_why() {
        use voicecast_proto::{Status, TargetResult};
        let report = [TargetResult {
            device: "Phone".into(),
            endpoint_id: "x".into(),
            status: Status::Muted,
            took_ms: None,
            detail: None,
        }];
        assert_eq!(spoken_or_why(&report).unwrap_err(), "this device is muted");
    }

    /// An engine failure carries the engine's own reason, not a stand-in.
    ///
    /// Decision 30: "Piper is not installed in any of …" sends someone to the
    /// right place; "no speech engine" sends them to install what they have.
    #[test]
    fn an_engine_failure_keeps_its_reason() {
        use voicecast_proto::{Status, TargetResult};
        let report = [TargetResult {
            device: "Laptop".into(),
            endpoint_id: "x".into(),
            status: Status::NoEngine,
            took_ms: None,
            detail: Some("Piper is not installed in any of: /app/share/voicecast".into()),
        }];
        assert!(
            spoken_or_why(&report)
                .unwrap_err()
                .contains("/app/share/voicecast")
        );
    }

    /// One device speaking is enough, even if another refused.
    #[test]
    fn one_device_speaking_is_not_a_failure() {
        use voicecast_proto::{Status, TargetResult};
        let row = |status| TargetResult {
            device: "d".into(),
            endpoint_id: "x".into(),
            status,
            took_ms: None,
            detail: None,
        };
        assert!(spoken_or_why(&[row(Status::Muted), row(Status::Spoken)]).is_ok());
    }
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    /// Issue #72. These three were indistinguishable to the interface,
    /// because the only signal it had was whether `node_status` errored —
    /// and it errored identically for "not registered yet" and "never will
    /// be".
    #[test]
    fn the_three_states_are_distinguishable() {
        let starting = status_of(&Startup::Starting);
        assert!(starting.starting, "a node coming up says so");
        assert!(starting.failed.is_none());

        let failed = status_of(&Startup::Failed("the keychain is locked".into()));
        assert!(!failed.starting, "a failure is not a transient state");
        assert_eq!(failed.failed.as_deref(), Some("the keychain is locked"));
    }

    /// The reason has to survive to the interface verbatim, since it is the
    /// only place it can be read: stderr is nowhere for an app opened from
    /// Finder.
    #[test]
    fn the_reason_is_carried_not_summarised() {
        let why = "another voicecast node is already running on this machine";
        let status = status_of(&Startup::Failed(why.into()));
        assert_eq!(status.failed.as_deref(), Some(why));
        assert_eq!(
            status.engine, "unavailable",
            "a node that does not exist has no engine, and 'unknown' would \
             read as a device that might still speak"
        );
    }
}

/// Issue #46: the node changes what it returns and the app goes on matching
/// the old shape.
///
/// Three times this compiled, passed CI, and was found by a person pressing a
/// button. Nothing here makes the pairing a compile-time promise — that is
/// #79's seam — but it makes it *executable*: a real node is driven through
/// the same `replies` functions the commands use, and the assertion is that
/// the catch-all did not fire.
///
/// **Not `#[cfg(all(test, target_os = "macos"))]`**, which is the module
/// thirty lines up. Tests written there compile on every target and run on
/// none of the ones CI tests, which is indistinguishable from passing. Four
/// tests landed in it before being moved. Check which module you are in.
#[cfg(test)]
mod command_tests {
    use super::*;
    use voicecast_core::identity::FileKeyStore;
    use voicecast_engine::SilentEngine;

    /// Whether a reply was understood at all.
    ///
    /// A refusal is not a failure here — a silent engine legitimately reports
    /// that it cannot speak, and a command that surfaces that is doing its
    /// job. What this catches is the catch-all: `describe` fires only when
    /// the node returned a shape no arm names, which is the drift itself.
    fn understood<T>(what: &str, r: Result<T, String>) {
        if let Err(message) = r {
            assert!(
                !message.starts_with("unexpected response"),
                "{what}: {message}\n\
                 The node returned a reply this command does not name. That is \
                 the drift in #46: it compiles, and a person finds it by \
                 pressing the button."
            );
        }
    }

    /// One node, on a scratch config directory, driven through every command
    /// that a device with no peers can answer.
    ///
    /// One test rather than several: `set_config_dir` is a `OnceLock`, so the
    /// first caller in the process wins and a second test wanting its own
    /// directory would silently share this one's.
    #[tokio::test(flavor = "multi_thread")]
    async fn every_command_understands_what_the_node_returns() {
        let dir = std::env::temp_dir().join(format!("voicecast-commands-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        voicecast_core::identity::set_config_dir(dir.clone());

        // A file store rather than the platform keyring: this must not touch
        // the developer's own keychain, and a file is the one store that
        // behaves identically on all five targets.
        let store = FileKeyStore::at(dir.join("identity.key"));
        let identity = Identity::load_or_create(&store).expect("identity");
        let transport = Transport::bind(identity.secret().clone(), None)
            .await
            .expect("binding a transport");
        let engine = Arc::new(SilentEngine::new("no engine in a test"));
        let node = Node::new(engine, identity, transport, "Test".into())
            .await
            .expect("node");

        // Reading. Nothing here changes state, so order does not matter.
        assert!(
            replies::status(node.status()).is_some(),
            "node_status: the reply was not a Status, so the panel would draw \
             an unknown engine for a node that is running"
        );
        assert!(
            replies::queue(node.queue_state()).is_some(),
            "now_playing: the reply was not a Queue, so the playback controls \
             would draw an idle device forever"
        );
        understood("list_devices", replies::devices(node.devices().await));
        understood("list_spaces", replies::spaces(node.spaces().await));
        understood("history", replies::history(node.history(None)));
        assert!(
            PolicyView::try_from(node.policy().await).is_ok(),
            "policy: a policy the app cannot read is drawn as settled state"
        );

        // Speaking. The engine is silent, so this reports a refusal — which
        // is a understood reply, and exactly the case that used to produce no
        // message at all.
        understood(
            "speak",
            replies::spoke(node.speak("hello".into(), Priority::Normal, None).await),
        );

        // Replaying needs something to replay, so it follows the speak above.
        let msg_id = match node.history(Some(1)) {
            Response::History { entries } => entries.first().map(|e| e.msg_id.clone()),
            _ => None,
        };
        if let Some(msg_id) = msg_id {
            understood("replay", replies::accepted(node.replay(&msg_id)));
        }

        // Invites. `preview` is fed the invite this node just made, which is
        // the only ticket available without a second device.
        let invite = node.invite(None).await;
        understood("make_invite", replies::invite(invite.clone()));
        if let Response::Invite { url, .. } = invite {
            understood("preview_invite", replies::preview(node.preview(&url)));
        }

        // Naming.
        understood("rename_device", replies::renamed(node.rename("Test").await));

        // Spaces, in an order that leaves the node with one to belong to.
        // `join` is deliberately absent: it dials a peer, so without a second
        // device it would wait on a network rather than return a shape.
        understood(
            "new_space",
            replies::spaces_changed(node.new_space("Two").await),
        );
        understood(
            "rename_space",
            replies::spaces_changed(node.rename_space("Two", "Three").await),
        );
        understood(
            "default_space",
            replies::spaces_changed(node.default_space("Three").await),
        );
        understood(
            "rotate_space",
            replies::rotated(node.rotate(Some("Three")).await),
        );
        understood(
            "revoke_device",
            replies::renamed(node.revoke("Test", Some("Three")).await),
        );
        understood(
            "leave_space",
            replies::left(node.leave(Some("Three")).await),
        );
        understood(
            "drop_space",
            replies::spaces_changed(node.leave_space("Three").await),
        );
        understood("clear_history", replies::done(node.clear_history()));

        node.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
