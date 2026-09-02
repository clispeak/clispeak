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
    /// Some targets spoke and some did not.
    pub const PARTIAL: u8 = 3;
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
    /// Device to speak on. A name, a group, `all`, `here`, or a
    /// comma-separated list of those.
    ///
    /// Defaults to `default_target` from the config, then this machine.
    #[arg(short, long, global = true)]
    to: Option<String>,

    /// Urgency. `high` interrupts whatever is speaking.
    ///
    /// Left unset rather than defaulted here so the config can supply it —
    /// clap cannot otherwise tell "not given" from "given as normal".
    #[arg(short, long, value_enum, global = true)]
    priority: Option<Prio>,

    /// Convert markdown to speakable text instead of rejecting it.
    #[arg(long, global = true)]
    strip: bool,

    /// Speak exactly as given, skipping validation entirely.
    #[arg(long, global = true)]
    raw: bool,

    /// Wait for every device to finish, and report what happened on each.
    ///
    /// Off by default so an agent firing notifications is not blocked on
    /// playback; on when it needs to know the message was actually heard.
    #[arg(short, long, global = true)]
    wait: bool,

    /// Print the result as JSON. Implies waiting.
    #[arg(long, global = true)]
    json: bool,

    /// Read the text to speak from a file.
    #[arg(short, long, global = true)]
    file: Option<std::path::PathBuf>,

    /// Ask the receiver for a particular voice.
    ///
    /// A request, not an instruction: a device that does not have it speaks
    /// in its own rather than refusing.
    #[arg(short = 'v', long, global = true)]
    voice: Option<String>,

    /// Seconds to wait with --wait before reporting "still speaking".
    #[arg(long, global = true)]
    timeout: Option<u64>,

    /// Suppress normal output. Errors still go to stderr, and the exit code
    /// still says what happened.
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Resolve the targets and print them without speaking anything.
    #[arg(long, global = true)]
    dry_run: bool,

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

/// What to do with spaces.
#[derive(Subcommand)]
enum SpaceAction {
    /// List the spaces this device belongs to.
    List,
    /// Found a new space from this device, and make it the default.
    New {
        /// What to call it on this device.
        name: String,
    },
    /// Drop one space, keeping the others.
    Leave {
        /// The space's name on this device.
        name: String,
    },
    /// Choose which space bare device names resolve in.
    Default {
        /// The space's name on this device.
        name: String,
    },
    /// Rename a space. Local to this device, like a device label.
    Rename {
        /// Its current name.
        name: String,
        /// What to call it instead.
        to: String,
    },
    /// Replace the default space, locking every other device out.
    Rotate,
}

/// What to do to the group list.
#[derive(Subcommand)]
enum GroupAction {
    /// Define or replace a group.
    Set {
        /// The group's name.
        name: String,
        /// Comma-separated device names.
        devices: String,
    },
    /// Delete a group.
    Rm {
        /// The group's name.
        name: String,
    },
    /// List the groups defined on this machine.
    List,
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
    /// Remove another device from this space.
    Revoke {
        /// The device's name.
        name: String,
    },
    /// Leave the space, keeping this device's identity.
    Leave,
    /// Work with the spaces this device belongs to.
    ///
    /// A space is a set of your own devices, kept fully separate from the
    /// others. Bare device names resolve in the default space; qualify with
    /// `work/laptop` to reach anywhere else.
    Space {
        #[command(subcommand)]
        action: SpaceAction,
    },
    /// Replace this space with a fresh one, locking every other device out.
    ///
    /// For a device that was stolen rather than sold. Revoking is eventually
    /// consistent, so a device that has been offline since the revoke still
    /// honours it until it syncs; a rotated space never contained the
    /// excluded device at all. Every other device has to be re-invited.
    Rotate,
    /// Change this device's name.
    Rename {
        /// The new name.
        name: String,
    },
    /// Abandon the current message and carry on with the queue.
    Skip,
    /// Hold speech without discarding it.
    Pause,
    /// Start speaking again after a pause.
    Resume,
    /// Show what is being spoken and what is waiting.
    Queue,
    /// Silence this device until it is unmuted.
    ///
    /// Local to the device it is run on: one device cannot mute another. A
    /// sender states urgency, the device that makes the noise decides whether
    /// noise is welcome.
    Mute,
    /// Let this device speak again.
    Unmute,
    /// Name a set of devices, so `--to phones` reaches all of them.
    ///
    /// Local to this machine: groups expand to device names before anything
    /// is sent, so they never appear in the protocol and two devices need
    /// not agree on what a group means.
    Group {
        #[command(subcommand)]
        action: GroupAction,
    },
    /// List the groups defined on this machine.
    Groups,
    /// Show, set, or clear this device's quiet hours.
    Quiet {
        /// `22:00-07:00`, or `off`. Omit to show the current window.
        window: Option<String>,
        /// Let `high` messages break through the window.
        ///
        /// Off unless asked for: "urgent" stops meaning anything the first
        /// time an agent marks every message urgent.
        #[arg(long)]
        high: bool,
    },
}

