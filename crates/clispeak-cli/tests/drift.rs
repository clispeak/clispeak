//! The copies this crate keeps of `clispeak-core`, held up against the
//! originals.
//!
//! **Why they are copies at all.** `clispeak-cli` depends on
//! `clispeak-proto` and `clispeak-text` and nothing else, deliberately: that
//! is what keeps startup at about 3ms, which is the whole premise of the
//! thin-client design. Importing the node's crate to share four constants and
//! one wire format would pull in iroh, its transitive dependencies and the
//! whole of the roster, queue and policy code, to save a page of duplication.
//! The duplication is the better trade and `CLAUDE.md` says so.
//!
//! **Why this file exists.** The trade has a bill and nobody had ever been
//! handed it. The two copies were in step, and if they ever stopped, the
//! symptom would arrive somewhere else entirely — that is not a guess, it has
//! already happened once. Forgetting the environment override in the socket
//! name made every command talk to the first node on the machine, "which
//! looked like two unrelated bugs" (#80).
//!
//! `clispeak-core` is a `[dev-dependencies]` entry, so it is linked into this
//! test and into nothing else. The shipped binary and its startup are exactly
//! as they were.
//!
//! **What a failure here means.** Not that this crate is wrong — that the two
//! are no longer the same. Read both, decide which is right, and change the
//! other. The originals are `clispeak_core::ipc` and
//! `clispeak_core::transport`.

use clispeak_cli::{frame, mirror};
use clispeak_proto::{Priority, Request, Response, Status};

/// What a probe run prints, for the parent process to compare.
const PROBE: &str = "CLISPEAK_DRIFT_PROBE";

/// Prints both copies' answers, when asked to by the test below.
///
/// A separate process is the only honest way to do this. Both functions read
/// a process-wide environment variable, `std::env::set_var` is `unsafe`, and
/// this workspace *forbids* `unsafe` — a lint that cannot be overridden by an
/// `allow`, and should not be worked around for a convenience. So the
/// variable is set where setting it is safe: on a child, before it starts.
///
/// It also removes the other reason to distrust an in-process version. Two
/// tests manipulating the same variable at once is a race, and an
/// intermittently failing drift check is worse than none.
#[test]
fn drift_probe() {
    if std::env::var(PROBE).is_err() {
        return;
    }
    println!(
        "PROBE socket {} {}",
        mirror::socket_name(),
        clispeak_core::ipc::socket_name()
    );
    println!(
        "PROBE config {} {}",
        clispeak_cli::config::dir().expect("a config dir").display(),
        clispeak_core::config_dir().expect("a config dir").display()
    );
}

/// Run this test binary again with the environment set, and read back what
/// the two copies said.
fn probe(vars: &[(&str, &str)]) -> Vec<(String, String, String)> {
    let mut cmd =
        std::process::Command::new(std::env::current_exe().expect("the path to this test binary"));
    cmd.args(["--exact", "drift_probe", "--nocapture"])
        .env(PROBE, "1");
    for (k, v) in vars {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("running the probe");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let rows: Vec<(String, String, String)> = text
        .lines()
        .filter_map(|l| l.strip_prefix("PROBE "))
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((
                it.next()?.to_string(),
                it.next()?.to_string(),
                it.next()?.to_string(),
            ))
        })
        .collect();
    assert!(
        !rows.is_empty(),
        "the probe printed nothing; it exited {:?} with {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    rows
}

/// The socket name, with and without its override.
///
/// **The override is the half that has actually broken.** Both copies return
/// the same default whether or not either reads the environment, so a test
/// that never sets the variable would pass against a CLI that ignores it
/// entirely — which is the bug that happened: every command talked to the
/// first node on the machine, and it looked like two unrelated bugs.
#[test]
fn the_socket_name_agrees_including_its_override() {
    for (label, cli, node) in probe(&[("CLISPEAK_SOCKET", "vc-2.sock")]) {
        if label != "socket" {
            continue;
        }
        assert_eq!(
            cli, node,
            "the socket name has drifted; a CLI looking for one name while \
             the node holds another reports no node at all, with the node \
             running perfectly"
        );
        assert_eq!(cli, "vc-2.sock", "neither copy read the override");
    }
}

/// The config directory, with its override.
///
/// This is where the token lives, so disagreeing means the CLI cannot prove
/// itself to a node that is answering it — which reads as the node being
/// broken.
#[test]
fn the_config_directory_agrees_including_its_override() {
    let scratch = std::env::temp_dir().join("clispeak-drift-config");
    let want = scratch.display().to_string();
    for (label, cli, node) in probe(&[("CLISPEAK_CONFIG_DIR", &want)]) {
        if label != "config" {
            continue;
        }
        assert_eq!(cli, node, "the config directory has drifted");
        assert_eq!(cli, want, "neither copy read the override");
    }
}

/// And the defaults agree too, with nothing set.
#[test]
fn the_defaults_agree_with_nothing_set() {
    for (label, cli, node) in probe(&[]) {
        assert_eq!(cli, node, "the {label} default has drifted");
    }
}

