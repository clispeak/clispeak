# 102. The Mac speaks, and backgrounding costs nothing

**Status:** Accepted.

Decision 91 moved macOS to `AVSpeechSynthesizer` and merged with two claims
unmeasured, said so in the entry, in the README and in `docs/architecture.md`,
and #154's title carried "(speech unmeasured)" so that nobody had to read any
of them to know. Both are now measured.

| | |
|---|---|
| speaks at all | **yes** — `engine: Samantha (en-US)`, heard |
| speaks while backgrounded | **yes** — heard with another app in front |
| backgrounded, end to end from the laptop | **5.0s** |
| foregrounded, same route | **5.7s** |

**Backgrounding costs nothing measurable.** The two numbers are within the
variation of the network hop they both include, and the backgrounded one is
the *faster* of the two — which is not a claim that backgrounding helps, it is
the clearest way of saying the difference is noise.

That was the property in doubt. `AVSpeechSynthesizer` dispatches to the main
thread's run loop, and a Tauri app pumps that with its window hidden; the
worry was that macOS would throttle a backgrounded app enough to matter. It
does not.

**Piper's 3.3s is not a like-for-like comparison and must not be read as a
regression.** It was measured on that machine by a different route, and both
figures are dominated by reaching the Mac over the network rather than by
synthesis. Recording it as "the number to match" was the wrong framing, and
carrying it into a comparison would have manufactured a 50% slowdown out of
two measurements of different things.

**What this closes.** `CLAUDE.md` distinguishes compiled, linked and launched,
and macOS was at *launched* with a working engine unheard. It is now heard,
which is the only one of the three that ever mattered to a person. The
documents that said otherwise have been corrected rather than left to age.

**Still unmeasured, and still said so:** Windows. SAPI 5 is compiled and has
never run — no one here has a Windows machine, and a 7.1MB installer is
waiting for a clean box.
