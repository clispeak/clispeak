# 118. The browser probes run in CI, because the reason they did not was wrong

**Status:** Accepted.

**Chosen:** `app/tests/harness.mjs` runs as a `frontend probes` job on every
pull request and every push to `main`. The merge rule in `CLAUDE.md` moves
from five named checks to six.

**Why.** The five probes are the best-evidenced tests in this repository.
Each was written against a bug that had shipped — a list rebuilt on a poll
throwing away an expanded message and a focused button (#74), four dialogs
claiming `aria-modal` while letting Tab walk out behind the backdrop (#75), a
pause that hid the only control that could undo it (#109), an error toast that
could never be dismissed (#144), a skill path with no way back to the default
— and every one was confirmed to fail before its fix rather than written
afterwards.

They ran when somebody remembered.

**The reason they were manual was not checked and was false.** The file said
it needed "a real Chrome, which the build images do not carry, and pulling one
in to run two probes is a poor trade". `ubuntu-latest` ships Google Chrome
*and* Chromium preinstalled, and has throughout. The trade being declined did
not exist.

That is the pattern this project keeps paying for, in a new place: not a
failure, an absence, wearing a reason nobody re-read. The cost was asserted
rather than measured — and an asserted cost is worth exactly what an unrun
test is.

**What it costs.** A few seconds of Linux on every pull request. No cargo, no
build, no toolchain: node and a headless browser that is already on the image.
It is not filtered by path, deliberately — a path filter is another place an
absence can hide, and this job exists because of one.

**What moves with it.** The merge rule counts names, so the number is part of
the rule and going out of date by one is how a pull request passes with its
newest check unrun. `CLAUDE.md` says six, names the sixth, and says to move
the number in the same change next time.

**What it still does not cover.** The probes drive the interface against a
stubbed `window.__TAURI__`, so they prove what the page does with the answers
it is given, not that the node gives those answers. The join-flow contract and
hostile-string rendering named in #80 are still unwritten.
