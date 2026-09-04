//! The node: transport, membership, queue, playback.
//!
//! Compiles to all five targets and contains **no** `#[cfg(target_os)]` —
//! platform code belongs in `clispeak-engine` or the Tauri shell. CI enforces
//! this; see `docs/build-plan.md`.
//!
//! Built out from M3 onward.

pub mod history;
pub mod identity;
pub mod ipc;
mod node;
pub mod policy;
mod queue;
pub mod roster;
pub mod spaces;
pub mod store;
pub mod ticket;
pub mod transport;

pub use clispeak_proto::Member;
pub use history::{Entry, History};
pub use identity::{
    FileKeyStore, Identity, IdentityError, KeyStore, config_dir, device_name, device_name_or,
    load_voice_settings, save_voice_settings, set_config_dir, set_device_name,
};
pub use identity::{migrate_from, migrate_from_previous_name};
pub use node::{Node, WindowHook};
pub use policy::{Policies, Policy, QuietHours};
pub use roster::{Roster, RosterError, verify};
pub use spaces::{SpaceInfo, Spaces};
pub use ticket::{Ticket, qr_svg};
pub use transport::Transport;

/// This crate's version, reported by `clispeak status`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
