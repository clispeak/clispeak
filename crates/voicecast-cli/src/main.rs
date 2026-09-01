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
use voicecast_proto::{Priority, Request, Response};

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
        text: Vec<String>,
    },
    /// Stop speaking and clear the queue.
    Stop,
    /// Show node health.
    Status,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(exit::USAGE)
        }
    }
}

async fn run() -> anyhow::Result<u8> {
    let cli = Cli::parse();

    let request = match &cli.command {
        Some(Command::Stop) => Request::Stop,
        Some(Command::Status) => Request::Status,
        Some(Command::Say { text }) => match build_speak(&cli, text.join(" "))? {
            Ok(req) => req,
            Err(code) => return Ok(code),
        },
        None => {
            let text = if cli.text.is_empty() {
                read_stdin()?
            } else {
                cli.text.join(" ")
            };
            match build_speak(&cli, text)? {
                Ok(req) => req,
                Err(code) => return Ok(code),
            }
        }
    };

    send(request).await
}

/// Validate and wrap text, or print a rejection and return its exit code.
fn build_speak(cli: &Cli, text: String) -> anyhow::Result<Result<Request, u8>> {
    let text = text.trim().to_string();
    if text.is_empty() {
        eprintln!("error: nothing to say");
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
    }))
}

/// Print the offending span, then a rewrite the caller can resend verbatim.
///
/// The suggestion is the point. An agent that can only be told "no" will
/// guess; one handed replacement text can simply send it.
fn print_rejection(text: &str, rejection: &voicecast_text::Rejection) {
    let (start, end) = rejection.span();
    eprintln!("error: {rejection}");
    eprintln!();

    // Show the offending line with a caret run beneath the span.
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[end..].find('\n').map_or(text.len(), |i| end + i);
    let line = &text[line_start..line_end];
    eprintln!("  {line}");
    let pad = text[line_start..start].chars().count();
    let width = text[start..end].chars().count().max(1);
    eprintln!(
        "  {}{} {}",
        " ".repeat(pad),
        "^".repeat(width),
        rejection.kind()
    );
    eprintln!();

    let suggestion = voicecast_text::strip(text);
    if !suggestion.is_empty() && suggestion != text {
        eprintln!("Write text as it should be spoken:");
        eprintln!("  {suggestion:?}");
        eprintln!();
    }
    eprintln!("Or pass --strip to convert automatically.");
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
    let name = voicecast_core_socket_name().to_ns_name::<GenericNamespaced>()?;

    let mut stream = match Stream::connect(name).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: no voicecast node is running ({e})");
            eprintln!("start one with: voicecastd");
            return Ok(exit::NO_NODE);
        }
    };

    write_frame(&mut stream, &request).await?;
    let response: Response = read_frame(&mut stream).await?;

    Ok(match response {
        Response::Accepted { msg_id } => {
            println!("{msg_id}");
            exit::OK
        }
        Response::Finished { status } => {
            println!("{status:?}");
            exit::OK
        }
        Response::Status {
            engine,
            fallback,
            queued,
        } => {
            println!(
                "engine:  {engine}{}",
                if fallback { "  (fallback)" } else { "" }
            );
            println!("queued:  {queued}");
            exit::OK
        }
        Response::Error { message } => {
            eprintln!("error: {message}");
            exit::USAGE
        }
    })
}

/// Socket name, duplicated rather than depending on `voicecast-core`.
///
/// A handful of bytes of duplication is a fair price for keeping this binary
/// free of the node's entire dependency graph.
fn voicecast_core_socket_name() -> String {
    "voicecast.sock".to_string()
}

// Frame helpers, mirroring `voicecast_core::ipc` for the same reason.
mod frame;
use frame::{read_frame, write_frame};
