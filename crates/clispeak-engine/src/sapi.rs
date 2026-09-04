//! Speech on Windows, through SAPI 5.
//!
//! **Named `sapi` rather than `windows` on purpose.** A module called
//! `windows` in a crate that also depends on a crate called `windows` makes
//! every `use windows::…` below ambiguous, which is a compile error with a
//! confusing message. The type is still `WindowsEngine`, matching
//! `AppleEngine` and `AndroidEngine`.
//!
//! **Why Windows stopped using Piper** (#132, and Patrick's decision of
//! 4 September 2026). Piper is a native binary we spawn, and on Windows it
//! links the Visual C++ runtime that Windows does not ship — so on a clean
//! machine it installs correctly, is found correctly, and exits
//! `0xC0000135` with no message (#20). It is also why the installer had to
//! carry a runtime, a voice model and 300MB of engine with no terminal
//! (#30), and why the build machine could never reproduce the failure: any
//! Windows box able to compile has the Build Tools, which install the very
//! runtime whose absence is the bug.
//!
//! One engine ends all of that, and it is the same move macOS made. It also
//! takes GPL-3.0 espeak-ng out of a second artefact, leaving Linux as the
//! only platform shipping it — which is where copyleft is least of an
//! obstacle.
//!
//! # Why there is a thread in here
//!
//! `ISpVoice` is a COM object. COM apartments are per-thread: the object is
//! created on a thread that has called `CoInitializeEx`, and calling into it
//! from another thread is only sound through a marshalled proxy nobody here
//! is building. `SpeechEngine` requires `Send + Sync`.
//!
//! The obvious move is an `unsafe impl Send`. That would be an assertion
//! about COM's threading contract read out of documentation, on a platform
//! nobody here can run — precisely the kind of claim this project has been
//! wrong about repeatedly. So instead the object never leaves the thread
//! that made it: one worker owns it, everything else is a message, and there
//! is nothing to assert. The same shape as `AppleEngine`'s `MainThreadBound`
//! and for the same reason.
//!
//! # What is unverified
//!
//! **All of it.** Written against the bindings and Microsoft's documented
//! behaviour, type-checked for `x86_64-pc-windows-msvc`, and never run.
//! Nobody here has a Windows machine. The claim this makes is "compiles",
//! which `CLAUDE.md` is explicit is a weaker claim than "links" and much
//! weaker than "launched".

// FFI. Every SAPI call is an `unsafe fn` because it crosses into COM. That
// is a different thing from the `unsafe impl` avoided above: these are
// calls, not assertions about what may cross a thread.
#![allow(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use windows::Win32::Media::Speech::{
    ISpObjectTokenCategory, ISpVoice, SPCAT_VOICES, SPF_ASYNC, SPF_IS_NOT_XML,
    SPF_PURGEBEFORESPEAK, SPRS_IS_SPEAKING, SPVOICESTATUS, SpObjectTokenCategory, SpVoice,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::core::PCWSTR;

use crate::{EngineError, SpeechEngine, Tier, Voice};

/// How often a speaking chunk is checked for completion.
///
/// `speak` blocks, because the queue calls it that way. SAPI is asked to
/// speak asynchronously so that a `stop` can land mid-sentence, which makes
/// the wait a poll — the same shape as `child.rs` and `apple.rs`, for the
/// same reason (#58).
const POLL: std::time::Duration = std::time::Duration::from_millis(20);

/// How long to wait for a queued utterance to actually begin.
///
/// `Speak` with `SPF_ASYNC` queues and returns, so there is a window in
/// which nothing is speaking yet and nothing is wrong. A loop that only
/// waits while speaking would exit immediately and report a four-second
/// sentence spoken in no time — which is #61 on Android exactly, where
/// `speak` returned on queueing and `--wait` lied about having been heard.
const START_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// What the worker thread is asked to do.
enum Cmd {
    Speak {
        text: String,
        done: Sender<Result<(), EngineError>>,
    },
    Voices(Sender<Vec<Voice>>),
    SetVoice {
        id: String,
        reply: Sender<Result<(), EngineError>>,
    },
    SetRate(f32),
}

/// Speech through the platform synthesiser.
pub struct WindowsEngine {
    to_worker: Sender<Cmd>,
    /// Set by `stop`, read by the worker's poll loop.
    ///
    /// Not a `Cmd`, deliberately: a stop has to be seen *while* the worker
    /// is inside a chunk, and a message sitting in the channel would not be
    /// read until that chunk had finished — which is the opposite of what
    /// stopping means.
    stopping: Arc<AtomicBool>,
    voice: std::sync::Mutex<Option<String>>,
    rate: std::sync::Mutex<f32>,
}

impl WindowsEngine {
    /// Build one, or say why not.
    ///
    /// Blocks until the worker has initialised COM and created the voice, so
    /// a machine with no SAPI reports it here rather than on first speech.
    pub fn new() -> Result<Self, EngineError> {
        let (to_worker, from_engine) = channel::<Cmd>();
        let (started, startup) = channel::<Result<(), String>>();
        let stopping = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stopping);

