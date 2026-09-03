//! Piper: neural speech on the CPU, no account and no network.
//!
//! The engine every desktop uses. espeak-ng exists so a Linux device is never
//! silent; this is what it should actually sound like.
//!
//! Driven as processes rather than through a library: Piper writes raw audio
//! on stdout, which goes straight to the system player. That keeps synthesis
//! streaming — speech starts before the whole chunk is rendered — and makes
//! `stop` a matter of killing two children. Where no player reads raw audio
//! on stdin, which is every stock Mac and every stock Windows machine, a
//! rendered file stands in; see [`Player`].
//!
//! Windows costs one more thing than the others: Piper links the Microsoft
//! Visual C++ runtime, which Windows does not ship. A machine without it has
//! Piper installed, found, and dying before `main` — so discovery checks that
//! the binary starts rather than only that it exists. See [`starts`].

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::child::Running;
use crate::{EngineError, SpeechEngine, Tier, Voice};

/// The Piper executable's file name.
///
/// Windows will not run a file that has no executable extension, and the
/// failure is a quiet one: every root is searched, nothing matches, and Piper
/// is reported "not installed" while sitting exactly where it was put.
const PIPER_BINARY: &str = if cfg!(windows) { "piper.exe" } else { "piper" };

/// Where a locally installed Piper and its voices live.
///
/// Under the user's data directory rather than a system path: this is
/// per-user state, and needs no privileges to install or replace.
fn install_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    // A user-installed copy wins, so someone can drop in a better voice
    // without rebuilding anything. On Windows this is the only root there is:
    // `%LOCALAPPDATA%\voicecast`, which is also where the installer puts
    // Piper, and which needs no administrator to write.
    if let Some(dirs) = directories::BaseDirs::new() {
        roots.push(dirs.data_local_dir().join("voicecast"));
    }
    // Carried inside the application bundle. On macOS an app is dragged to
    // /Applications with everything it needs inside it, so for a normal
    // install this is the only copy there is.
    roots.extend(bundled_root());
    // Shipped with the package. Inside a Flatpak this is the only copy, since
    // the sandbox has no access to a system-wide install.
    //
    // Listed only where they could exist. These paths reach the user in the
    // "not installed in any of" error, and naming a Unix directory on Windows
    // sends whoever reads it hunting for somewhere that cannot be there.
    #[cfg(unix)]
    {
        roots.push(PathBuf::from("/app/share/voicecast"));
        roots.push(PathBuf::from("/usr/share/voicecast"));
    }
    roots
}

/// The resources directory inside a macOS `.app`, when running from one.
///
/// The executable sits at `Contents/MacOS/<name>`, alongside
/// `Contents/Resources` — which is where the bundle carries Piper and its
/// voices. Returns `None` for a bare `cargo run`, where there is no bundle
/// and the user data directory is the only root that matters.
#[cfg(target_os = "macos")]
fn bundled_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let contents = exe.parent()?.parent()?;
    (contents.file_name()? == "Contents").then(|| contents.join("Resources/speech"))
}

#[cfg(not(target_os = "macos"))]
fn bundled_root() -> Option<PathBuf> {
    None
}

