//! The node process.
//!
//! Temporary: from M6 the Tauri app is the long-running process and links
//! `voicecast-core` directly. Until that exists, this stands in so the CLI
//! has something to talk to.

use std::sync::Arc;

use anyhow::{Context, Result};
use voicecast_core::{Identity, Node, Transport};
use voicecast_engine::{EngineError, PiperEngine, SpeechEngine};
// The floor engine differs by platform, and only one of them exists per
// build; importing both unconditionally is what broke Windows last time.
#[cfg(unix)]
use voicecast_engine::EspeakEngine;
#[cfg(not(unix))]
use voicecast_engine::SilentEngine;
use voicecast_keystore::DesktopKeyStore;

/// The best engine this platform has.
///
/// Piper first everywhere. It is the engine every desktop is meant to use,
/// and running the same one on all of them is what makes a message sound the
/// same wherever it lands.
fn engine() -> Result<Arc<dyn SpeechEngine>> {
    match PiperEngine::discover() {
        Ok(piper) => Ok(Arc::new(piper)),
        Err(e) => {
            eprintln!("piper unavailable: {e}");
            fallback(e)
        }
    }
}

/// What speaks when Piper does not.
///
/// espeak-ng is the Unix floor, and a device is never silent while it is
/// there — see decision 17.
#[cfg(unix)]
fn fallback(_piper: EngineError) -> Result<Arc<dyn SpeechEngine>> {
    Ok(Arc::new(EspeakEngine::new().context(
        "espeak-ng is not available. On Arch: sudo pacman -S espeak-ng",
    )?))
}

/// Windows has no floor engine, so it carries the reason instead.
///
/// This used to refuse to start — decision 22 — which was right while Windows
/// had no engine at all: the only alternative then was a node that accepted
/// messages, reported `queued`, and said nothing. Piper runs here now, so the
/// choice is no longer between silence and refusal. Refusing would take a
/// node off the network over an install that can be fixed, and the sender
/// would learn no more from a daemon that is absent than from one that
/// explains itself.
///
/// Piper's own error is what travels, because it is the part that names
/// something a person can act on. A device that cannot speak still joins
/// spaces and still answers for itself.
#[cfg(not(unix))]
fn fallback(piper: EngineError) -> Result<Arc<dyn SpeechEngine>> {
    Ok(Arc::new(SilentEngine::new(piper.to_string())))
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
