# 17. espeak-ng floor, Piper downloaded, fallback made visible

**Status:** Accepted.

**Decision.** Linux depends on distro espeak-ng as a guaranteed floor and
downloads Piper on first run, preferring it once present. Fallback state is
shown in the UI with a machine-readable reason, and travels in `Presence` so
other devices see it too.

**Why not bundle the model.** ~60MB on every package, a packaging fight over
binary blobs, and decisively: one bundled voice serves only English speakers.
Everyone else would get a tool that cannot speak their language.

**Why not download-only.** First run would fail without network, and the
message would be lost rather than merely ugly.

**Why degrade quietly here, when text validation fails loudly.** Not an
inconsistency — the difference is who can act on the failure. A markdown error
reaches an agent that rewrites and retries in one round trip. A missing voice
model can only be fixed by a human who is not watching, so failing loudly
would just lose the message. **Loud failure is right when someone can respond
to it.**

**Why fallback must explain itself.** The real risk of a silent fallback is
that someone hears espeak, assumes that is the product, and never discovers a
better voice was one click away. So the UI states which engine is active, why,
what to do about it, and — equally important — that speech is working and no
fault needs hunting. `user_selected` is a reason code precisely so a
deliberate espeak user is reported rather than nagged.

**Cost.** `no_engine` becomes nearly unreachable on Linux, which is the point,
but it means quality varies silently unless the UI does its job.