/// Whether `--quiet` was given.
///
/// A global rather than a parameter because every printing site would
/// otherwise have to carry it, including ones several calls deep that have no
/// other reason to know about the command line.
static QUIET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Write a line to stdout, tolerating a closed pipe.
///
/// `println!` panics on EPIPE, so `voicecast ... | head -1` exited 101 —
/// Rust's panic code — instead of the exit code the caller needs to read.
/// An agent piping our output should still get 6 for "bad text".
fn out(s: &str) {
    use std::io::Write;
    if QUIET.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
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

    if cli.quiet {
        QUIET.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let config = config::load();

    // Group commands never reach the node: groups are this machine's own
    // shorthand, so editing them is a local file operation and answering
    // "what groups exist" needs nothing running.
    match &cli.command {
        Some(Command::Groups)
        | Some(Command::Group {
            action: GroupAction::List,
        }) => {
            return Ok(list_groups(&config));
        }
        Some(Command::Group { action }) => return Ok(edit_groups(config, action)),
        _ => {}
    }

    // Answered before any text is read: the point of a dry run is to check
    // where a message *would* go, and demanding the message first would make
    // it useless for exactly that.
    if cli.dry_run {
        return send(
            Request::Resolve {
                to: target(&cli, &config),
            },
            cli.json,
        )
        .await;
    }

    let request = match &cli.command {
        Some(Command::Stop) => Request::Stop,
        Some(Command::Status) => Request::Status,
        Some(Command::Invite) => Request::Invite,
        Some(Command::Devices) => Request::Devices,
        Some(Command::Show) => Request::Show,
        Some(Command::Revoke { name }) => Request::Revoke { name: name.clone() },
        Some(Command::Leave) => Request::Leave,
        Some(Command::Rotate) => Request::Rotate,
        Some(Command::Space { action }) => match action {
            SpaceAction::List => Request::Spaces,
            SpaceAction::New { name } => Request::NewSpace {
                label: name.clone(),
            },
            SpaceAction::Leave { name } => Request::LeaveSpace {
                label: name.clone(),
            },
            SpaceAction::Default { name } => Request::DefaultSpace {
                label: name.clone(),
            },
            SpaceAction::Rename { name, to } => Request::RenameSpace {
                label: name.clone(),
                to: to.clone(),
            },
            SpaceAction::Rotate => Request::Rotate,
        },
        Some(Command::Quit) => Request::Quit,
        Some(Command::Rename { name }) => Request::Rename { name: name.clone() },
        Some(Command::Skip) => Request::Skip,
        Some(Command::Pause) => Request::Pause,
        Some(Command::Resume) => Request::Resume,
        Some(Command::Queue) => Request::Queue,
        Some(Command::Mute) => Request::SetMute { muted: true },
        Some(Command::Unmute) => Request::SetMute { muted: false },
        Some(Command::Quiet { window, high }) => match quiet_request(window.as_deref(), *high) {
            Ok(req) => req,
            Err(code) => return Ok(code),
        },
        Some(Command::Join { ticket }) => Request::Join {
            ticket: ticket.clone(),
        },
        // Handled above, before the node is contacted.
        Some(Command::Group { .. }) | Some(Command::Groups) => unreachable!("local commands"),
        Some(Command::Say { text }) => match build_speak(&cli, &config, text)? {
            Ok(req) => req,
            Err(code) => return Ok(code),
        },
        None => {
            let tokens = if cli.file.is_some() || !cli.text.is_empty() {
                cli.text.clone()
            } else {
                vec![read_stdin()?]
            };
            match build_speak(&cli, &config, &tokens)? {
                Ok(req) => req,
                Err(code) => return Ok(code),
            }
        }
    };

    send(request, cli.json).await
}

/// Which devices to send to, with any group expanded.
///
/// `None` means the node's own default, which is the machine it runs on.
fn target(cli: &Cli, config: &config::Config) -> Option<String> {
    cli.to
        .clone()
        .or_else(|| config.default_target.clone())
        .map(|sel| config::expand(&sel, &config.groups))
}

/// Show the groups defined on this machine.
fn list_groups(config: &config::Config) -> u8 {
    if config.groups.is_empty() {
        err("no groups defined");
        err("");
        err("Define one with:  voicecast group set phones pixel,iphone");
        return exit::OK;
    }
    for (name, devices) in &config.groups {
        out(&format!("{:<16} {}", name, devices.join(", ")));
    }
    exit::OK
}

/// Add or remove a group, then say what the list looks like now.
fn edit_groups(mut config: config::Config, action: &GroupAction) -> u8 {
    match action {
        GroupAction::Set { name, devices } => {
            let members: Vec<String> = devices
                .split(',')
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty())
                .collect();
            if members.is_empty() {
                err("error: a group needs at least one device");
                return exit::USAGE;
            }
            config.groups.insert(name.clone(), members);
        }
        GroupAction::Rm { name } => {
            if config.groups.remove(name).is_none() {
                err(&format!("error: no group named '{name}'"));
                return exit::USAGE;
            }
        }
        GroupAction::List => unreachable!("handled before the node is contacted"),
    }
    match config::write_groups(&config.groups) {
        Ok(_) => list_groups(&config),
        Err(e) => {
            err(&format!("error: could not save the config: {e}"));
            exit::USAGE
        }
    }
}