/// How rendered audio reaches the speakers.
///
/// Linux ships players that read raw samples on stdin, which is what lets
/// synthesis and playback overlap so speech starts before a long chunk has
/// finished rendering. macOS ships nothing of the sort — `afplay` opens a
/// file and nothing else — so there the chunk is rendered to a temporary WAV
/// and played afterwards. Slower to start, and it works on a Mac with
/// nothing installed, which is what a normal app install has to assume.
///
/// Windows ships neither: no player that reads raw audio, and none that takes
/// a bare path either. PowerShell can play a WAV and is on every Windows
/// install, so it stands in — but the path has to reach it inside a script
/// rather than as an argument, which is what [`Player::FileOnStdin`] exists
/// for.
#[derive(Clone, Copy)]
enum Player {
    /// Reads raw PCM on stdin.
    Streaming(&'static str),
    /// Plays a finished file, named as its last argument.
    File(&'static str),
    /// Plays a finished file whose path it reads on stdin.
    ///
    /// The path travels the same way chunk text already does, and for the
    /// same reason: stdin has no quoting to get wrong. A rendered path can
    /// contain a space, and the alternative here is threading it through a
    /// PowerShell command line, which is two layers of quoting deep and
    /// fails by playing nothing rather than by complaining.
    FileOnStdin(&'static str),
}

impl Player {
    /// The command this player runs as.
    fn command(self) -> &'static str {
        match self {
            Self::Streaming(c) | Self::File(c) | Self::FileOnStdin(c) => c,
        }
    }
}

/// Players that can play what Piper produces, best first.
///
/// Streaming entries come first wherever they exist: they are what makes
/// speech start promptly on a long chunk. The list is shared across
/// platforms and filtered by what is actually installed, so a Windows
/// machine with sox on it gets streaming playback like any other.
const PLAYERS: &[Player] = &[
    // PulseAudio, PipeWire and ALSA, in the order a Linux desktop is likely
    // to have them.
    Player::Streaming("paplay"),
    Player::Streaming("pw-play"),
    Player::Streaming("aplay"),
    // sox, the one streaming player commonly installed on a Mac — and the
    // only way a Windows machine gets streaming playback at all.
    Player::Streaming("play"),
    // Built into macOS. Last because it cannot stream, and therefore the one
    // chosen on any Mac without sox.
    Player::File("afplay"),
    // Built into Windows. Last for the same reason, and the one chosen on
    // any Windows machine without sox — which is nearly all of them.
    Player::FileOnStdin("powershell"),
];

/// A unique path for one utterance's rendered audio.
///
/// Process id and a counter rather than a temp-file crate: this engine
/// depends on three crates in total, and uniqueness is all that is wanted.
fn scratch_wav() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("voicecast-{}-{n}.wav", std::process::id()))
}

/// The processes currently producing sound, so `stop` can end them.
struct Playing {
    synth: Arc<Running>,
    player: Arc<Running>,
    /// A rendered file to delete once it has been played. Only a file-based
    /// player leaves one behind.
    scratch: Option<PathBuf>,
}

/// Speaks through Piper, playing the audio with the system's player.
pub struct PiperEngine {
    binary: PathBuf,
    /// Every voice model found on disk, so the choice can change at runtime.
    available: Vec<PathBuf>,
    /// The selected voice and the sample rate it declares.
    ///
    /// Held together because they must change as a pair: playing one voice's
    /// audio at another's rate is the kind of bug that sounds like a haunting.
    selected: Mutex<(PathBuf, u32)>,
    /// Length scale, inverted from a rate multiplier. Piper stretches audio,
    /// so a *higher* scale is slower.
    rate: Mutex<f32>,
    player: Player,
    current: Mutex<Option<Playing>>,
}

