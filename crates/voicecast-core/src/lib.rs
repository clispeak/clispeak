//! The node: transport, membership, queue, playback.
//!
//! Compiles to all five targets and contains **no** `#[cfg(target_os)]` —
//! platform code belongs in `voicecast-engine` or the Tauri shell. CI enforces
//! this; see `docs/build-plan.md`.
//!
//! Built out from M3 onward.

pub mod ipc;
mod node;

pub use node::Node;

/// This crate's version, reported by `voicecast status`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
