# 96. Windows speaks in its own voice too

**Status:** Accepted.

Patrick, 4 September 2026: Windows moves to the platform synthesiser, as
macOS did in decision 91. **The reasons are not the same ones**, and that is
worth being precise about, because it is why this was a separate decision
rather than a consequence of the last.

macOS moved because Piper could not be *installed* there without Xcode. On
Windows Piper installs perfectly. It then links the Visual C++ runtime, which
Windows does not ship, so on a clean machine `piper.exe` installs correctly,
is discovered correctly, and exits `0xC0000135` with no message (#20). Three
open issues trace to that one fact:

- **#20** — whether we may redistribute Microsoft's runtime, a licensing
  question nobody here could answer.
- **#30** — an installer that had to carry Piper, a voice model, that runtime,
  and set a PATH, all without a terminal.
- **the build machine could never reproduce the bug**, because any Windows box
  able to compile has the Build Tools, which install the very runtime whose
  absence is the failure.

**One engine ends all three**, and it ends the fourth thing nobody had listed:
the Windows artefact stops carrying GPL-3.0 espeak-ng, which leaves Linux as
the only platform shipping it.

**What may remain of #20 is our own executable.** The `windows-msvc` target
links `vcruntime140.dll` by default. If it does, the answer is
`-C target-feature=+crt-static` on our own build — a flag, not a reading of
Microsoft's terms. That is the difference: #20 rejected static linking as
"much more work" because it meant rebuilding Piper's C++ and ONNX Runtime.
Statically linking a Rust binary is a build setting. **Unverified**, and #20
stays open until a clean Windows box speaks with no redistributable present.

**SAPI 5 rather than WinRT.** `ISpVoice::Speak` goes to the default audio
device; `Windows.Media.SpeechSynthesis` hands back a stream that something
else has to play, which means an audio path to write and get wrong. SAPI is
older and is the one that speaks.

**A thread, for the same reason `AppleEngine` has `MainThreadBound`.** COM
apartments are per-thread and `SpeechEngine` requires `Send + Sync`. The
obvious move is an `unsafe impl Send` — an assertion about COM's threading
contract, read from documentation, about a platform nobody here can run. So
the object never leaves the thread that made it: one worker owns it,
everything else is a message, and there is nothing to assert.

**`stop` is a flag rather than a message**, because a stop has to be seen
*while* the worker is inside a chunk. A message would sit in the channel until
that chunk finished, which is the opposite of what stopping means.

**`SPF_IS_NOT_XML`, deliberately.** SAPI interprets its input as markup by
default, so a message containing something shaped like a SAPI tag would change
the voice or the rate of the machine reading it aloud. The text is a person's
message and must be spoken, not obeyed — the same rule as escaping peer text
at the point it is printed (decisions 38 and 87).

**And the rate conversion lives outside the Windows module so that it is
tested.** `cargo test` runs on `ubuntu-latest` and nowhere else, so a test
inside `#[cfg(windows)]` is not a test that runs on Windows — it is a test
that runs nowhere, which is worse than no test because the count says
otherwise. It was written that way first and the count is what gave it away:
nine tests passed and the new one was not among them.

**Costs, stated plainly.** *Nobody has run any of this.* It is type-checked
for `x86_64-pc-windows-msvc` locally and built by the matrix, which is
"compiled", the weakest of the three claims `CLAUDE.md` distinguishes.
Patrick's instruction was to write it blind, take the compile, and give
Windows a dedicated pass once the other platforms are stable.

Two things were caught by checking the target locally rather than by a matrix
run, which is the cheaper way round: `child.rs` became dead code on Windows
when Piper left, and `RUSTFLAGS: -D warnings` would have failed that job
rather than warned; and the rate test above.

Windows voices sound different from Piper. That cost was already paid in
decision 91 — this completes a direction rather than opening one.
