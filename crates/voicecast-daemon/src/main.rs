//! The node process.
//!
//! Temporary: from M6 the Tauri app is the long-running process and links
//! `voicecast-core` directly. Until that exists, this stands in so the CLI
//! has something to talk to.

use std::sync::Arc;

use anyhow::{Context, Result};
use voicecast_core::{Identity, Node, Transport};
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

    // The device's label. A local convenience only — identity is the key.
    let name = std::env::var("VOICECAST_NAME")
        .unwrap_or_else(|_| hostname().unwrap_or_else(|| "this device".to_string()));

    let transport = Transport::bind(identity.secret().clone())
        .await
        .context("binding the peer-to-peer endpoint")?;

    let node = Node::new(Arc::new(engine), identity, transport, name).await?;
    node.serve().await
}

/// This machine's hostname, if it has a usable one.
fn hostname() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
