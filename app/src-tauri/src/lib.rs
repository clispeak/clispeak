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
use voicecast_core::{FileKeyStore, Identity, Node, Transport};
use voicecast_engine::{EspeakEngine, SpeechEngine};
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

/// An invite, ready to hand to another device.
#[derive(Serialize)]
pub struct Invite {
    /// The `voicecast://join/...` string.
    pub url: String,
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
        Response::Invite { url, expires_in } => Ok(Invite { url, expires_in }),
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

/// Build the node this app wraps.
async fn start_node() -> anyhow::Result<Node> {
    let store = FileKeyStore::default_location()?;
    let identity = Identity::load_or_create(&store)?;
    let name = device_name();

    let engine: Arc<dyn SpeechEngine> = Arc::new(EspeakEngine::new()?);
    let transport = Transport::bind(identity.secret().clone()).await?;
    Node::new(engine, identity, transport, name).await
}

/// This device's local label. A convenience only — identity is the key.
fn device_name() -> String {
    std::env::var("VOICECAST_NAME")
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "this device".to_string())
}

/// Start the app.
///
/// # Panics
/// If Tauri itself cannot start, since there is nothing to show without it.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match start_node().await {
                    Ok(node) => {
                        let node = Arc::new(node);
                        handle.manage(AppState {
                            node: Arc::clone(&node),
                        });
                        // Peers only: a phone has no CLI, so binding a local
                        // IPC socket would serve nobody.
                        if let Err(e) = node.serve_peers().await {
                            eprintln!("peer loop stopped: {e:#}");
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
            speak
        ])
        .run(tauri::generate_context!())
        .expect("error while running voicecast");
}