impl PiperEngine {
    /// Find a local Piper installation, or explain what is missing.
    ///
    /// Errors are written for someone who could install it, since that is the
    /// only way to act on them.
    pub fn discover() -> Result<Self, EngineError> {
        // The binary and the voices may live in different places: a bundled
        // Piper with a voice the user added themselves is a normal setup.
        let roots = install_roots();
        let binary = roots
            .iter()
            .map(|r| r.join("piper").join(PIPER_BINARY))
            .find(|p| p.exists())
            .ok_or_else(|| {
                EngineError::Unavailable(format!(
                    "Piper is not installed in any of: {}",
                    roots
                        .iter()
                        .map(|r| r.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;
        // Before anything else: a file that exists is not the same as a
        // binary that runs, and the gap between them is where the worst
        // failure on this path lives.
        starts(&binary)?;

        let available: Vec<PathBuf> = roots
            .iter()
            .flat_map(|r| voices_in(&r.join("voices")))
            .collect();
        let voice = available
            .first()
            .cloned()
            .ok_or_else(|| EngineError::Unavailable("Piper has no voice model installed".into()))?;
        let sample_rate = sample_rate_of(&voice).unwrap_or(22_050);

        let player = PLAYERS
            .iter()
            .copied()
            .find(|p| which(p.command()))
            .ok_or_else(|| {
                EngineError::Unavailable(format!(
                    "no audio player found (looked for {})",
                    PLAYERS
                        .iter()
                        .map(|p| p.command())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;

        Ok(Self {
            binary,
            available,
            selected: Mutex::new((voice, sample_rate)),
            rate: Mutex::new(1.0),
            player,
            current: Mutex::new(None),
        })
    }

    /// The selected voice model and its sample rate.
    fn voice(&self) -> (PathBuf, u32) {
        self.selected.lock().expect("voice lock").clone()
    }

    /// The selected voice's name, without its extension.
    fn voice_name(&self) -> String {
        name_of(&self.voice().0)
    }

    /// Arguments telling the chosen player how to read raw audio.
    fn player_args(&self, sample_rate: u32) -> Vec<String> {
        let rate = sample_rate.to_string();
        match self.player {
            Player::Streaming("aplay") => vec![
                "-q".into(),
                "-r".into(),
                rate,
                "-f".into(),
                "S16_LE".into(),
                "-c".into(),
                "1".into(),
                "-t".into(),
                "raw".into(),
                "-".into(),
            ],
            // sox spells the same description out one flag at a time.
            Player::Streaming("play") => vec![
                "-q".into(),
                "-t".into(),
                "raw".into(),
                "-r".into(),
                rate,
                "-e".into(),
                "signed".into(),
                "-b".into(),
                "16".into(),
                "-c".into(),
                "1".into(),
                "-".into(),
            ],
            // paplay and pw-play share the same flags.
            Player::Streaming(_) => vec![
                "--raw".into(),
                format!("--rate={rate}"),
                "--format=s16le".into(),
                "--channels=1".into(),
            ],
            // PowerShell is handed a script that reads the path on stdin and
            // blocks until the sound has finished. Waiting on this child
            // therefore means waiting for the audio, which is the contract
            // every other player here honours and the one `finish` relies on.
            //
            // `-NoProfile` is not only for start-up time: a profile is
            // arbitrary code belonging to whoever is logged in, and running
            // it to play a sound would be a surprising thing to do.
            Player::FileOnStdin(_) => vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                "$p = [Console]::In.ReadLine(); (New-Object Media.SoundPlayer $p).PlaySync()"
                    .into(),
            ],
            // A file player is handed a path, and works out the format from
            // the header itself.
            Player::File(_) => Vec::new(),
        }
    }

    /// Start a file-based player on a rendered chunk.
    ///
    /// The path reaches the player as its last argument or on its stdin,
    /// depending on which kind it is. Nothing else about the two differs.
    fn play_file(
        &self,
        command: &str,
        sample_rate: u32,
        scratch: &Path,
    ) -> Result<Child, EngineError> {
        let on_stdin = matches!(self.player, Player::FileOnStdin(_));
        let mut player = Command::new(command);
        player
            .args(self.player_args(sample_rate))
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if on_stdin {
            player.stdin(Stdio::piped());
        } else {
            player.arg(scratch).stdin(Stdio::null());
        }
        let mut player = player
            .spawn()
            .map_err(|e| EngineError::Unavailable(format!("could not start {command}: {e}")))?;

        if on_stdin {
            let mut stdin = player
                .stdin
                .take()
                .ok_or_else(|| EngineError::Unavailable(format!("no stdin on {command}")))?;
            // The newline is what ends the player's read. Without it the
            // script waits for input that never arrives, and the failure is
            // silence rather than an error.
            writeln!(stdin, "{}", scratch.display()).map_err(|e| {
                EngineError::Unavailable(format!("handing the audio to {command}: {e}"))
            })?;
            drop(stdin);
        }
        Ok(player)
    }

    /// Start Piper on one chunk, with its text on stdin.
    ///
    /// `sink` decides whether the audio comes back on stdout or goes to a
    /// file, which is the only difference between the two playback paths.
    fn synthesise(
        &self,
        chunk: &str,
        voice: &Path,
        length_scale: f32,
        sink: &Sink,
    ) -> Result<Child, EngineError> {
        let mut command = Command::new(&self.binary);
        command
            .arg("--model")
            .arg(voice)
            .arg("--length_scale")
            .arg(format!("{length_scale:.3}"));
        match sink {
            Sink::Stdout => {
                command.arg("--output_raw").stdout(Stdio::piped());
            }
            Sink::File(path) => {
                command.arg("--output_file").arg(path).stdout(Stdio::null());
            }
        }
        let mut synth = command
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| EngineError::Unavailable(format!("could not start Piper: {e}")))?;

        // Text goes on stdin, never argv: a chunk can contain anything an
        // agent emitted, and stdin has no quoting to get wrong.
        let mut stdin = synth
            .stdin
            .take()
            .ok_or_else(|| EngineError::Unavailable("no stdin on Piper".into()))?;
        stdin
            .write_all(chunk.as_bytes())
            .map_err(|e| EngineError::Unavailable(format!("writing to Piper: {e}")))?;
        // Dropping stdin here is what tells Piper the input is complete.
        drop(stdin);

        Ok(synth)
    }

    /// Wait for the utterance now playing, then clear it.
    ///
    /// The handles are lifted out from under the lock and waited on outside
    /// it. Holding the lock across the wait is what made `stop` park until
    /// playback had finished and then kill nothing — issue #58. The entry
    /// stays in `current` while we wait, because that is how `stop` finds it.
    fn finish(&self) -> Result<(), EngineError> {
        let waiting = self
            .current
            .lock()
            .expect("engine lock")
            .as_ref()
            .map(|playing| (Arc::clone(&playing.synth), Arc::clone(&playing.player)));

        let outcome = match waiting {
            Some((synth, player)) => {
                // Both are checked, and the player last, because waiting on
                // the player is what makes this return when the sound has
                // actually stopped. Piper exiting non-zero while the player
                // reads a truncated stream and exits 0 is a real shape: the
                // synthesiser's failure is the one worth reporting, so it is
                // asked about first.
                let synthesised = synth.wait();
                let played = player.wait();
                synthesised.and(played)
            }
            None => Ok(()),
        };

        if let Some(done) = self.current.lock().expect("engine lock").take() {
            done.clean_up();
        }
        outcome
    }
}

/// Where Piper is told to put the audio it renders.
enum Sink {
    /// Straight down a pipe to a streaming player.
    Stdout,
    /// Into a file, for a player that can only open one.
    File(PathBuf),
}

impl Playing {
    /// Remove anything this utterance left on disk.
    fn clean_up(self) {
        if let Some(scratch) = self.scratch {
            let _ = std::fs::remove_file(scratch);
        }
    }
}

impl SpeechEngine for PiperEngine {
    fn speak(&self, chunk: &str) -> Result<(), EngineError> {
        let (voice, sample_rate) = self.voice();
        // Piper stretches audio by a length scale, so a faster rate is a
        // smaller scale — the reciprocal, not the value.
        let length_scale = 1.0 / self.rate().max(0.1);

        match self.player {
            // Synthesis and playback run at once, joined by a pipe.
            Player::Streaming(command) => {
                let mut synth = self.synthesise(chunk, &voice, length_scale, &Sink::Stdout)?;
                let audio = synth.stdout.take().ok_or_else(|| {
                    EngineError::Unavailable("Piper produced no audio stream".into())
                })?;
                let player = Command::new(command)
                    .args(self.player_args(sample_rate))
                    .stdin(Stdio::from(audio))
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| {
                        EngineError::Unavailable(format!("could not start {command}: {e}"))
                    })?;

                *self.current.lock().expect("engine lock") = Some(Playing {
                    synth: Running::new("piper", synth),
                    player: Running::new(command, player),
                    scratch: None,
                });
            }
            // Nothing on macOS or Windows reads raw audio on stdin, so the
            // chunk is rendered in full and then played. Synthesis is not yet
            // registered as playing, so a `stop` arriving during it is a
            // no-op — the wait is short, and the alternative is claiming to
            // have cancelled sound that has not started.
            Player::File(command) | Player::FileOnStdin(command) => {
                let scratch = scratch_wav();
                let synth = Running::new(
                    "piper",
                    self.synthesise(chunk, &voice, length_scale, &Sink::File(scratch.clone()))?,
                );
                // Checked, not merely awaited. A Piper that exits non-zero
                // here leaves an absent or truncated file, and playing it was
                // reported as having spoken — issue #59. The status is
                // remembered, so `finish` asking again gets this answer
                // rather than a second `try_wait` on a reaped process.
                synth.wait().inspect_err(|_| {
                    let _ = std::fs::remove_file(&scratch);
                })?;

                let player = self
                    .play_file(command, sample_rate, &scratch)
                    .inspect_err(|_| {
                        let _ = std::fs::remove_file(&scratch);
                    })?;

                *self.current.lock().expect("engine lock") = Some(Playing {
                    synth,
                    player: Running::new(command, player),
                    scratch: Some(scratch),
                });
            }
        }

        self.finish()
    }

    fn voices(&self) -> Vec<Voice> {
        self.available
            .iter()
            .map(|p| {
                let id = name_of(p);
                Voice {
                    name: pretty_voice_name(&id),
                    id,
                }
            })
            .collect()
    }

    fn current_voice(&self) -> Option<String> {
        Some(self.voice_name())
    }

    fn set_voice(&self, id: &str) -> Result<(), EngineError> {
        let chosen = self
            .available
            .iter()
            .find(|p| name_of(p) == id)
            .ok_or_else(|| {
                EngineError::Unavailable(format!("no voice called '{id}' is installed"))
            })?;
        let rate = sample_rate_of(chosen).unwrap_or(22_050);
        *self.selected.lock().expect("voice lock") = (chosen.clone(), rate);
        Ok(())
    }

    fn rate(&self) -> f32 {
        *self.rate.lock().expect("rate lock")
    }

    fn set_rate(&self, rate: f32) -> Result<(), EngineError> {
        // Bounded because the ends are unusable: too slow to sit through, or
        // too fast to follow.
        if !(0.5..=2.0).contains(&rate) {
            return Err(EngineError::Unavailable(
                "speaking rate must be between 0.5 and 2.0".into(),
            ));
        }
        *self.rate.lock().expect("rate lock") = rate;
        Ok(())
    }

    fn stop(&self) {
        // Taken out under the lock and killed outside it, so this returns in
        // the time two kills take rather than the length of the utterance.
        let playing = self.current.lock().expect("engine lock").take();
        if let Some(playing) = playing {
            // The player first: it is the one making noise, and killing the
            // synthesiser first leaves the player draining a pipe that has
            // already been filled.
            playing.player.kill();
            playing.synth.kill();
            playing.clean_up();
        }
    }

    fn tier(&self) -> Tier {
        // The intended engine on every desktop, not a stand-in.
        Tier::Full
    }
}

/// Every voice model in `dir`, sorted so ordering is stable between runs
/// rather than whatever the filesystem happens to return.
fn voices_in(dir: &Path) -> Vec<PathBuf> {
    let mut voices: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "onnx"))
        .collect();
    voices.sort();
    voices
}

/// A voice model's name, without its extension.
fn name_of(voice: &Path) -> String {
    voice
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "piper".into())
}

/// Names that capitalising the first letter gets wrong.
///
/// Only the voice we ship, deliberately. Upstream has hundreds and most are
/// ordinary given names that the general rule handles — but the default is
/// the one every user reads in `status` and in the voice picker before they
/// have chosen anything, and "Ljspeech" reads as a typo rather than a name.
///
/// A list of one is a poor thing to grow. If it reaches half a dozen, the
/// answer is a display name in the sidecar config rather than a longer table
/// here.
const DISPLAY_NAMES: &[(&str, &str)] = &[("ljspeech", "LJSpeech")];

/// Turn `en_US-ljspeech-medium` into something worth showing a person.
fn pretty_voice_name(id: &str) -> String {
    let mut parts = id.split('-');
    let locale = parts.next().unwrap_or(id);
    let name = parts.next().unwrap_or("");
    let quality = parts.next().unwrap_or("");
    let mut label = match DISPLAY_NAMES.iter().find(|(k, _)| *k == name) {
        Some((_, pretty)) => (*pretty).to_string(),
        None => name.to_string(),
    };
    if let Some(first) = label.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    match (label.is_empty(), quality.is_empty()) {
        (true, _) => id.to_string(),
        (false, true) => format!("{label} ({locale})"),
        (false, false) => format!("{label} — {quality} ({locale})"),
    }
}

/// The sample rate a voice declares, from its sidecar config.
fn sample_rate_of(voice: &Path) -> Option<u32> {
    let config = voice.with_extension("onnx.json");
    let text = std::fs::read_to_string(config).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("audio")?
        .get("sample_rate")?
        .as_u64()
        .map(|r| r as u32)
}

/// The exit code a process gets when the system could not load it at all.
///
/// On Windows this is `STATUS_DLL_NOT_FOUND`: the process is created, the
/// loader cannot resolve a library the binary links against, and it dies
/// before `main` having printed nothing. On Unix the dynamic loader does
/// print its complaint, and exits 127.
#[cfg(windows)]
const LOADER_FAILED: i32 = 0xC000_0135u32 as i32;
#[cfg(not(windows))]
const LOADER_FAILED: i32 = 127;

/// What a person can actually do about a Piper that will not load.
#[cfg(windows)]
const LOADER_ADVICE: &str = "is installed but will not start, because a library it needs is \
     missing. Piper links the Microsoft Visual C++ runtime, which Windows does not ship and \
     which voicecast therefore installs beside it. Reinstalling voicecast restores that copy \
     — antivirus quarantine is the usual reason it goes missing.";
#[cfg(not(windows))]
const LOADER_ADVICE: &str = "is installed but will not start, because a library it needs is \
     missing. Reinstalling voicecast, or running `cargo xtask piper`, restores it.";

/// Check that Piper actually runs, rather than only that the file is there.
///
/// Discovery used to stop at "the binary exists", which is a different
/// question from "the binary starts", and the difference is not academic: a
/// Piper that cannot load its libraries is installed correctly, found
/// correctly, and dies before `main`. Every message would be accepted and
/// never spoken, with a bare exit code as the only clue — precisely the
/// swallowing this project refuses everywhere else.
///
/// Deliberately narrow. Only the signature that means *the process never
/// started* counts as failure, never a non-zero exit on its own. This runs on
/// every platform, and a probe that guessed at what `--help` ought to return
/// would break working installs in order to catch broken ones.
fn starts(binary: &Path) -> Result<(), EngineError> {
    let outcome = Command::new(binary)
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status();

    match outcome {
        Err(e) => Err(EngineError::Unavailable(format!(
            "{} could not be run: {e}",
            binary.display()
        ))),
        Ok(status) if status.code() == Some(LOADER_FAILED) => Err(EngineError::Unavailable(
            format!("{} {LOADER_ADVICE}", binary.display()),
        )),
        Ok(_) => Ok(()),
    }
}

/// Whether a command exists on `PATH`.
///
/// Windows names executables with an extension from `PATHEXT`, so looking for
/// the bare name finds nothing there — and the failure would be a player that
/// is plainly installed being reported as missing, with the engine refusing
/// to start on a machine that could have spoken perfectly well.
fn which(command: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let bare = dir.join(command);
        bare.is_file()
            || executable_extensions()
                .iter()
                .any(|ext| dir.join(format!("{command}{ext}")).is_file())
    })
}

/// Extensions that make a file executable, on platforms that use them.
///
/// Empty everywhere but Windows, where the list comes from `PATHEXT` so a
/// machine that has been configured differently is still searched correctly.
/// The fallback is the stock value, used when the variable is missing rather
/// than assuming the search can be skipped.
fn executable_extensions() -> Vec<String> {
    if !cfg!(windows) {
        return Vec::new();
    }
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter(|e| !e.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::pretty_voice_name;

    #[test]
    fn a_voice_is_named_as_a_person_would_read_it() {
        // The default, which every user reads in `status` and in the voice
        // picker before they have chosen anything. Capitalising the first
        // letter — right for every ordinary given name upstream ships — turns
        // this one into "Ljspeech", which reads as a typo.
        assert_eq!(
            pretty_voice_name("en_US-ljspeech-medium"),
            "LJSpeech — medium (en_US)"
        );

        // The general rule still handles the ordinary case, which is why it
        // is still the rule.
        assert_eq!(
            pretty_voice_name("en_US-amy-medium"),
            "Amy — medium (en_US)"
        );

        // Something not shaped like a voice id is left alone rather than
        // mangled into a prettier lie.
        assert_eq!(pretty_voice_name("custom"), "custom");
    }
}
