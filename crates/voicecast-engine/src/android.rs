//! Android speech, via `android.speech.tts.TextToSpeech`.
//!
//! Calls a small Kotlin object (`com.voicecast.app.Speech`) rather than
//! driving `TextToSpeech` directly. That API initialises asynchronously
//! through a listener callback, which is awkward to orchestrate from raw JNI,
//! and it needs a `Context` — so the Kotlin side owns both, and Rust is left
//! with four plain static calls and no jobject to keep alive.
//!
//! The `JavaVM` is captured in `JNI_OnLoad`, which Android calls when it loads
//! our library, long before any of this runs.

use std::sync::OnceLock;

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};

use crate::{EngineError, SpeechEngine, Tier, Voice};

/// The JVM, captured when Android loads this library.
static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();

/// A global reference to the Kotlin helper class.
///
/// Resolved once in `JNI_OnLoad` and kept, because `FindClass` called from a
/// thread the JVM did not create searches the *system* class loader, which
/// knows nothing about application classes. Speech runs on a plain Rust
/// worker thread, so looking the class up by name there fails with
/// `ClassNotFoundException` — the error is about the class loader, not a
/// missing class.
static SPEECH_CLASS: OnceLock<GlobalRef> = OnceLock::new();

/// Fully-qualified name of the Kotlin helper.
const SPEECH_CLASS_NAME: &str = "com/voicecast/app/Speech";

/// Called by Android when the native library is loaded.
///
/// # Safety
/// Invoked by the JVM with a valid `JavaVM` pointer.
#[allow(unsafe_code, non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(vm: JavaVM, _reserved: *mut std::ffi::c_void) -> jni::sys::jint {
    // This runs on a Java thread, so the application class loader is in
    // scope. Resolve the helper class now and keep a global reference — later
    // calls come from Rust threads that could not find it themselves.
    if let Ok(mut env) = vm.attach_current_thread() {
        for (name, slot) in [
            (SPEECH_CLASS_NAME, &SPEECH_CLASS),
            (BATTERY_CLASS_NAME, &BATTERY_CLASS),
            (INVITES_CLASS_NAME, &INVITES_CLASS),
        ] {
            if let Ok(class) = env.find_class(name)
                && let Ok(global) = env.new_global_ref(class)
            {
                let _ = slot.set(global);
            }
        }
    }
    let _ = JAVA_VM.set(vm);
    jni::sys::JNI_VERSION_1_6
}

/// The cached helper class, or an error explaining why speech is unavailable.
fn speech_class() -> Result<&'static GlobalRef, EngineError> {
    SPEECH_CLASS.get().ok_or_else(|| {
        EngineError::Unavailable("the speech helper class was not found at load time".into())
    })
}

/// Fully-qualified name of the pending-invite holder.
const INVITES_CLASS_NAME: &str = "com/voicecast/app/Invites";

/// A global reference to the invite holder class. See [`SPEECH_CLASS`].
static INVITES_CLASS: OnceLock<GlobalRef> = OnceLock::new();

/// Take an invite opened from a QR scan, if one is waiting.
///
/// Scanning launches the activity with the invite in its intent, which can
/// happen before the interface exists — so Kotlin parks it and this collects
/// it once, clearing it so a rotation does not rejoin.
pub fn take_pending_invite() -> Option<String> {
    let class = INVITES_CLASS.get()?;
    AndroidEngine::with_env(|env| {
        let value = env.call_static_method(
            <&JClass>::from(class.as_obj()),
            "take",
            "()Ljava/lang/String;",
            &[],
        )?;
        let obj: JObject = value.l()?;
        if obj.is_null() {
            return Ok(None);
        }
        let s: String = env.get_string(&JString::from(obj))?.into();
        Ok(Some(s))
    })
    .ok()
    .flatten()
}

/// Fully-qualified name of the battery-optimisation helper.
const BATTERY_CLASS_NAME: &str = "com/voicecast/app/Battery";

/// A global reference to the battery helper class. See [`SPEECH_CLASS`].
static BATTERY_CLASS: OnceLock<GlobalRef> = OnceLock::new();

/// Whether Android will let this device keep receiving while asleep.
///
/// Lives in this crate because it owns the JNI boundary — `JNI_OnLoad` can
/// only be defined once, and class references must be resolved there. It is
/// not speech, but splitting it out would mean a second entry point that
/// cannot exist.
pub fn is_battery_exempt() -> bool {
    call_static_bool(&BATTERY_CLASS, "isExempt").unwrap_or(false)
}

