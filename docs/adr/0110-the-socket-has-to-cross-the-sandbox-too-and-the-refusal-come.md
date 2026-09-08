# 110. The socket has to cross the sandbox too, and the refusal comes back out

**Status:** Superseded by [0111](0111-one-directory-with-two-names-and-the-refusal-comes-back.md).

Two corrections to today's own work, found by Patrick doing a clean-room
install of the Flatpak on 6 September 2026. Both were invisible to every gate
and to every developer machine, because a developer machine already has the
app's state and a clean one does not.

### The socket did not cross, and nothing said so

Decision 103 moved the socket out of a namespaced name and into a private
directory under `$XDG_RUNTIME_DIR`. That was right, and it quietly broke the
Flatpak's command-line tool.

Until then the socket was an *abstract* socket on Linux, living in the network
namespace — and `--share=network` already shares that, so it crossed the
sandbox boundary for free, without anyone deciding it should. A filesystem
socket does not: Flatpak gives the app its own `/run/user/1000`, so the node
and the host CLI print the same path and mean different directories.

```
host     /run/user/1000/clispeak/    empty
sandbox  $XDG_RUNTIME_DIR/clispeak/  holds clispeak.sock
```

The symptom is total and misleading: `clispeak` reports that **nothing is
listening** while the app is plainly running and visible in the tray. One more
grant fixes it, `--filesystem=xdg-run/clispeak:create`, proven by applying it
as an override before writing it into the manifest.

**Decision 107 was half of this.** The token and the socket are the same wound
— two files the sandbox keeps to itself that the host CLI must reach — and
only one of them was noticed, because the token failure produced an error
about *identity* and the socket failure had not been reached yet.

### And decision 108's refusal is downgraded to a warning

The identity-conflict check refused to start on a **clean install**: nothing
installed, both config directories deleted first, and it still reported two
copies of this device's state.

The observed state contradicts the code. The keyring marker is written through
`config_dir()`, which is the same function the check reads — and yet the
marker landed in the *host* directory while the check believed its own
directory was the sandbox. That is not explained, and it is not something to
guess at.

**A check that stops a node from starting for a reason nobody can account for
is worse than the bug it guards.** This one would have met every new Linux
user before it met a second developer. So it warns, naming both directories
and saying exactly what goes wrong — which was always the part worth having —
and the refusal comes back when the behaviour is understood rather than when
the pressure is off.

**What both have in common** is the thing this repository keeps writing down.
Neither is reachable on a machine that has ever run the app before. The clean
room is the test, and until today nobody had used one.
