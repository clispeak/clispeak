//! The node process.
//!
//! Temporary: from M6 the Tauri app is the long-running process and links
//! `voicecast-core` directly. Until that exists, this stands in so the CLI
//! has something to talk to.

use std::sync::Arc;

use anyhow::{Context, Result};
use voicecast_core::{Identity, Node, Transport};
use voicecast_engine::SpeechEngine;
use voicecast_keystore::DesktopKeyStore;

/// The best engine this platform has.
///
/// Both engines this binary can use spawn a Unix process, so both are
/// `#[cfg(unix)]` — and this file imported them unconditionally, which broke
/// the Windows build the moment Piper landed and kept it broken. Selecting
/// behind the same gate is the fix; the portability rule in
/// `docs/build-plan.md` is what should have caught it.
#[cfg(unix)]
fn engine() -> Result<Arc<dyn SpeechEngine>> {
    use voicecast_engine::{EspeakEngine, PiperEngine};

    // Same order as the app: Piper if installed, espeak as the floor.
    match PiperEngine::discover() {
        Ok(piper) => Ok(Arc::new(piper)),
        Err(e) => {
            eprintln!("piper unavailable: {e}");
            Ok(Arc::new(EspeakEngine::new().context(
                "espeak-ng is not available. On Arch: sudo pacman -S espeak-ng",
            )?))
        }
    }
}

/// Refused rather than silent.
///
/// Windows has no engine wired yet, and a daemon that accepts messages and
/// says nothing is worse than one that will not start: the sender is told
/// "queued" and nothing ever happens. The app is the node on Windows anyway.
#[cfg(not(unix))]
fn engine() -> Result<Arc<dyn SpeechEngine>> {
    anyhow::bail!(
        "no speech engine on this platform yet; run the voicecast app instead, \
         which is the node here"
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let store = DesktopKeyStore::new().context("locating a key store")?;
    let identity = Identity::load_or_create(&store).context("loading device identity")?;

    let engine = engine()?;

    eprintln!("device id: {}", identity.id());
    eprintln!("key store: {}", identity.location());

    // The device's label. A local convenience only — identity is the key.
    let name = voicecast_core::device_name();

    let transport = Transport::bind(identity.secret().clone(), None)
        .await
        .context("binding the peer-to-peer endpoint")?;

    let node = Arc::new(Node::new(engine, identity, transport, name).await?);
    node.start_presence_checks();
    node.serve().await
}
