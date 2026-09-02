//! The `voicecast` binary.
//!
//! Deliberately thin: validate, hand the text to the local node, exit. It
//! depends on `voicecast-proto` and `voicecast-text` but never on
//! `voicecast-core`, which is what keeps startup in single-digit
//! milliseconds — the whole premise of the thin-client design.

use std::io::{IsTerminal, Read};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use interprocess::local_socket::{
    GenericNamespaced, ToNsName,
    tokio::{Stream, prelude::*},
};
use voicecast_proto::{Priority, Request, Response, Status};

mod exit {
    //! Exit codes, as specified in `docs/cli.md`.
    //!
    //! Distinct codes matter here because the caller is an agent: "my text was
    //! wrong" and "the device is offline" need different responses, and an
    //! agent can only tell them apart if we say so.

    /// Accepted, or spoken if the caller waited.
    pub const OK: u8 = 0;
    /// Usage or configuration error.
    pub const USAGE: u8 = 1;
    /// The local node could not be reached or started.
    pub const NO_NODE: u8 = 5;
    /// Every target failed to speak the message.
    pub const ALL_FAILED: u8 = 4;
    /// Text rejected: markdown or a URL that will not read well aloud.
    pub const REJECTED: u8 = 6;
}

#[derive(Parser)]
#[command(
    name = "voicecast",
    about = "Speak text aloud on your devices",
    version
)]
struct Cli {
    /// Device to speak on. Defaults to this machine.
    #[arg(short, long, global = true)]
    to: Option<String>,

    /// Urgency. `high` interrupts whatever is speaking.
    #[arg(short, long, value_enum, default_value = "normal", global = true)]
    priority: Prio,

    /// Convert markdown to speakable text instead of rejecting it.
    #[arg(long, global = true)]
    strip: bool,

    /// Speak exactly as given, skipping validation entirely.
    #[arg(long, global = true)]
    raw: bool,

    #[command(subcommand)]
    command: Option<Command>,

    /// Text to speak. Reads stdin when absent.
    ///
    /// Hyphen values are allowed because speech legitimately starts with one
    /// — "- item one", "-5 degrees" — and clap would otherwise read them as
    /// unknown flags.
    #[arg(allow_hyphen_values = true)]
    text: Vec<String>,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Prio {
    Low,
    Normal,
    High,
}

impl From<Prio> for Priority {
    fn from(p: Prio) -> Self {
        match p {
            Prio::Low => Priority::Low,
            Prio::Normal => Priority::Normal,
            Prio::High => Priority::High,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Speak text. Only needed to say a word that is also a subcommand.
    Say {
        /// Text to speak.
        #[arg(allow_hyphen_values = true)]
        text: Vec<String>,
    },
    /// Stop speaking and clear the queue.
    Stop,
    /// Show node health.
    Status,
    /// Print an invite for another device to join this space.
    Invite,
    /// Join a space using an invite from another device.
    Join {
        /// The `voicecast://join/...` string.
        ticket: String,
    },
    /// List devices in this space.
    Devices,
    /// Bring the app window back into view.
    Show,
    /// Shut the local node down.
    Quit,
    /// Change this device's name.
    Rename {
        /// The new name.
        name: String,
    },
}

/// Write a line to stdout, tolerating a closed pipe.
///
/// `println!` panics on EPIPE, so `voicecast ... | head -1` exited 101 —
/// Rust's panic code — instead of the exit code the caller needs to read.
/// An agent piping our output should still get 6 for "bad text".
fn out(s: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "{s}");
}

/// Write a line to stderr, tolerating a closed pipe. See [`out`].
fn err(s: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{s}");
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            err(&format!("error: {e:#}"));
            ExitCode::from(exit::USAGE)
        }
    }
}

async fn run() -> anyhow::Result<u8> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return Ok(match e.kind() {
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
                    exit::OK
                }
                _ => exit::USAGE,
            });
        }
    };

    let request = match &cli.command {
        Some(Command::Stop) => Request::Stop,
        Some(Command::Status) => Request::Status,
        Some(Command::Invite) => Request::Invite,
        Some(Command::Devices) => Request::Devices,
        Some(Command::Show) => Request::Show,
        Some(Command::Quit) => Request::Quit,
        Some(Command::Rename { name }) => Request::Rename { name: name.clone() },
        Some(Command::Join { ticket }) => Request::Join {
            ticket: ticket.clone(),
        },
        Some(Command::Say { text }) => match build_speak(&cli, text)? {
            Ok(req) => req,
            Err(code) => return Ok(code),
        },
        None => {
            let tokens = if cli.text.is_empty() {
                vec![read_stdin()?]
            } else {
                cli.text.clone()
            };
            match build_speak(&cli, &tokens)? {
                Ok(req) => req,
                Err(code) => return Ok(code),
            }
        }
    };

    send(request).await
}

/// Reject text that is almost certainly a mistyped flag.
///
/// `allow_hyphen_values` is needed so ordinary speech like "- item one" or
/// "-5 degrees" works, but it also makes clap accept `--priorty` as text —
/// and silently *speaking* a typo'd flag is a worse failure than refusing it.
///
/// The Unix convention settles it: after an explicit `--` separator anything
/// goes, and without one a token shaped like a long flag is treated as an
/// error. Checked against the raw arguments because clap consumes `--`.
fn flaglike_token(tokens: &[String]) -> Option<String> {
    if std::env::args().any(|a| a == "--") {
        return None;
    }
    tokens
        .iter()
        .find(|t| t.starts_with("--") && t.len() > 2)
        .cloned()
}