/// Turn a `22:00-07:00` argument into a request, or explain what went wrong.
///
/// No argument means "show me", which is a plain read rather than a write of
/// the current value — asking a question should never change the answer.
fn quiet_request(window: Option<&str>, high: bool) -> Result<Request, u8> {
    let Some(window) = window else {
        return Ok(Request::Policy);
    };
    if window.eq_ignore_ascii_case("off") || window.eq_ignore_ascii_case("none") {
        return Ok(Request::SetQuiet {
            from: None,
            to: None,
            high_breaks_through: false,
        });
    }
    let Some((from, to)) = window.split_once('-') else {
        err(&format!("error: '{window}' is not a time range"));
        err("");
        err("Write it as a range, or turn it off:");
        err("  voicecast quiet 22:00-07:00");
        err("  voicecast quiet off");
        return Err(exit::USAGE);
    };
    Ok(Request::SetQuiet {
        from: Some(from.trim().to_string()),
        to: Some(to.trim().to_string()),
        high_breaks_through: high,
    })
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
fn build_speak(
    cli: &Cli,
    config: &config::Config,
    tokens: &[String],
) -> anyhow::Result<Result<Request, u8>> {
    if let Some(flag) = flaglike_token(tokens) {
        err(&format!("error: unknown option '{flag}'"));
        err("");
        err("If you meant to speak it, use the -- separator:");
        err(&format!("  voicecast -- {flag} ..."));
        return Ok(Err(exit::USAGE));
    }

    let text = match &cli.file {
        Some(path) => {
            if !tokens.is_empty() {
                err("error: give text or --file, not both");
                return Ok(Err(exit::USAGE));
            }
            match std::fs::read_to_string(path) {
                Ok(text) => text.trim().to_string(),
                Err(e) => {
                    err(&format!("error: could not read {}: {e}", path.display()));
                    return Ok(Err(exit::USAGE));
                }
            }
        }
        None => tokens.join(" ").trim().to_string(),
    };
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

    // Resolution order for all of these: the flag, then the config, then the
    // built-in default. Groups expand here rather than in the node, so what
    // crosses the socket is only ever device names.
    let to = target(cli, config);

    let priority = match cli.priority {
        Some(p) => p.into(),
        None => match config.default_priority.as_deref() {
            Some("low") => Priority::Low,
            Some("high") => Priority::High,
            _ => Priority::Normal,
        },
    };

    Ok(Ok(Request::Speak {
        text,
        priority,
        to,
        voice: cli.voice.clone(),
        timeout_secs: cli.timeout,
        // JSON output exists to be consumed, and a report of "queued" tells a
        // consumer nothing — so asking for it implies waiting.
        wait: cli.wait || cli.json,
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
async fn send(request: Request, json: bool) -> anyhow::Result<u8> {
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
            out(label(&status));
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
            muted,
            quiet,
        } => {
            out(&format!("device:  {device_id}"));
            out(&format!("keys:    {key_store}"));
            out(&format!(
                "engine:  {engine}{}",
                if fallback { "  (fallback)" } else { "" }
            ));
            out(&format!("queued:  {queued}"));
            // Shown only when set. A device that will speak normally should
            // not have to be read carefully to establish that.
            if muted {
                out("muted:   yes");
            }
            if let Some(window) = quiet {
                out(&format!("quiet:   {window}"));
            }
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
        Response::Report { msg_id, targets } => report(&msg_id, &targets, json),
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
                    "{:<16} {}{}{}",
                    d.name,
                    &d.endpoint_id[..16.min(d.endpoint_id.len())],
                    d.space
                        .as_deref()
                        .map(|s| format!("  [{s}]"))
                        .unwrap_or_default(),
                    if d.is_self { "  (this device)" } else { "" }
                ));
            }
            exit::OK
        }
        Response::Spaces { spaces } => {
            for s in spaces {
                out(&format!(
                    "{:<16} {:<3} devices{}{}",
                    s.label,
                    s.devices,
                    if s.is_default { "  (default)" } else { "" },
                    if s.founded_here { "  founded here" } else { "" },
                ));
            }
            exit::OK
        }
        Response::Policy {
            muted,
            quiet_from,
            quiet_to,
            high_breaks_through,
        } => {
            out(&format!("muted:   {}", if muted { "yes" } else { "no" }));
            match (quiet_from, quiet_to) {
                (Some(from), Some(to)) => {
                    out(&format!(
                        "quiet:   {from}-{to}{}",
                        if high_breaks_through {
                            "  (high breaks through)"
                        } else {
                            ""
                        }
                    ));
                }
                _ => out("quiet:   off"),
            }
            exit::OK
        }
        Response::Rotated { devices } => {
            out("this space has been replaced");
            if devices.is_empty() {
                err("No other devices were in it.");
            } else {
                err("");
                err(&format!("Re-invite:  {}", devices.join(", ")));
                err("Run `voicecast invite` once per device.");
            }
            exit::OK
        }
        Response::Targets { devices } => {
            if json {
                out(&serde_json::to_string_pretty(&devices).unwrap_or_else(|_| "[]".into()));
            } else {
                for d in &devices {
                    out(d);
                }
            }
            exit::OK
        }
        Response::Queue {
            speaking,
            pending,
            paused,
        } => {
            if paused {
                out("paused");
            }
            match &speaking {
                Some(id) => out(&format!("speaking  {id}")),
                None if !paused => out("nothing is being spoken"),
                None => {}
            }
            for id in &pending {
                out(&format!("waiting   {id}"));
            }
            exit::OK
        }
        Response::Error { message } => {
            err(&format!("error: {message}"));
            exit::USAGE
        }
    })
}

