//! The `voicecast` binary.
//!
//! Deliberately thin: it writes to the local node's IPC socket and exits. It
//! depends only on `voicecast-proto` and `voicecast-text`, never on
//! `voicecast-core` — that is what keeps startup in single-digit milliseconds,
//! which is the whole premise of the thin-client design.

fn main() -> anyhow::Result<()> {
    println!("voicecast {} (skeleton)", env!("CARGO_PKG_VERSION"));
    Ok(())
}
