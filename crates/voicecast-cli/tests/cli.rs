//! CLI behaviour that is easy to regress silently.
//!
//! Only exercises paths that do **not** need a running node: validation and
//! argument parsing happen before we ever open a socket, so these stay
//! deterministic whether or not a daemon is up.

use std::io::Read;
use std::process::{Command, Stdio};

/// Path to the built binary, provided by cargo for integration tests.
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_voicecast")
}

/// Exit code from running the binary with these arguments.
fn code(args: &[&str]) -> i32 {
    Command::new(bin())
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("failed to run voicecast")
        .code()
        .expect("killed by signal")
}

#[test]
fn markdown_is_rejected_with_its_own_code() {
    // 6 rather than a generic failure, so an agent can tell "my text was
    // wrong" from "the device is offline" and respond differently.
    assert_eq!(code(&["Updated **3 files**."]), 6);
    assert_eq!(code(&["See `foo.rs` now."]), 6);
    assert_eq!(code(&["Go to https://example.com now."]), 6);
}

#[test]
fn empty_text_is_a_usage_error() {
    assert_eq!(code(&[""]), 1);
    assert_eq!(code(&["   "]), 1);
}

#[test]
fn unknown_flags_are_refused_rather_than_spoken() {
    // The dangerous failure: `allow_hyphen_values` is needed so "-5 degrees"
    // works, but it also lets clap accept `--priorty` as text, which would
    // speak the typo aloud instead of reporting it.
    assert_eq!(code(&["--nonsense", "x"]), 1);
    assert_eq!(code(&["--priorty", "high", "x"]), 1);
}

#[test]
fn help_and_version_succeed() {
    assert_eq!(code(&["--help"]), 0);
    assert_eq!(code(&["--version"]), 0);
}

#[test]
fn a_closed_pipe_does_not_become_a_panic() {
    // `println!` panics on EPIPE, which turned `voicecast ... | head -1` into
    // exit 101 — Rust's panic code — and hid the real result from anything
    // piping our output.
    let mut child = Command::new(bin())
        .arg("Updated **3 files**.")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run voicecast");

    // Read one byte, then drop both pipes while the child is still writing.
    if let Some(mut e) = child.stderr.take() {
        let mut one = [0u8; 1];
        let _ = e.read(&mut one);
    }
    drop(child.stdout.take());

    let status = child.wait().expect("wait failed");
    assert_eq!(status.code(), Some(6), "expected 6, not a panic");
}
