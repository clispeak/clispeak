//! Speech on Apple platforms, through `AVSpeechSynthesizer`.
//!
//! **iOS, because it is the only speech there is.** The other unix engines
//! spawn a process and iOS does not permit that at all, which is why it was
//! excluded from them (#126).
//!
//! **macOS, because Piper there is three problems in a trenchcoat** (#132).
//! Upstream's macOS build ships without an rpath and without the dylibs it
//! links against, so installing it runs `otool`, `install_name_tool` and
//! `codesign` — Xcode Command Line Tools, not base macOS. "Download the
//! engine on first run" therefore meant "install a gigabyte of developer
//! tooling first", which is not a first-run download. On top of that
//! `rhasspy/piper` was archived in October 2025 and its maintained successor
//! is GPL-3.0 for the whole project, because it embeds espeak-ng rather than
//! spawning it.
//!
//! One engine ends all three on macOS, and it is the engine iOS already
//! needed. What it costs is that a message no longer sounds identical on
//! every desktop — which `docs/architecture.md` stated as a goal, and which
//! was really a consequence of Piper being the only thing available
//! everywhere.
//!
//! # Why nothing here claims to be `Send`
//!
//! `SpeechEngine` requires `Send + Sync`, and `Retained<AVSpeechSynthesizer>`
//! is neither. The obvious move is an `unsafe impl` — and that would be an
//! assertion about AVFoundation's threading contract read out of Apple's
//! documentation, which is the kind of claim this project has been wrong
//! about repeatedly.
//!
//! [`MainThreadBound`] needs no such claim. The synthesiser is created on the
//! main thread, only ever touched there through [`run_on_main`], and dropped
//! there. Nothing non-`Send` crosses a thread boundary, so there is nothing
//! to assert. The `unsafe impl` that makes it work is `dispatch2`'s, made
//! once, with its reasoning written beside it.
//!
//! The main thread specifically, rather than a thread of our own: a
//! synthesiser on a bare background thread has no run loop, and the failure
//! that produces is `isSpeaking()` answering perfectly while no audio ever
//! starts — an engine that looks like it is working.

// Every method in the AVFoundation bindings is an `unsafe fn`, because it is
// FFI. That is a different thing from the `unsafe impl` avoided above: these
// are calls into Objective-C, not assertions about what may cross threads.
#![allow(unsafe_code)]

use std::sync::Mutex;

use dispatch2::{MainThreadBound, run_on_main};
use objc2_avf_audio::{
    AVAudioSession, AVAudioSessionCategoryPlayback, AVSpeechBoundary, AVSpeechSynthesisVoice,
    AVSpeechSynthesizer, AVSpeechUtterance, AVSpeechUtteranceDefaultSpeechRate,
    AVSpeechUtteranceMaximumSpeechRate, AVSpeechUtteranceMinimumSpeechRate,
};
use objc2_foundation::NSString;

use crate::{EngineError, SpeechEngine, Tier, Voice};

/// How often a speaking chunk is checked for completion.
///
/// `speak` blocks, because the queue calls it that way and every other engine
/// waits on its process. `AVSpeechSynthesizer` is asynchronous, so the wait
/// is a poll — the same shape as `child.rs`, for the same reason: a `stop`
/// has to land mid-sentence, and a wait that holds something is issue #58.
const POLL: std::time::Duration = std::time::Duration::from_millis(20);

/// How long to wait for a queued utterance to actually begin.
///
/// `speakUtterance` is asynchronous, so there is a window in which nothing is
/// speaking yet and nothing is wrong. Long enough to cover the audio session
/// and synthesiser starting; short enough that a chunk which finishes inside
/// it costs a caller half a second rather than a hang.
const START_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// Speech through the platform synthesiser.
pub struct AppleEngine {
    synth: MainThreadBound<objc2::rc::Retained<AVSpeechSynthesizer>>,
    /// Chosen voice identifier, or `None` for the system default.
    voice: Mutex<Option<String>>,
    /// Rate as a multiplier of normal, matching the trait's units rather
    /// than AVFoundation's.
    rate: Mutex<f32>,
}

impl AppleEngine {
    /// Build one, configuring the audio session so it can actually be heard.
    pub fn new() -> Result<Self, EngineError> {
        // Playback, or the hardware silent switch mutes it. The default
        // category respects that switch, which is correct for most apps and
        // fatal for this one: the whole point is being heard when nobody is
        // looking at the screen, and a phone in a pocket has the switch on.
        //
        // A failure here is reported rather than ignored. An engine that
        // cannot be heard is not an engine that works, and "it spoke and you
        // heard nothing" is the exact silence this project forbids.
        run_on_main(|_mtm| unsafe {
            let session = AVAudioSession::sharedInstance();
            let playback = AVAudioSessionCategoryPlayback.ok_or_else(|| {
                EngineError::Unavailable("this iOS has no playback audio category".into())
            })?;
            session.setCategory_error(playback).map_err(|e| {
                EngineError::Unavailable(format!(
                    "could not set the audio session to playback, so speech \
                         would be muted by the silent switch: {e}"
                ))
            })?;
            session.setActive_error(true).map_err(|e| {
                EngineError::Unavailable(format!("could not activate the audio session: {e}"))
            })
        })?;

        let synth = run_on_main(|mtm| {
            let synth = unsafe { AVSpeechSynthesizer::new() };
            MainThreadBound::new(synth, mtm)
        });

        Ok(Self {
            synth,
            voice: Mutex::new(None),
            rate: Mutex::new(1.0),
        })
    }

