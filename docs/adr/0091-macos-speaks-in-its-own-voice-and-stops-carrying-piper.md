# 91. macOS speaks in its own voice, and stops carrying Piper

**Status:** Accepted.

**Chosen:** macOS uses `AVSpeechSynthesizer` through the engine iOS already
needed, and `xtask bundle` stages no speech payload there.

Patrick decided this; #132 costed it as option 4 and it is the only one that
closes all three of that issue's problems at once.

**Piper on macOS was three problems wearing one coat.** Upstream's macOS build
ships without an rpath and without the dylibs it links against, so installing
it runs `otool`, `install_name_tool` and `codesign` — Xcode Command Line
Tools, not base macOS. "Download the engine on first run" therefore meant
"install a gigabyte of developer tooling first", which is not a first-run
download. `rhasspy/piper` was archived in October 2025 and that pinned release
is still the latest, so no corrected build is coming. And the maintained
successor is GPL-3.0 for the whole project, because it *embeds* espeak-ng
rather than spawning it.

One engine ends all three, and the second Apple engine was much cheaper than
the first because the first already existed (decision 80).

**Switching the engine does not deliver the licensing half, and that is the
part worth recording.** The first build with the new engine came out at
**208MB with `libespeak-ng.1.dylib` still inside**, because `xtask bundle`
stages the payload whether or not anything reads it. GPL-3.0 espeak-ng in the
artefact is a redistribution with obligations, and carrying it for an engine
the Mac no longer calls is the worst of both. So the bundler skips the payload
on macOS: **208MB to 32MB, and zero speech files.**

**And this is the first platform where the redistribution question is
answered rather than deferred.** Decision 74 makes the project MIT OR
Apache-2.0 while the speech payload is not: Piper, its phonemiser and ONNX
Runtime are MIT, espeak-ng is GPL-3.0-or-later, and the default voice carries
terms of its own. Fetching that onto your own machine is fine; putting it in
something handed to somebody else is the open question, and it is the last
thing between here and publishing anything. A macOS build that ships no speech
files is not a size optimisation — it is one platform where that question no
longer has to be answered at all. **After this change, Linux and Windows still
stage the payload; macOS does not.**

**macOS gets its own bundle config, because a declared resource that is absent
fails Tauri's own build-script check.** Removing the payload while leaving
`"resources": ["speech/**/*"]` in place would have broken the build rather
than shrunk it — the same shape as everything else here: a thing looked for in
the wrong place does not say so, it says nothing, and the build stops for a
reason that names a file rather than the decision that removed it. Windows
keeps both the payload and the original config — Piper is still its engine,
and unlike macOS its archive is self-contained.

**What it costs**, and it is a real cost rather than a footnote: a message no
longer sounds identical on every desktop. `docs/architecture.md` states that
as a goal, and it was really a *consequence* — Piper was the only engine
available everywhere, and uniformity followed from that rather than from a
decision. Linux and Windows keep Piper and keep sounding alike; macOS sounds
like macOS, the way Android already sounds like Android and nobody has ever
thought that a defect. The architecture update is the lead's.

**Speech through `AVSpeechSynthesizer` is UNMEASURED on this build, and so is
speech while backgrounded.** Both are to be verified on hardware. Patrick
decided to land it rather than hold it, which is his call to make; what would
not be acceptable is landing it in a way that reads as verified. It does not.
Nobody has heard this build speak.

The rest of this entry says what *was* measured, and the boundary matters. The bundle builds,
shrinks and signs, and both Apple targets compile. Whether the Mac *speaks*
through the new engine is unmeasured: the rename changed the identifier to
`org.clispeak.app`, so macOS treats it as a different application and asks for
a fresh keychain grant before the node binds. That prompt needs Patrick's
password and he is away. The keychain migration itself is implemented —
`PREVIOUS_SERVICE` reads the old `voicecast` item — so this is one
authorisation rather than a lost identity.

**And the background case is untested here too.** `AVSpeechSynthesizer`
dispatches to the main thread's run loop; a Tauri app pumps that even with the
window hidden, so it should hold, but "should" has been the wrong word often
enough that it is written down as unmeasured rather than assumed. Piper spoke
while backgrounded — measured, 3.3s with another app frontmost — and that
property has to be re-established for the new engine rather than inherited.
