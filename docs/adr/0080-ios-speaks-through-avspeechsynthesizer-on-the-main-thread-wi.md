# 80. iOS speaks through AVSpeechSynthesizer, on the main thread, without an unsafe claim

**Status:** Accepted.

**Chosen:** an `IosEngine` wrapping `AVSpeechSynthesizer`, held in
`dispatch2::MainThreadBound` and touched only through `run_on_main`.

Decision 74 stopped iOS pretending it could spawn Piper and left it saying so.
This is the engine that makes the sentence unnecessary.

**No `unsafe impl Send`, deliberately.** `SpeechEngine` requires `Send +
Sync`; `Retained<AVSpeechSynthesizer>` is neither. The obvious move is to
assert it — and that assertion is a claim about AVFoundation's threading
contract, read out of Apple's documentation, which is the move that has been
wrong repeatedly. `MainThreadBound` needs no claim: the synthesiser is
created, used and dropped on the main thread and never crosses a boundary, so
there is nothing to assert. The `unsafe impl` that makes it work is
`dispatch2`'s, made once with its reasoning beside it.

The lint would have allowed it — the workspace `forbid` is downgraded to
`deny` in this crate and `android.rs` already carries an `#[allow]`. So the
cost avoided was not configuration. It was the claim.

**The main thread specifically, not a thread of our own.** A synthesiser on a
bare background thread has no run loop, and the failure that produces is
`isSpeaking()` answering correctly while no audio ever starts — an engine that
looks like it is working. Suggested by the lead before a line was written,
which is cheaper than finding it afterwards.

**The audio session is part of the engine, not a follow-up.** iOS mutes
`AVSpeechSynthesizer` behind the hardware silent switch unless the session
category is playback. Correct for most apps and fatal for this one: the point
is being heard when nobody is looking at the screen, and a phone in a pocket
has the switch on. A failure to set it is reported rather than swallowed —
"it spoke and you heard nothing" is the silence this project forbids.

**`speak` blocks by polling**, because the queue calls it synchronously and
every other engine waits on its process. Nothing is held across the sleep:
`get_on_main` returns a `bool` and carries no lock, which is `child.rs`'s
shape for `child.rs`'s reason (#58).

**Two link failures found on the way, both the same shape.** Objective-C
*classes* resolve through the runtime at load; C *constants* need the
framework linked at build time. `AVSpeechSynthesizer` linked and
`AVSpeechUtteranceDefaultSpeechRate` did not, which is why `cargo check`
passed and the app link failed — a check never links. `AVFAudio` joins
`SystemConfiguration` in `bundle.iOS.frameworks`, and both are now declared in
config rather than hand-edited into a generated project.

**What this does and does not establish.** iOS was compiled, then linked, then
launched, and now **spoke** — confirmed on the simulator by the system loading
`com.apple.AudioUnit-Speech` and its MacinTalk and Siri synthesis units. It
does not establish that a *node* works there: binding a local socket, holding
an identity, and reaching a peer are all untested, and #129 now requires a
token from a config directory iOS sandboxes differently. The engine buys the
fourth claim only.

**And the silent switch is untested.** The simulator may not model it at all.
That is the first thing to check on a real phone, and until then it is
reasoning rather than a result.

**Costs.** A fourth engine to keep in step. `objc2-avf-audio` and `dispatch2`
as iOS-only dependencies. And a poll every 20ms while speaking, which is the
same trade `child.rs` already made.