/// One timeout, two crates.
///
/// The node applies the bound and is the one that times out, because it knows
/// *why* and can answer `unreachable` with a reason. That only works while
/// the CLI waits longer than the node does. When this crate's copy was
/// shorter, a speak to an unreachable device reported that the *local* node
/// had never answered and told the reader to restart a healthy app (#151).
#[test]
fn the_peer_connect_bound_agrees() {
    assert_eq!(
        mirror::PEER_CONNECT,
        clispeak_core::transport::PEER_CONNECT,
        "if the node's bound moves, this one moves with it — being shorter \
         than the node's is the bug"
    );
}

/// A frame written by one is read by the other, both ways round.
///
/// The shape is a big-endian `u32` length and then CBOR, written twice. This
/// is the only test that would notice either side changing its mind about
/// that.
#[tokio::test]
async fn a_frame_written_by_one_is_read_by_the_other() {
    let request = Request::Speak {
        text: "the build finished".into(),
        priority: Priority::Normal,
        to: Some("Phone".into()),
        wait: false,
        timeout_secs: None,
        voice: None,
    };

    // CLI writes, node reads.
    let mut wire = Vec::new();
    frame::write_frame(&mut wire, &request)
        .await
        .expect("the CLI writes a request");
    let seen: Request = clispeak_core::ipc::read_frame(&mut wire.as_slice())
        .await
        .expect("the node reads what the CLI wrote");
    assert_eq!(
        format!("{seen:?}"),
        format!("{request:?}"),
        "a request did not survive the crossing"
    );

    // Node writes, CLI reads. The reply direction is the one an agent sees.
    let response = Response::Finished {
        status: Status::Spoken,
    };
    let mut wire = Vec::new();
    clispeak_core::ipc::write_frame(&mut wire, &response)
        .await
        .expect("the node writes a response");
    let seen: Response = frame::read_frame(&mut wire.as_slice())
        .await
        .expect("the CLI reads what the node wrote");
    assert_eq!(
        format!("{seen:?}"),
        format!("{response:?}"),
        "a response did not survive the crossing"
    );
}

/// Both refuse the same absurd frame.
///
/// The cap exists so neither side trusts a length and allocates for it. Two
/// caps that disagree is one side refusing what the other is happily
/// building.
#[tokio::test]
async fn both_refuse_a_frame_that_is_too_large() {
    // A length prefix and nothing behind it: whichever limit is lower decides
    // this, and the point is that they decide it the same way.
    let absurd = (u32::MAX / 2).to_be_bytes();

    let cli: Result<Request, _> = frame::read_frame(&mut absurd.as_slice()).await;
    let node: Result<Request, _> = clispeak_core::ipc::read_frame(&mut absurd.as_slice()).await;
    assert!(cli.is_err() && node.is_err(), "both must refuse it");

    let cli = cli.unwrap_err().to_string();
    let node = node.unwrap_err().to_string();
    assert_eq!(
        cli, node,
        "the frame caps have drifted, so one side refuses what the other \
         sends: {cli} vs {node}"
    );
}

/// The token the node installs is the token this crate reads.
///
/// The file name, the length and the directory all have to match. A
/// disagreement here is a CLI that cannot prove itself to a node that is
/// answering it — which reads as the node being broken.
#[test]
fn the_token_is_written_where_the_cli_looks_for_it() {
    let dir = std::env::temp_dir().join(format!("clispeak-token-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");

    let installed = clispeak_core::ipc::install_token(&dir).expect("the node installs one");
    let read = frame::read_token(&dir).expect("the CLI finds it");
    assert_eq!(installed, read, "the token did not survive the crossing");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The handshake this crate offers is the one the node accepts.
///
/// The node answers first, because the local socket is a name any other user
/// on this machine can take: speaking first would hand text, invites and
/// history to whoever got there first (#54). Both halves — the labels, the
/// nonce length, the order — are written out twice, and only this runs them
/// against each other.
#[tokio::test(flavor = "multi_thread")]
async fn the_handshake_halves_fit_together() {
    let token = [7u8; 32];
    let (mut caller, mut node) = tokio::io::duplex(4096);

    let serving =
        tokio::spawn(async move { clispeak_core::ipc::accept_handshake(&mut node, &token).await });
    frame::offer_handshake(&mut caller, &token)
        .await
        .expect("the CLI's half of the handshake");
    serving
        .await
        .expect("the node's task")
        .expect("the node's half of the handshake");
}

/// And it fails when the tokens differ, so the test above is not vacuous.
///
/// Only the caller's verdict is awaited. The node's half is aborted rather
/// than joined: a caller that walks away mid-handshake is exactly what this
/// is, and the node answers that by waiting out its five second handshake
/// timeout. Joining it spent those five seconds in every run of the suite, on
/// every machine, to learn something the caller had already said.
#[tokio::test(flavor = "multi_thread")]
async fn the_handshake_refuses_a_different_token() {
    let (mut caller, mut node) = tokio::io::duplex(4096);
    let serving =
        tokio::spawn(
            async move { clispeak_core::ipc::accept_handshake(&mut node, &[1u8; 32]).await },
        );

    let offered = frame::offer_handshake(&mut caller, &[2u8; 32]).await;
    serving.abort();
    let refusal = offered.expect_err("a caller holding the wrong token was let through");
    assert!(
        refusal.to_string().contains("token"),
        "the reason has to say what did not match, since something else \
         holding the socket and a token from a different node look identical \
         from here: {refusal}"
    );
}
