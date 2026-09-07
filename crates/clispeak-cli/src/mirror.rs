//! The values this binary copies from `clispeak-core` by hand.
//!
//! `clispeak-cli` depends on `clispeak-proto` and `clispeak-text` and nothing
//! else, deliberately: that is what keeps startup at about 3ms, which is the
//! whole premise of the thin-client design. The cost is that a handful of
//! constants and one wire format exist twice and are kept in step by a
//! person remembering to.
//!
//! Gathering them here does not make them one copy. It makes the copies
//! *findable*, and it puts them somewhere a test can reach — see
//! `tests/drift.rs`, which compares each one against the original through a
//! dev-dependency. A dev-dependency does not reach the shipped binary or its
//! startup, so the trade this file exists to protect is untouched.

/// How long the node spends reaching a device before calling it unreachable.
///
/// `clispeak_core::transport::PEER_CONNECT`, duplicated by hand.
///
/// The node applies the bound and is the one that times out, because it knows
/// *why* and can answer `unreachable` with a reason. That only works if the
/// CLI waits longer than the node does — and it did not: the dial fell
/// through to iroh's own timeout at about thirty seconds while the CLI gave
/// up at ten, so a speak to an unreachable device reported that the local
/// node had never answered and told the reader to restart a healthy app
/// (#151).
///
/// Ninety seconds, measured rather than guessed: a real phone answered in
/// 2.1s warm and 58s cold. See the constant in `clispeak-core` for why a cold
/// phone is the ordinary case here rather than an edge one.
///
/// Being *shorter* than the node's bound is the bug. `tests/drift.rs` asserts
/// they are equal.
pub const PEER_CONNECT: std::time::Duration = std::time::Duration::from_secs(90);

/// The name of this device's local socket.
///
/// `clispeak_core::ipc::socket_name`, duplicated by hand, **including the
/// environment override**. Forgetting that here made every command talk to
/// the first node, which looked like two unrelated bugs.
pub fn socket_name() -> String {
    std::env::var("CLISPEAK_SOCKET").unwrap_or_else(|_| "clispeak.sock".to_string())
}
