//! The node process.
//!
//! Temporary: from M6 the Tauri app is the long-running process and simply
//! links `voicecast-core`. Until that exists, this stands in so the CLI has
//! something to talk to.

use std::sync::Arc;

use anyhow::{Context, Result};
use voicecast_core::Node;
use voicecast_engine::EspeakEngine;

#[tokio::main]
async fn main() -> Result<()> {
    let engine = EspeakEngine::new()
        .context("espeak-ng is not available. On Arch: sudo pacman -S espeak-ng")?;
    Node::new(Arc::new(engine)).serve().await
}