        std::thread::Builder::new()
            .name("clispeak-sapi".into())
            .spawn(move || worker(&from_engine, &started, &flag))
            .map_err(|e| {
                EngineError::Unavailable(format!("could not start the speech thread: {e}"))
            })?;

        match startup.recv() {
            Ok(Ok(())) => Ok(Self {
                to_worker,
                stopping,
                voice: std::sync::Mutex::new(None),
                rate: std::sync::Mutex::new(1.0),
            }),
            Ok(Err(why)) => Err(EngineError::Unavailable(why)),
            // The thread died before saying anything, which is not something
            // that should happen and is not something to paper over.
            Err(_) => Err(EngineError::Unavailable(
                "the speech thread stopped before it reported whether it had started".into(),
            )),
        }
    }

    /// Ask the worker for something, and say so if it has gone.
    fn ask<T>(&self, cmd: Cmd, reply: Receiver<T>) -> Result<T, EngineError> {
        self.to_worker.send(cmd).map_err(|_| {
            EngineError::Unavailable("the speech thread is no longer running".into())
        })?;
        reply.recv().map_err(|_| {
            EngineError::Unavailable("the speech thread stopped while answering".into())
        })
    }
}

impl SpeechEngine for WindowsEngine {
    fn speak(&self, chunk: &str) -> Result<(), EngineError> {
        // Cleared here rather than after a stop, so a stop that arrives
        // between two chunks cannot silence the next message as well.
        self.stopping.store(false, Ordering::SeqCst);
        let (done, wait) = channel();
        self.ask(
            Cmd::Speak {
                text: chunk.to_string(),
                done,
            },
            wait,
        )?
    }

    fn voices(&self) -> Vec<Voice> {
        let (reply, answer) = channel();
        // An engine that cannot list its voices lists none. The alternative
        // is a panic on a picker being opened.
        self.ask(Cmd::Voices(reply), answer).unwrap_or_default()
    }

    fn current_voice(&self) -> Option<String> {
        self.voice
            .lock()
            .expect("voice lock")
            .clone()
            .or_else(|| self.voices().first().map(|v| v.id.clone()))
    }

    fn set_voice(&self, id: &str) -> Result<(), EngineError> {
        let (reply, answer) = channel();
        self.ask(
            Cmd::SetVoice {
                id: id.to_string(),
                reply,
            },
            answer,
        )??;
        *self.voice.lock().expect("voice lock") = Some(id.to_string());
        Ok(())
    }

    fn rate(&self) -> f32 {
        *self.rate.lock().expect("rate lock")
    }

    fn set_rate(&self, rate: f32) -> Result<(), EngineError> {
        *self.rate.lock().expect("rate lock") = rate;
        self.to_worker
            .send(Cmd::SetRate(rate))
            .map_err(|_| EngineError::Unavailable("the speech thread is no longer running".into()))
    }

    fn stop(&self) {
        // The flag, not a message: see the field's comment. The worker turns
        // this into a purge on the thread that owns the voice.
        self.stopping.store(true, Ordering::SeqCst);
    }

    fn tier(&self) -> Tier {
        // The platform's own synthesiser is what speech on Windows *is*, not
        // a stand-in for something better that failed to load.
        Tier::Full
    }
}

/// Wide, NUL-terminated, and kept alive for the duration of the call.
///
/// `PCWSTR` borrows; a temporary would be freed before SAPI read it. The
/// buffer is returned alongside so the caller holds it.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Everything that touches COM, on the one thread that may.
fn worker(
    from_engine: &Receiver<Cmd>,
    started: &Sender<Result<(), String>>,
    stopping: &AtomicBool,
) {
    // Multithreaded apartment: this thread services a queue and pumps no
    // window messages, which is what a single-threaded apartment would
    // require. SAPI's voice object supports both.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if com.is_err() {
        let _ = started.send(Err(format!("COM could not be initialised: {com:?}")));
        return;
    }

    let voice: ISpVoice = match unsafe { CoCreateInstance(&SpVoice, None, CLSCTX_ALL) } {
        Ok(v) => v,
        Err(e) => {
            let _ = started.send(Err(format!(
                "this machine has no SAPI speech voice available: {e}"
            )));
            unsafe { CoUninitialize() };
            return;
        }
    };
    let _ = started.send(Ok(()));

    while let Ok(cmd) = from_engine.recv() {
        match cmd {
            Cmd::Speak { text, done } => {
                let _ = done.send(speak_one(&voice, &text, stopping));
            }
            Cmd::Voices(reply) => {
                let _ = reply.send(list_voices());
            }
            Cmd::SetVoice { id, reply } => {
                let _ = reply.send(choose_voice(&voice, &id));
            }
            Cmd::SetRate(multiplier) => unsafe {
                // Ignored rather than reported: the setter has already
                // returned, and a rate that would not take is a worse thing
                // to learn about by panicking on a background thread.
                let _ = voice.SetRate(crate::scale::sapi_rate(multiplier));
            },
        }
    }

    unsafe { CoUninitialize() };
}