/// Ask the user to exempt this app from battery optimisation.
pub fn request_battery_exemption() -> bool {
    call_static_bool(&BATTERY_CLASS, "requestExemption").unwrap_or(false)
}

/// Call a no-argument static method returning a boolean on a cached class.
fn call_static_bool(class: &OnceLock<GlobalRef>, method: &str) -> Result<bool, EngineError> {
    let class = class
        .get()
        .ok_or_else(|| EngineError::Unavailable("helper class not found at load time".into()))?;
    AndroidEngine::with_env(|env| {
        env.call_static_method(<&JClass>::from(class.as_obj()), method, "()Z", &[])?
            .z()
    })
}

/// Speaks through Android's system text-to-speech engine.
pub struct AndroidEngine;

impl AndroidEngine {
    /// Create the engine.
    ///
    /// Does not check readiness: `TextToSpeech` initialises asynchronously, so
    /// a device that is merely still starting up would be wrongly reported as
    /// having no engine at all. [`SpeechEngine::ready`] answers that per
    /// message instead.
    pub fn new() -> Result<Self, EngineError> {
        if JAVA_VM.get().is_none() {
            return Err(EngineError::Unavailable(
                "the Java VM was not captured; JNI_OnLoad did not run".into(),
            ));
        }
        speech_class()?;
        Ok(Self)
    }

    /// Run `f` with a JNI environment attached to this thread.
    ///
    /// Speech happens on a dedicated worker thread that the JVM knows nothing
    /// about, so it has to attach before making any call.
    fn with_env<T>(
        f: impl FnOnce(&mut JNIEnv) -> Result<T, jni::errors::Error>,
    ) -> Result<T, EngineError> {
        let vm = JAVA_VM
            .get()
            .ok_or_else(|| EngineError::Unavailable("no Java VM".into()))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| EngineError::Unavailable(format!("could not attach to the JVM: {e}")))?;
        f(&mut env).map_err(|e| EngineError::Unavailable(format!("speech call failed: {e}")))
    }

    /// Call a no-argument static method returning a boolean.
    fn call_bool(method: &str) -> Result<bool, EngineError> {
        let class = speech_class()?;
        Self::with_env(|env| {
            env.call_static_method(<&JClass>::from(class.as_obj()), method, "()Z", &[])?
                .z()
        })
    }
}

impl SpeechEngine for AndroidEngine {
    fn ready(&self) -> Result<(), EngineError> {
        if Self::call_bool("isReady")? {
            return Ok(());
        }
        // Ask Kotlin why, so the sender is told something it can act on
        // rather than a bare "not ready".
        let class = speech_class()?;
        let reason = Self::with_env(|env| {
            let value = env.call_static_method(
                <&JClass>::from(class.as_obj()),
                "failureReason",
                "()Ljava/lang/String;",
                &[],
            )?;
            let obj: JObject = value.l()?;
            if obj.is_null() {
                return Ok(None);
            }
            let s: String = env.get_string(&JString::from(obj))?.into();
            Ok(Some(s))
        })?;
        Err(EngineError::Unavailable(
            reason.unwrap_or_else(|| "the speech engine is not ready".into()),
        ))
    }

    fn speak(&self, chunk: &str) -> Result<(), EngineError> {
        let class = speech_class()?;
        let spoken = Self::with_env(|env| {
            let text = env.new_string(chunk)?;
            env.call_static_method(
                <&JClass>::from(class.as_obj()),
                "speak",
                "(Ljava/lang/String;)Z",
                &[JValue::Object(&text)],
            )?
            .z()
        })?;
        if spoken {
            Ok(())
        } else {
            Err(EngineError::Unavailable(
                "the speech engine refused the text".into(),
            ))
        }
    }

    fn voices(&self) -> Vec<Voice> {
        vec![Voice {
            id: "system".into(),
            name: "Android text-to-speech".into(),
        }]
    }

    fn stop(&self) {
        let Ok(class) = speech_class() else { return };
        let _ = Self::with_env(|env| {
            env.call_static_method(<&JClass>::from(class.as_obj()), "stop", "()V", &[])?;
            Ok(())
        });
    }

    fn tier(&self) -> Tier {
        // The platform's own engine, which is what the design intends on
        // Android — not a stand-in.
        Tier::Full
    }
}
