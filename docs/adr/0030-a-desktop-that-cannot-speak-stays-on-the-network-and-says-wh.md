# 30. A desktop that cannot speak stays on the network and says why

**Status:** Accepted.

**Decision.** When Piper will not start, only Linux falls back to espeak-ng.
macOS and Windows start anyway with a silent engine carrying Piper's own
error, and every path reports the reason Piper gave rather than a stand-in.

**What was actually wrong.** The daemon gated its espeak fallback on `unix`,
but decision 17 says *Linux* — espeak-ng is the Linux floor because
distributions provide it. Nothing on a Mac does, and the bundle carries only
Piper, so macOS only ever reached that branch's error path: it refused to
start, and the message recommended an Arch package. Nobody decided that. It
was a `cfg` that read one word wider than the decision under it.

**Why starting is better than refusing.** This is decision 22's reasoning,
which was superseded on Windows and applies unchanged here. Refusing takes a
node off the network over an install that can be fixed, and a device that
cannot speak is still worth having: it joins spaces, answers for itself, and
keeps its history. The sender learns more from a node that explains itself
than from one that is absent.

**Why the reason has to be Piper's.** The app's Unix branch reported "no
speech engine is installed on this device". On a Mac whose Piper is present
but broken — a missing dylib, a signature the OS refuses, an Intel binary on
arm64 — that is false, and it sends whoever reads it to install what they
already have. Piper's own error names the fault and lists where it looked.
It went to stderr, which for an app launched from Finder is nowhere.

**Cost.** macOS and Windows have no floor: a device whose Piper is broken says
nothing aloud until somebody fixes it. Bundling espeak-ng on those platforms
would buy a floor at the price of shipping a second engine that sounds
nothing like the first, which is the opposite of why every desktop runs Piper.
The `no_engine` status and its reason are what stand in instead.
