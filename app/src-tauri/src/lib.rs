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
    tray::TrayIconBuilder,
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
}

impl From<Response> for PolicyView {
    fn from(r: Response) -> Self {
        match r {
            Response::Policy {
                muted,
                quiet_from,
                quiet_to,
                high_breaks_through,
            } => Self {
                muted,
                from: quiet_from,
                to: quiet_to,
                high_breaks_through,
            },
            // Any other reply means the write failed. Reporting "not muted"
            // would be a lie the interface then shows as settled state, so
            // report the safe reading: nothing configured.
            _ => Self {
                muted: false,
                from: None,
                to: None,
                high_breaks_through: false,
            },
        }
    }
}

/// This device's speaking policy.
#[tauri::command]
fn policy(state: State<'_, AppState>) -> PolicyView {
    state.node.policy().into()
}

/// Silence this device, or let it speak again.
#[tauri::command]
fn set_mute(state: State<'_, AppState>, muted: bool) -> PolicyView {
    state.node.set_mute(muted).into()
}

/// Set or clear the daily quiet window.
#[tauri::command]
fn set_quiet(
    state: State<'_, AppState>,
    from: Option<String>,
    to: Option<String>,
    high_breaks_through: bool,
) -> Result<PolicyView, String> {
    match state.node.set_quiet(from, to, high_breaks_through) {
        Response::Error { message } => Err(message),
        other => Ok(other.into()),
    }
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

#[tauri::command]
fn node_status(state: State<'_, AppState>) -> NodeStatus {
    match state.node.status() {
        Response::Status {
            device_id,
            engine,
            fallback,
            ..
        } => NodeStatus {
            name: state.node.name().to_string(),
            device_id,
            engine,
            fallback,
        },
        _ => NodeStatus {
            name: state.node.name().to_string(),
            device_id: state.node.id(),
            engine: "unknown".into(),
            fallback: true,
        },
    }
}

#[tauri::command]
async fn list_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, String> {
    match state.node.devices().await {
        Response::Devices { devices } => Ok(devices),
        other => Err(describe(other)),
    }
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
    let (speaking, pending, paused) = match state.node.queue_state() {
        Response::Queue {
            speaking,
            pending,
            paused,
        } => (speaking, pending, paused),
        _ => (None, Vec::new(), false),
    };
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
    match state.node.history(limit) {
        Response::History { entries } => Ok(entries),
        other => Err(describe(other)),
    }
}

/// Speak a message from the history again.
///
/// Plays through mute and quiet hours: pressing play is the consent those
/// settings exist to require.
#[tauri::command]
fn replay(state: State<'_, AppState>, msg_id: String) -> Result<(), String> {
    match state.node.replay(&msg_id) {
        Response::Accepted { .. } => Ok(()),
        other => Err(describe(other)),
    }
}

/// Forget the history.
#[tauri::command]
fn clear_history(state: State<'_, AppState>) -> Result<(), String> {
    match state.node.clear_history() {
        Response::Done => Ok(()),
        other => Err(describe(other)),
    }
}

/// The spaces this device belongs to.
#[tauri::command]
async fn list_spaces(state: State<'_, AppState>) -> Result<Vec<voicecast_proto::SpaceRow>, String> {
    match state.node.spaces().await {
        Response::Spaces { spaces } => Ok(spaces),
        other => Err(describe(other)),
    }
}

/// Found a new space from this device, and make it the default.
#[tauri::command]
async fn new_space(state: State<'_, AppState>, label: String) -> Result<(), String> {
    match state.node.new_space(&label).await {
        Response::Spaces { .. } => Ok(()),
        other => Err(describe(other)),
    }
}

/// Drop one space, keeping the others.
///
/// Distinct from `leave_space`, which leaves the space this device is
/// currently using; this one names which of several to drop.
#[tauri::command]
async fn drop_space(state: State<'_, AppState>, label: String) -> Result<(), String> {
    match state.node.leave_space(&label).await {
        Response::Spaces { .. } => Ok(()),
        other => Err(describe(other)),
    }
}

/// Choose which space bare device names resolve in.
#[tauri::command]
async fn default_space(state: State<'_, AppState>, label: String) -> Result<(), String> {
    match state.node.default_space(&label).await {
        Response::Spaces { .. } => Ok(()),
        other => Err(describe(other)),
    }
}

/// Rename a space locally.
#[tauri::command]
async fn rename_space(state: State<'_, AppState>, label: String, to: String) -> Result<(), String> {
    match state.node.rename_space(&label, &to).await {
        Response::Spaces { .. } => Ok(()),
        other => Err(describe(other)),
    }
}

/// Replace the default space, locking every other device out at once.
///
/// Returns the devices that were in it, so the interface can say who has to
/// be re-invited rather than leaving the user to remember.
#[tauri::command]
async fn rotate_space(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    match state.node.rotate().await {
        Response::Rotated { devices } => Ok(devices),
        other => Err(describe(other)),
    }
}

#[tauri::command]
async fn make_invite(state: State<'_, AppState>) -> Result<Invite, String> {
    match state.node.invite().await {
        Response::Invite { url, expires_in } => {
            // A missing QR is a degraded invite, not a failed one: the code
            // can still be copied.
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

#[tauri::command]
async fn join_space(state: State<'_, AppState>, ticket: String) -> Result<usize, String> {
    match state.node.join(&ticket).await {
        Response::Joined { members } => Ok(members),
        other => Err(describe(other)),
    }
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
    match state.node.rename(&name).await {
        Response::Renamed { name } => Ok(name),
        other => Err(describe(other)),
    }
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
async fn revoke_device(state: State<'_, AppState>, name: String) -> Result<String, String> {
    match state.node.revoke(&name).await {
        Response::Renamed { name } => Ok(name),
        other => Err(describe(other)),
    }
}

#[tauri::command]
async fn leave_space(state: State<'_, AppState>) -> Result<String, String> {
    match state.node.leave().await {
        Response::Renamed { name } => Ok(name),
        other => Err(describe(other)),
    }
}

#[tauri::command]
async fn speak(state: State<'_, AppState>, text: String) -> Result<(), String> {
    // Validated here rather than in the UI so the app and the CLI reject
    // exactly the same things.
    if let Err(rejection) = voicecast_text::validate(&text) {
        return Err(rejection.to_string());
    }
    match state.node.speak(text, Priority::Normal, None).await {
        Response::Accepted { .. } | Response::Finished { .. } => Ok(()),
        other => Err(describe(other)),
    }
}

/// Turn an unexpected response into something worth showing a person.
fn describe(r: Response) -> String {
    match r {
        Response::Error { message } => message,
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
    #[cfg(all(unix, not(target_os = "android")))]
    {
        // Best available, in order. Piper is what this platform should sound
        // like; espeak is the floor that guarantees a device is never silent.
        match voicecast_engine::PiperEngine::discover() {
            Ok(engine) => return Arc::new(engine),
            Err(e) => eprintln!("piper unavailable: {e}"),
        }
        match voicecast_engine::EspeakEngine::new() {
            Ok(engine) => Arc::new(engine),
            Err(e) => {
                eprintln!("espeak-ng unavailable: {e}");
                Arc::new(voicecast_engine::SilentEngine::new(
                    "no speech engine is installed on this device",
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
                Arc::new(voicecast_engine::SilentEngine::new(e.to_string()))
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
    let store = key_store()?;
    let identity = Identity::load_or_create(store.as_ref())?;
    let name = voicecast_core::device_name();

    let engine = speech_engine();
    let transport = Transport::bind(identity.secret().clone(), dns_resolver()).await?;
    Node::new(engine, identity, transport, name).await
}

/// Start the app.
///
/// # Panics
/// If Tauri itself cannot start, since there is nothing to show without it.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match start_node().await {
                    Ok(node) => {
                        let node = Arc::new(node);
                        node.start_presence_checks();
                        handle.manage(AppState {
                            node: Arc::clone(&node),
                        });

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
                            eprintln!("node stopped: {e:#}");
                        }
                    }
                    Err(e) => eprintln!("could not start node: {e:#}"),
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            node_status,
            list_devices,
            make_invite,
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
        .run(tauri::generate_context!())
        .expect("error while running voicecast");
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
        .show_menu_on_left_click(false)
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
