//! The node process.
//!
//! Temporary: from M6 the Tauri app is the long-running process and links
//! `voicecast-core` directly. Until that exists, this stands in so the CLI
//! has something to talk to.

use anyhow::{Context, Result};
use voicecast_core::{Identity, Node, Transport};
use voicecast_engine::EspeakEngine;

use voicecast_keystore::DesktopKeyStore;

#[tokio::main]
async fn main() -> Result<()> {
    let store = DesktopKeyStore::new().context("locating a key store")?;
    let identity = Identity::load_or_create(&store).context("loading device identity")?;

    // Same order as the app: Piper if installed, espeak as the floor.
    let engine: std::sync::Arc<dyn voicecast_engine::SpeechEngine> =
        match voicecast_engine::PiperEngine::discover() {
            Ok(piper) => std::sync::Arc::new(piper),
            Err(e) => {
                eprintln!("piper unavailable: {e}");
                std::sync::Arc::new(
                    EspeakEngine::new()
                        .context("espeak-ng is not available. On Arch: sudo pacman -S espeak-ng")?,
                )
            }
        };

    eprintln!("device id: {}", identity.id());
    eprintln!("key store: {}", identity.location());

    // The device's label. A local convenience only — identity is the key.
    let name = voicecast_core::device_name();

    let transport = Transport::bind(identity.secret().clone(), None)
        .await
        .context("binding the peer-to-peer endpoint")?;

    let node = std::sync::Arc::new(Node::new(engine, identity, transport, name).await?);
    node.start_presence_checks();
    node.serve().await
}
