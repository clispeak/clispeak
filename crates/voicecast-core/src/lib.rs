//! The node: transport, membership, queue, playback.
//!
//! Compiles to all five targets and contains **no** `#[cfg(target_os)]` —
//! platform code belongs in `voicecast-engine` or the Tauri shell. CI enforces
//! this; see `docs/build-plan.md`.
//!
//! Built out from M3 onward.

pub mod identity;
pub mod ipc;
mod node;
pub mod roster;
pub mod ticket;
pub mod transport;

pub use identity::{
    FileKeyStore, Identity, IdentityError, KeyStore, config_dir, device_name, set_config_dir,
    set_device_name,
};
pub use node::{Node, WindowHook};
pub use roster::{Roster, RosterError, verify};
pub use ticket::Ticket;
pub use transport::Transport;
pub use voicecast_proto::Member;

/// This crate's version, reported by `voicecast status`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
