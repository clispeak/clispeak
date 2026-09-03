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
#[cfg(target_os = "linux")]
use voicecast_engine::EspeakEngine;
#[cfg(not(target_os = "linux"))]
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

/// What speaks when Piper does not, on Linux.
///
/// espeak-ng is the *Linux* floor, and a device is never silent while it is
/// there — decision 17, which says Linux and means it. This gate used to read
/// `unix`, which handed macOS a floor it has never had: nothing on a Mac
/// installs espeak-ng, so the branch only ever reached its own error path,
/// and that error recommended an Arch package.
///
/// Both reasons travel. Piper is the engine this platform is meant to use, so
/// why *it* failed is the actionable half; espeak's failure arrives as the
/// cause. The package manager goes unnamed because this one binary runs on
/// every distribution.
#[cfg(target_os = "linux")]
fn fallback(piper: EngineError) -> Result<Arc<dyn SpeechEngine>> {
    Ok(Arc::new(EspeakEngine::new().with_context(|| {
        format!(
            "no speech engine is available. Piper: {piper}. \
             espeak-ng is the floor here and is also missing — install it with \
             your distribution's package manager"
        )
    })?))
}

/// Everywhere else, the reason travels and the node still starts.
///
/// Windows and macOS have no floor engine: Windows never had one, and the
/// macOS bundle carries only Piper. This used to refuse to start on Windows —
/// decision 22 — which was right while Windows had no engine at all: the only
/// alternative then was a node that accepted messages, reported `queued`, and
/// said nothing. Piper runs on both now, so the choice is no longer between
/// silence and refusal. Refusing would take a node off the network over an
/// install that can be fixed, and the sender would learn no more from a
/// daemon that is absent than from one that explains itself.
///
/// macOS reached the Linux branch until now and so refused to start, which was
/// never a decision anybody made — see decision 30.
///
/// Piper's own error is what travels, because it is the part that names
/// something a person can act on. A device that cannot speak still joins
/// spaces and still answers for itself.
#[cfg(not(target_os = "linux"))]
fn fallback(piper: EngineError) -> Result<Arc<dyn SpeechEngine>> {
    Ok(Arc::new(SilentEngine::new(piper.reason())))
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
