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
/// `~/.local/bin` because it is on the default PATH and needs no privileges.
fn cli_destination() -> Option<std::path::PathBuf> {
    Some(
        directories::BaseDirs::new()?
            .home_dir()
            .join(".local/bin/voicecast"),
    )
}

/// Whether the app is running inside a Flatpak sandbox.
///
/// Only then is installing a copy of the CLI meaningful: outside one it is
/// already alongside the app, wherever the package put it.
fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

/// The copy of the CLI carried inside the sandbox.
fn bundled_cli() -> &'static std::path::Path {
    std::path::Path::new("/app/libexec/voicecast")
}

/// Whether the command-line tool is available to agents on this machine.
#[tauri::command]
fn cli_status() -> Option<String> {
    if !in_flatpak() {
        return None;
    }
    let dest = cli_destination()?;
    dest.exists().then(|| dest.display().to_string())
}

/// Whether offering to install the CLI makes sense here.
#[tauri::command]
fn cli_installable() -> bool {
    in_flatpak() && cli_status().is_none()
}

/// Where the CLI is expected to be, whether or not it is there yet.
///
/// Reported so the interface can say what went wrong when the automatic
/// install failed, rather than leaving a missing `voicecast` unexplained.
#[tauri::command]
fn cli_expected_path() -> Option<String> {
    in_flatpak()
        .then(cli_destination)
        .flatten()
        .map(|p| p.display().to_string())
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
    let Ok(bundled) = std::fs::read(bundled_cli()) else {
        // Nothing to install from.
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
    if !in_flatpak() || !cli_needs_install() {
        return;
    }
    match install_cli() {
        Ok(path) => eprintln!("installed the command-line tool at {path}"),
        Err(e) => eprintln!("could not install the command-line tool: {e}"),
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
    let source = bundled_cli();
    if !source.exists() {
        return Err("this build does not carry a copy of the command-line tool".into());
    }
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
    #[cfg(not(unix))]
    {
        Arc::new(voicecast_engine::SilentEngine::new(
            "no speech engine is wired up for this platform yet",
        ))
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
            // The CLI lives on the host, not in the sandbox. Put it there on
            // first launch, and rewrite it when a Flatpak update has moved
            // the app on without it.
            #[cfg(desktop)]
            install_cli_on_host();

            // Android has no XDG config directory, so tell the core where its
            // app-private storage is before anything tries to read a key.
            match app.path().app_data_dir() {
                Ok(dir) => voicecast_core::set_config_dir(dir),
                Err(e) => eprintln!("no app data directory: {e}"),
            }

            // A missing tray must not stop the app. Some environments have no
            // StatusNotifier host, and the GNOME Flatpak runtime ships no
            // appindicator library at all — the node is still perfectly
            // useful without an icon, and `voicecast show` can reach it.
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
            install_cli,
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
