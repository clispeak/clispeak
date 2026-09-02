//! Piper: neural speech on the CPU, no account and no network.
//!
//! The engine every desktop uses. espeak-ng exists so a Linux device is never
//! silent; this is what it should actually sound like.
//!
//! Driven as processes rather than through a library: Piper writes raw audio
//! on stdout, which goes straight to the system player. That keeps synthesis
//! streaming — speech starts before the whole chunk is rendered — and makes
//! `stop` a matter of killing two children. Where no player reads raw audio
//! on stdin, which is every stock Mac, a rendered file stands in; see
//! [`Player`].

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{EngineError, SpeechEngine, Tier, Voice};

/// Where a locally installed Piper and its voices live.
///
/// Under the user's data directory rather than a system path: this is
/// per-user state, and needs no privileges to install or replace.
fn install_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    // A user-installed copy wins, so someone can drop in a better voice
    // without rebuilding anything.
    if let Some(dirs) = directories::BaseDirs::new() {
        roots.push(dirs.data_local_dir().join("voicecast"));
    }
    // Carried inside the application bundle. On macOS an app is dragged to
    // /Applications with everything it needs inside it, so for a normal
    // install this is the only copy there is.
    roots.extend(bundled_root());
    // Shipped with the package. Inside a Flatpak this is the only copy, since
    // the sandbox has no access to a system-wide install.
    roots.push(PathBuf::from("/app/share/voicecast"));
    roots.push(PathBuf::from("/usr/share/voicecast"));
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
#[derive(Clone, Copy)]
enum Player {
    /// Reads raw PCM on stdin.
    Streaming(&'static str),
    /// Plays a finished file, named as its last argument.
    File(&'static str),
}

impl Player {
    /// The command this player runs as.
    fn command(self) -> &'static str {
        match self {
            Self::Streaming(c) | Self::File(c) => c,
        }
    }
}

/// Players that can play what Piper produces, best first.
///
/// Streaming entries come first wherever they exist: they are what makes
/// speech start promptly on a long chunk.
const PLAYERS: &[Player] = &[
    // PulseAudio, PipeWire and ALSA, in the order a Linux desktop is likely
    // to have them.
    Player::Streaming("paplay"),
    Player::Streaming("pw-play"),
    Player::Streaming("aplay"),
    // sox, the one streaming player commonly installed on a Mac.
    Player::Streaming("play"),
    // Built into macOS. Last because it cannot stream, and therefore the one
    // chosen on any Mac without sox.
    Player::File("afplay"),
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
    synth: Child,
    player: Child,
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
            .map(|r| r.join("piper/piper"))
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
            // A file player is handed a path, and works out the format from
            // the header itself.
            Player::File(_) => Vec::new(),
        }
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
            .stderr(Stdio::null())
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
    fn finish(&self) -> Result<(), EngineError> {
        let mut guard = self.current.lock().expect("engine lock");
        if let Some(playing) = guard.as_mut() {
            let _ = playing.synth.wait();
            // Waiting on the player, not the synthesiser, is what makes this
            // return when the sound has actually finished.
            playing
                .player
                .wait()
                .map_err(|e| EngineError::Unavailable(format!("playback failed: {e}")))?;
        }
        if let Some(done) = guard.take() {
            done.clean_up();
        }
        Ok(())
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
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| {
                        EngineError::Unavailable(format!("could not start {command}: {e}"))
                    })?;

                *self.current.lock().expect("engine lock") = Some(Playing {
                    synth,
                    player,
                    scratch: None,
                });
            }
            // Nothing on macOS reads raw audio on stdin, so the chunk is
            // rendered in full and then played. Synthesis is not yet
            // registered as playing, so a `stop` arriving during it is a
            // no-op — the wait is short, and the alternative is claiming to
            // have cancelled sound that has not started.
            Player::File(command) => {
                let scratch = scratch_wav();
                let mut synth =
                    self.synthesise(chunk, &voice, length_scale, &Sink::File(scratch.clone()))?;
                synth
                    .wait()
                    .map_err(|e| EngineError::Unavailable(format!("Piper failed: {e}")))?;

                let player = Command::new(command)
                    .arg(&scratch)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| {
                        let _ = std::fs::remove_file(&scratch);
                        EngineError::Unavailable(format!("could not start {command}: {e}"))
                    })?;

                *self.current.lock().expect("engine lock") = Some(Playing {
                    synth,
                    player,
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
        if let Some(mut playing) = self.current.lock().expect("engine lock").take() {
            let _ = playing.synth.kill();
            let _ = playing.player.kill();
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

/// Turn `en_US-lessac-medium` into something worth showing a person.
fn pretty_voice_name(id: &str) -> String {
    let mut parts = id.split('-');
    let locale = parts.next().unwrap_or(id);
    let name = parts.next().unwrap_or("");
    let quality = parts.next().unwrap_or("");
    let mut label = name.to_string();
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

/// Whether a command exists on `PATH`.
fn which(command: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(command).is_file()))
        .unwrap_or(false)
}