/// A status as a person would say it.
///
/// `{:?}` lowercased gives "quiethours", which reads as a typo. This is
/// display text; the machine-readable spelling stays in `--json`.
fn label(status: &Status) -> &'static str {
    match status {
        Status::Queued => "queued",
        Status::Speaking => "speaking",
        Status::Spoken => "spoken",
        Status::Muted => "muted",
        Status::QuietHours => "quiet hours",
        Status::NoEngine => "no engine",
        Status::Unreachable => "unreachable",
        Status::Rejected => "rejected",
        Status::Cancelled => "cancelled",
        Status::Dropped => "dropped",
    }
}

/// Show what happened on each device, and pick an exit code to match.
///
/// The exit code is what an agent branches on, so it has to distinguish
/// "everything worked" from "some of it did" from "none of it did" — see the
/// table in `docs/cli.md`.
fn report(msg_id: &str, targets: &[voicecast_proto::TargetResult], json: bool) -> u8 {
    use voicecast_proto::Status;

    let heard = |s: &Status| matches!(s, Status::Spoken | Status::Queued | Status::Speaking);
    let good = targets.iter().filter(|t| heard(&t.status)).count();

    if json {
        let value = serde_json::json!({
            "id": msg_id,
            "targets": targets.iter().map(|t| serde_json::json!({
                "device": t.device,
                "status": t.status,
                "took_ms": t.took_ms,
                "detail": t.detail,
            })).collect::<Vec<_>>(),
        });
        out(&serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into()));
    } else if targets.len() == 1 && targets[0].detail.is_none() && targets[0].took_ms.is_none() {
        // The common case: one target, fire and forget. A table would be
        // noise, so keep the bare id the shell can capture.
        out(msg_id);
    } else {
        for t in targets {
            let took = t.took_ms.map(|ms| format!("{:.1}s", ms as f64 / 1000.0));
            out(&format!(
                "  {:<16} {:<12} {}{}",
                t.device,
                label(&t.status),
                took.unwrap_or_default(),
                t.detail
                    .as_deref()
                    .map(|d| format!("  ({d})"))
                    .unwrap_or_default(),
            ));
        }
    }

    match (good, targets.len()) {
        (0, _) => exit::ALL_FAILED,
        (g, n) if g < n => exit::PARTIAL,
        _ => exit::OK,
    }
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

mod config;

// Frame helpers, mirroring `voicecast_core::ipc` for the same reason.
mod frame;
use frame::{read_frame, write_frame};