    /// Turn the trait's multiplier into AVFoundation's rate.
    ///
    /// AVFoundation's scale is neither linear nor centred on 1.0 — it runs
    /// from a minimum to a maximum with a default somewhere between, and the
    /// numbers are not documented as fixed. So this interpolates from the
    /// constants rather than hard-coding what they are today.
    fn platform_rate(multiplier: f32) -> f32 {
        let (min, default, max) = unsafe {
            (
                AVSpeechUtteranceMinimumSpeechRate,
                AVSpeechUtteranceDefaultSpeechRate,
                AVSpeechUtteranceMaximumSpeechRate,
            )
        };
        let m = multiplier.clamp(0.5, 2.0);
        if m >= 1.0 {
            default + (max - default) * (m - 1.0)
        } else {
            min + (default - min) * (m - 0.5) / 0.5
        }
    }
}

impl SpeechEngine for AppleEngine {
    fn speak(&self, chunk: &str) -> Result<(), EngineError> {
        let text = chunk.to_string();
        let voice = self.voice.lock().expect("voice lock").clone();
        let rate = Self::platform_rate(*self.rate.lock().expect("rate lock"));

        self.synth.get_on_main(move |synth| unsafe {
            let utterance =
                AVSpeechUtterance::speechUtteranceWithString(&NSString::from_str(&text));
            utterance.setRate(rate);
            if let Some(id) = voice {
                let chosen = AVSpeechSynthesisVoice::voiceWithIdentifier(&NSString::from_str(&id));
                // A voice that has gone missing is not a reason to say
                // nothing: the system default still speaks the words.
                if chosen.is_some() {
                    utterance.setVoice(chosen.as_deref());
                }
            }
            synth.speakUtterance(&utterance);
        });

        // Blocking, because the queue expects it. Nothing is held across the
        // sleep: `get_on_main` returns a `bool` and takes no lock with it.
        //
        // **Wait for it to start before waiting for it to stop.**
        // `speakUtterance` queues and returns; `isSpeaking()` is still false
        // for a moment afterwards while the audio session and synthesiser
        // spin up. A loop that only waits while speaking therefore exits
        // immediately and reports a four-second sentence spoken in 0.4s —
        // which is #61 on Android exactly, where `speak` returned on queueing
        // and `--wait` lied to its caller about having been heard.
        let deadline = std::time::Instant::now() + START_GRACE;
        let mut started = false;
        loop {
            let speaking = self
                .synth
                .get_on_main(|synth| unsafe { synth.isSpeaking() });
            if speaking {
                started = true;
            } else if started || std::time::Instant::now() >= deadline {
                // Either it spoke and finished, or it never started within
                // the grace. The second is not treated as an error: an
                // utterance short enough to complete inside the grace looks
                // identical from here, and reporting a failure for a message
                // that was spoken would be the worse of the two mistakes.
                break;
            }
            std::thread::sleep(POLL);
        }
        Ok(())
    }

    fn voices(&self) -> Vec<Voice> {
        run_on_main(|_mtm| unsafe {
            AVSpeechSynthesisVoice::speechVoices()
                .iter()
                // `Voice` carries no language field, and the language is the
                // thing that distinguishes two voices with the same name —
                // there is a Karen in en-AU and a Karen in en-US. It goes in
                // the name because that is what the picker shows.
                .map(|v| Voice {
                    id: v.identifier().to_string(),
                    name: format!("{} ({})", v.name(), v.language()),
                })
                .collect()
        })
    }

    fn current_voice(&self) -> Option<String> {
        // The chosen one if there is one; otherwise whatever the system
        // would use, which is the first it offers.
        self.voice
            .lock()
            .expect("voice lock")
            .clone()
            .or_else(|| self.voices().first().map(|v| v.id.clone()))
    }

    fn set_voice(&self, id: &str) -> Result<(), EngineError> {
        // Checked here rather than at speaking time, so a bad id is a
        // refusal the setter can report instead of a silent fallback later.
        let known = self.voices().iter().any(|v| v.id == id);
        if !known {
            return Err(EngineError::Unavailable(format!(
                "no voice with id {id} on this device"
            )));
        }
        *self.voice.lock().expect("voice lock") = Some(id.to_string());
        Ok(())
    }

    fn rate(&self) -> f32 {
        *self.rate.lock().expect("rate lock")
    }

    fn set_rate(&self, rate: f32) -> Result<(), EngineError> {
        *self.rate.lock().expect("rate lock") = rate;
        Ok(())
    }

    fn stop(&self) {
        self.synth.get_on_main(|synth| unsafe {
            synth.stopSpeakingAtBoundary(AVSpeechBoundary::Immediate);
        });
    }

    fn tier(&self) -> Tier {
        // The platform's own synthesiser is what iOS speech *is*, not a
        // stand-in for something better that failed to load.
        Tier::Full
    }
}
