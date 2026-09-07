//! The parts of the `clispeak` binary that something other than `main` needs.
//!
//! **A library target exists for one reason.** Three things in this crate are
//! copied from `clispeak-core` by hand — the IPC framing, the socket name and
//! config path, and two timeouts — because depending on the node's crate
//! would pull its whole dependency graph into a binary whose entire purpose
//! is starting instantly. They were in step, and nothing could have said when
//! they stopped being, because a binary crate exposes nothing to a test
//! (#80).
//!
//! Nothing here changes what the binary does or what it links against. A
//! `[dev-dependencies]` entry on `clispeak-core` lets `tests/drift.rs`
//! compare the two copies; dev-dependencies do not reach the shipped binary.

pub mod config;
pub mod frame;
pub mod mirror;