/// Speak one chunk and wait for it, watching for a stop.
fn speak_one(voice: &ISpVoice, text: &str, stopping: &AtomicBool) -> Result<(), EngineError> {
    let buf = wide(text);
    // `SPF_IS_NOT_XML` because the text is a person's message, and a message
    // containing something that looks like a SAPI tag must be *spoken*, not
    // interpreted. Without it a device name or a quoted fragment could
    // change the voice or the rate of the machine reading it aloud.
    let flags = (SPF_ASYNC.0 | SPF_PURGEBEFORESPEAK.0 | SPF_IS_NOT_XML.0) as u32;
    unsafe { voice.Speak(PCWSTR(buf.as_ptr()), flags, None) }.map_err(|e| EngineError::Failed {
        command: "SAPI Speak".into(),
        code: format!("{:#010x}", e.code().0),
        detail: Some(e.message()),
    })?;

    let deadline = std::time::Instant::now() + START_GRACE;
    let mut began = false;
    loop {
        if stopping.load(Ordering::SeqCst) {
            // A purge with nothing to say is SAPI's stop: it drops what is
            // queued and cuts off what is speaking.
            unsafe {
                let _ = voice.Speak(PCWSTR::null(), SPF_PURGEBEFORESPEAK.0 as u32, None);
            };
            return Ok(());
        }
        let mut status = SPVOICESTATUS::default();
        if unsafe { voice.GetStatus(&mut status, std::ptr::null_mut()) }.is_err() {
            // The voice stopped answering. Reporting success would be the
            // lie this project forbids; the caller can retry or fall back.
            return Err(EngineError::Failed {
                command: "SAPI GetStatus".into(),
                code: "no status".into(),
                detail: Some("the voice stopped answering while speaking".into()),
            });
        }
        let speaking = status.dwRunningState & (SPRS_IS_SPEAKING.0 as u32) != 0;
        if speaking {
            began = true;
        } else if began || std::time::Instant::now() >= deadline {
            // Either it spoke and finished, or it never started within the
            // grace. The second is not an error: a chunk short enough to
            // finish inside the grace looks identical from here, and
            // reporting a failure for something that *was* spoken is the
            // worse of the two mistakes.
            return Ok(());
        }
        std::thread::sleep(POLL);
    }
}

/// Every voice SAPI offers on this machine.
fn list_voices() -> Vec<Voice> {
    let mut found = Vec::new();
    let category: ISpObjectTokenCategory =
        match unsafe { CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_ALL) } {
            Ok(c) => c,
            Err(_) => return found,
        };
    if unsafe { category.SetId(SPCAT_VOICES, false) }.is_err() {
        return found;
    }
    let Ok(tokens) = (unsafe { category.EnumTokens(PCWSTR::null(), PCWSTR::null()) }) else {
        return found;
    };

    loop {
        let mut one = [const { None }; 1];
        let mut fetched = 0u32;
        if unsafe { tokens.Next(1, one.as_mut_ptr(), Some(&mut fetched)) }.is_err() || fetched == 0
        {
            break;
        }
        let Some(token) = one[0].take() else { break };
        let Ok(id) = (unsafe { token.GetId() }) else {
            continue;
        };
        // The token's default value is its description — "Microsoft Zira
        // Desktop - English (United States)" and the like. A voice with no
        // description is still usable, so it is listed under its id rather
        // than dropped.
        let name = unsafe { token.GetStringValue(PCWSTR::null()) }
            .ok()
            .and_then(|w| unsafe { w.to_string() }.ok());
        let id = match unsafe { id.to_string() } {
            Ok(id) => id,
            Err(_) => continue,
        };
        found.push(Voice {
            name: name.unwrap_or_else(|| id.clone()),
            id,
        });
    }
    found
}

/// Point the voice object at one of the tokens `list_voices` reported.
fn choose_voice(voice: &ISpVoice, id: &str) -> Result<(), EngineError> {
    let category: ISpObjectTokenCategory = unsafe {
        CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_ALL)
    }
    .map_err(|e| EngineError::Unavailable(format!("could not read the list of voices: {e}")))?;
    unsafe { category.SetId(SPCAT_VOICES, false) }
        .map_err(|e| EngineError::Unavailable(format!("could not open the voice list: {e}")))?;
    let tokens = unsafe { category.EnumTokens(PCWSTR::null(), PCWSTR::null()) }
        .map_err(|e| EngineError::Unavailable(format!("could not list voices: {e}")))?;

    loop {
        let mut one = [const { None }; 1];
        let mut fetched = 0u32;
        if unsafe { tokens.Next(1, one.as_mut_ptr(), Some(&mut fetched)) }.is_err() || fetched == 0
        {
            break;
        }
        let Some(token) = one[0].take() else { break };
        let matches = unsafe { token.GetId() }
            .ok()
            .and_then(|w| unsafe { w.to_string() }.ok())
            .is_some_and(|got| got == id);
        if matches {
            return unsafe { voice.SetVoice(&token) }.map_err(|e| {
                EngineError::Unavailable(format!("that voice could not be selected: {e}"))
            });
        }
    }
    // Checked here rather than at speaking time, so a bad id is a refusal
    // the setter reports instead of a silent fallback later.
    Err(EngineError::Unavailable(format!(
        "no voice with id {id} on this machine"
    )))
}
