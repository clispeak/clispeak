//! The node process.
//!
//! Temporary: from M6 the Tauri app is the long-running process and links
//! `voicecast-core` directly. Until that exists, this stands in so the CLI
//! has something to talk to.

use std::sync::Arc;

use anyhow::{Context, Result};
use voicecast_core::{Identity, Node};
use voicecast_engine::EspeakEngine;

mod keystore;
use keystore::DesktopKeyStore;

#[tokio::main]
async fn main() -> Result<()> {
    let store = DesktopKeyStore::new().context("locating a key store")?;
    let identity = Identity::load_or_create(&store).context("loading device identity")?;

    let engine = EspeakEngine::new()
        .context("espeak-ng is not available. On Arch: sudo pacman -S espeak-ng")?;

    eprintln!("device id: {}", identity.id());
    eprintln!("key store: {}", identity.location());

    Node::new(Arc::new(engine), identity).serve().await
}
