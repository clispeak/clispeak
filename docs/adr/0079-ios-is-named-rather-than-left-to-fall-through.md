# 79. iOS is named rather than left to fall through

**Status:** Accepted.

**Chosen:** `speech_engine()` gains a `#[cfg(target_os = "ios")]` arm
returning `SilentEngine`, and `voicecast-engine` stops compiling `espeak` and
`piper` for iOS at all.

iOS is unix and is not Android, so it took the first arm and would have called
`PiperEngine::discover()` and `EspeakEngine::new()` — both of which spawn a
child process, which iOS does not permit. Nobody wrote that. It was inherited
from phrasing the exclusion as "unix, and not Android", so **the code
disagreed with a decision its owner had already made out loud**: "if we ever
do iOS we will use iOS."

**Nothing would have caught it.** It compiles, every gate passes, the
portability gate is about `voicecast-core` and these conditionals live in the
two crates that are supposed to have them, and the five-target matrix builds
iOS — building being exactly what succeeds here. It is wrong only on the one
target nobody has run, and would have surfaced the day someone did, as a new
problem rather than a known one.

**Confirmed by compiling, not by reading.** A throwaway example calling both
engines was checked against `aarch64-apple-ios`:

```
before   Finished                                   <- both engines present
after    error[E0433]: cannot find `EspeakEngine`   <- gone
         error[E0433]: cannot find `PiperEngine`
```

and the same example still compiles for the host, so the gating removed them
from iOS without removing them from anywhere else.

**Both halves are locally verified, in the end.** The app crate's arm could
not be checked at first — `SDK "iphoneos" cannot be located`, because the iOS
SDK ships with Xcode and only the Command Line Tools were installed. Patrick
installed Xcode while this was being written, and
`cargo check -p voicecast-app --target aarch64-apple-ios` now finishes clean.
The distinction was worth writing down before it was resolved rather than
after: on a target nobody runs, which half was proven and which was assumed is
the whole of what the claim is worth.

**And the compiler found a loose end the fix had left.** With both spawning
engines gone from iOS, `child` — the module that waits on and kills a spawned
process — compiled for a phone with nothing able to reach it. Two dead-code
warnings on an `aarch64-apple-ios` check said so, and nobody would ever have
seen them: CI runs clippy on Linux only, and the matrix jobs build without
`-D warnings`. It is now gated with its callers.

**The reason string is the other half of the fix.** "No speech engine is
installed" sends a reader to install something; iOS has nothing to install
yet. The arm says the device can join a space, receive messages and keep their
history but cannot speak them — which is true, is the behaviour that already
exists, and is the difference between a fixable fault and a missing feature.

**Costs.** A fourth arm to keep in step, and one more place that has to change
when `AVSpeechSynthesizer` arrives. Deliberately not doing that now: the
smallest honest thing is to stop claiming an engine, and building a real one
is a separate piece of work nobody has started.