/// Validate and wrap text, or print a rejection and return its exit code.
fn build_speak(cli: &Cli, tokens: &[String]) -> anyhow::Result<Result<Request, u8>> {
    if let Some(flag) = flaglike_token(tokens) {
        err(&format!("error: unknown option '{flag}'"));
        err("");
        err("If you meant to speak it, use the -- separator:");
        err(&format!("  voicecast -- {flag} ..."));
        return Ok(Err(exit::USAGE));
    }

    let text = tokens.join(" ").trim().to_string();
    if text.is_empty() {
        err("error: nothing to say");
        return Ok(Err(exit::USAGE));
    }

    let text = if cli.raw {
        text
    } else if cli.strip {
        voicecast_text::strip(&text)
    } else {
        match voicecast_text::validate(&text) {
            Ok(()) => text,
            Err(rejection) => {
                print_rejection(&text, &rejection);
                return Ok(Err(exit::REJECTED));
            }
        }
    };

    Ok(Ok(Request::Speak {
        text,
        priority: cli.priority.into(),
        to: cli.to.clone(),
    }))
}

/// Print the offending span, then a rewrite the caller can resend verbatim.
///
/// The suggestion is the point. An agent that can only be told "no" will
/// guess; one handed replacement text can simply send it.
fn print_rejection(text: &str, rejection: &voicecast_text::Rejection) {
    let (start, end) = rejection.span();
    err(&format!("error: {rejection}"));
    err("");

    // Show the offending line with a caret run beneath the span.
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[end..].find('\n').map_or(text.len(), |i| end + i);
    let line = &text[line_start..line_end];
    err(&format!("  {line}"));
    let pad = text[line_start..start].chars().count();
    let width = text[start..end].chars().count().max(1);
    err(&format!(
        "  {}{} {}",
        " ".repeat(pad),
        "^".repeat(width),
        rejection.kind()
    ));
    err("");

    let suggestion = voicecast_text::strip(text);
    if !suggestion.is_empty() && suggestion != text {
        err("Write text as it should be spoken:");
        err(&format!("  {suggestion:?}"));
        err("");
    }
    err("Or pass --strip to convert automatically.");
}

/// Read all of stdin, refusing to hang on an interactive terminal.
fn read_stdin() -> anyhow::Result<String> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        anyhow::bail!("no text given, and stdin is a terminal");
    }
    let mut buf = String::new();
    stdin.lock().read_to_string(&mut buf)?;
    Ok(buf)
}

/// Send one request to the local node and report what came back.
async fn send(request: Request) -> anyhow::Result<u8> {
    // Whether a non-spoken outcome counts as failure depends on what was
    // asked: `Cancelled` is the point of `stop`, but a failure for `speak`.
    let was_speak = matches!(request, Request::Speak { .. });
    let name = voicecast_core_socket_name().to_ns_name::<GenericNamespaced>()?;

    let mut stream = match Stream::connect(name).await {
        Ok(s) => s,
        Err(e) => {
            err(&format!("error: no voicecast node is running ({e})"));
            err("start one with: voicecastd");
            return Ok(exit::NO_NODE);
        }
    };

    write_frame(&mut stream, &request).await?;
    let response: Response = read_frame(&mut stream).await?;

    Ok(match response {
        Response::Accepted { msg_id } => {
            out(&msg_id);
            exit::OK
        }
        Response::Finished { status } => {
            out(&format!("{status:?}"));
            // A message that was not spoken is a failure the caller needs to
            // see. `Rejected` in particular means this device is not in the
            // target's roster — silently exiting 0 would hide that.
            match status {
                _ if !was_speak => exit::OK,
                Status::Spoken | Status::Queued | Status::Speaking => exit::OK,
                _ => exit::ALL_FAILED,
            }
        }
        Response::Status {
            device_id,
            key_store,
            engine,
            fallback,
            queued,
        } => {
            out(&format!("device:  {device_id}"));
            out(&format!("keys:    {key_store}"));
            out(&format!(
                "engine:  {engine}{}",
                if fallback { "  (fallback)" } else { "" }
            ));
            out(&format!("queued:  {queued}"));
            exit::OK
        }
        Response::Invite { url, expires_in } => {
            out(&url);
            err(&format!(
                "\nExpires in {}m {}s. Single use.",
                expires_in / 60,
                expires_in % 60
            ));
            err("On the other device:  voicecast join <the line above>");
            exit::OK
        }
        Response::Done => exit::OK,
        Response::Renamed { name } => {
            out(&format!("renamed to {name}"));
            err("Other devices keep the old name until they sync.");
            exit::OK
        }
        Response::Joined { members } => {
            out(&format!("joined. {members} devices in this space"));
            exit::OK
        }
        Response::Devices { devices } => {
            for d in devices {
                out(&format!(
                    "{:<16} {}{}",
                    d.name,
                    &d.endpoint_id[..16.min(d.endpoint_id.len())],
                    if d.is_self { "  (this device)" } else { "" }
                ));
            }
            exit::OK
        }
        Response::Error { message } => {
            err(&format!("error: {message}"));
            exit::USAGE
        }
    })
}

/// Socket name, duplicated rather than depending on `voicecast-core`.
///
/// A handful of bytes of duplication is a fair price for keeping this binary
/// free of the node's entire dependency graph — but it must stay in step with
/// `voicecast_core::ipc::socket_name`, including the environment override.
/// Forgetting that here made every command talk to the first node, which
/// looked like two unrelated bugs.
fn voicecast_core_socket_name() -> String {
    std::env::var("VOICECAST_SOCKET").unwrap_or_else(|_| "voicecast.sock".to_string())
}

// Frame helpers, mirroring `voicecast_core::ipc` for the same reason.
mod frame;
use frame::{read_frame, write_frame};
