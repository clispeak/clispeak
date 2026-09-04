//! The node process.
//!
//! Temporary: from M6 the Tauri app is the long-running process and links
//! `clispeak-core` directly. Until that exists, this stands in so the CLI
//! has something to talk to.

use std::sync::Arc;

use anyhow::{Context, Result};
use clispeak_core::{Identity, Node, Transport};
use clispeak_engine::SpeechEngine;
use clispeak_keystore::DesktopKeyStore;

/// The best engine this platform has.
///
/// **One arm per platform, matching the app**, which is the point: a daemon
/// and an app on the same machine that chose different engines would be two
/// devices disagreeing about how the machine sounds. This used to be "Piper
/// first everywhere", and the comment said running the same engine on every
/// desktop is what makes a message sound the same wherever it lands. That
/// stopped being true on macOS with decision 91 and on Windows with the
/// platform engine here — and on macOS it had gone quietly wrong, because
/// `xtask bundle` no longer stages Piper there, so this asked for something
/// the bundle had stopped carrying.
#[cfg(any(target_os = "ios", target_os = "macos"))]
fn engine() -> Result<Arc<dyn SpeechEngine>> {
    match clispeak_engine::AppleEngine::new() {
        Ok(engine) => Ok(Arc::new(engine)),
        Err(e) => {
            eprintln!("the platform speech engine did not start: {e}");
            Ok(Arc::new(clispeak_engine::SilentEngine::new(e.reason())))
        }
    }
}

/// SAPI 5, the platform synthesiser (#20, #30, #132).
///
/// `SilentEngine` rather than `Rediscovering`: Piper could appear later
/// because somebody installs it, and SAPI either exists on this machine or
/// the machine is broken in a way that will not fix itself while this
/// process runs.
#[cfg(windows)]
fn engine() -> Result<Arc<dyn SpeechEngine>> {
    match clispeak_engine::WindowsEngine::new() {
        Ok(engine) => Ok(Arc::new(engine)),
        Err(e) => {
            eprintln!("the platform speech engine did not start: {e}");
            Ok(Arc::new(clispeak_engine::SilentEngine::new(e.reason())))
        }
    }
}

/// Piper, which is what Linux and the remaining unixes should sound like.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios"))))]
fn engine() -> Result<Arc<dyn SpeechEngine>> {
    match clispeak_engine::PiperEngine::discover() {
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
fn fallback(piper: clispeak_engine::EngineError) -> Result<Arc<dyn SpeechEngine>> {
    Ok(Arc::new(
        clispeak_engine::EspeakEngine::new().with_context(|| {
            format!(
                // `reason()`, not the error itself: `Display` prepends "no
                // speech engine available", and this sentence already says
                // so. Formatting the whole error here made the message say
                // it twice.
                "no speech engine is available. Piper: {}. \
                 espeak-ng is the floor here and is also missing — install it with \
                 your distribution's package manager",
                piper.reason()
            )
        })?,
    ))
}

/// A unix that is neither Linux nor Apple: the reason travels and the node
/// still starts.
///
/// Keeps looking rather than staying silent for the life of the process: a
/// daemon told "Piper is not installed" should stop saying so once it is
/// (#84). A device that cannot speak still joins spaces and still answers
/// for itself.
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos", target_os = "ios"))
))]
fn fallback(piper: clispeak_engine::EngineError) -> Result<Arc<dyn SpeechEngine>> {
    Ok(Arc::new(clispeak_engine::Rediscovering::new(
        piper.reason(),
        Box::new(|| {
            clispeak_engine::PiperEngine::discover()
                .ok()
                .map(|e| Arc::new(e) as Arc<dyn SpeechEngine>)
        }),
    )))
}

#[tokio::main]
async fn main() -> Result<()> {
    // Said before the call, not after. Reading the key can block for as long
    // as it takes somebody to notice a dialog: on macOS an ad-hoc signature
    // makes every rebuild a different application to the keychain's ACL, so
    // it asks permission to decrypt the existing item, every time. Until now
    // this produced no output at all, so a node parked on that prompt looked
    // identical to one that had never started.
    // Before the identity is looked for, because it is what decides where
    // "the identity" is. Said out loud rather than done quietly: it moves the
    // file that holds every device pairing (decision 82).
    match clispeak_core::migrate_from_previous_name() {
        Ok(moved) if !moved.is_empty() => {
            eprintln!(
                "moved {} from the previous name's directory",
                moved.join(", ")
            );
        }
        Ok(_) => {}
        Err(e) => eprintln!("could not move state from the previous name: {e}"),
    }

    eprintln!("opening the key store…");
    let store = DesktopKeyStore::new().context("locating a key store")?;
    let identity = Identity::load_or_create(&store).context("loading device identity")?;

    let engine = engine()?;

    eprintln!("device id: {}", identity.id());
    eprintln!("key store: {}", identity.location());

    // The device's label. A local convenience only — identity is the key.
    let name = clispeak_core::device_name();

    let transport = Transport::bind(identity.secret().clone(), None)
        .await
        .context("binding the peer-to-peer endpoint")?;

    let node = Arc::new(Node::new(engine, identity, transport, name).await?);
    node.start_presence_checks();
    node.serve().await
}
