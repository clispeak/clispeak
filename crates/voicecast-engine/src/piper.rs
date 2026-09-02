//! Piper: neural speech on the CPU, no account and no network.
//!
//! The engine `docs/architecture.md` intends for Linux. espeak-ng exists so a
//! device is never silent; this is what it should actually sound like.
//!
//! Driven as two piped processes rather than through a library: Piper writes
//! raw audio on stdout, which goes straight to the system player. That keeps
//! synthesis streaming — speech starts before the whole chunk is rendered —
//! and makes `stop` a matter of killing two children.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use crate::{EngineError, SpeechEngine, Tier, Voice};

/// Where a locally installed Piper and its voices live.
///
/// Under the user's data directory rather than a system path: this is
/// per-user state, and needs no privileges to install or replace.
fn install_root() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.data_local_dir().join("voicecast"))
}

/// The processes currently producing sound, so `stop` can end them.
struct Playing {
    synth: Child,
    player: Child,
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
    player: &'static str,
    current: Mutex<Option<Playing>>,
}

impl PiperEngine {
    /// Find a local Piper installation, or explain what is missing.
    ///
    /// Errors are written for someone who could install it, since that is the
    /// only way to act on them.
    pub fn discover() -> Result<Self, EngineError> {
        let root = install_root()
            .ok_or_else(|| EngineError::Unavailable("no data directory on this system".into()))?;

        let binary = root.join("piper/piper");
        if !binary.exists() {
            return Err(EngineError::Unavailable(format!(
                "Piper is not installed at {}",
                binary.display()
            )));
        }

        let available = voices_in(&root.join("voices"));
        let voice = available.first().cloned().ok_or_else(|| {
            EngineError::Unavailable(format!(
                "Piper has no voice model in {}",
                root.join("voices").display()
            ))
        })?;
        let sample_rate = sample_rate_of(&voice).unwrap_or(22_050);

        let player = ["paplay", "pw-play", "aplay"]
            .into_iter()
            .find(|p| which(p))
            .ok_or_else(|| {
                EngineError::Unavailable("no audio player found (paplay, pw-play or aplay)".into())
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
            "aplay" => vec![
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
            // paplay and pw-play share the same flags.
            _ => vec![
                "--raw".into(),
                format!("--rate={rate}"),
                "--format=s16le".into(),
                "--channels=1".into(),
            ],
        }
    }
}

impl SpeechEngine for PiperEngine {
    fn speak(&self, chunk: &str) -> Result<(), EngineError> {
        let (voice, sample_rate) = self.voice();
        // Piper stretches audio by a length scale, so a faster rate is a
        // smaller scale — the reciprocal, not the value.
        let length_scale = 1.0 / self.rate().max(0.1);

        let mut synth = Command::new(&self.binary)
            .arg("--model")
            .arg(&voice)
            .arg("--length_scale")
            .arg(format!("{length_scale:.3}"))
            .arg("--output_raw")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| EngineError::Unavailable(format!("could not start Piper: {e}")))?;

        let audio = synth
            .stdout
            .take()
            .ok_or_else(|| EngineError::Unavailable("Piper produced no audio stream".into()))?;

        let player = Command::new(self.player)
            .args(self.player_args(sample_rate))
            .stdin(Stdio::from(audio))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                EngineError::Unavailable(format!("could not start {}: {e}", self.player))
            })?;

        {
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
        }

        *self.current.lock().expect("engine lock") = Some(Playing { synth, player });

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
        *guard = None;
        Ok(())
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
        }
    }

    fn tier(&self) -> Tier {
        // The intended engine on Linux, not a stand-in.
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
