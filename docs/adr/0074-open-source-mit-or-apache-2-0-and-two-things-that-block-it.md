# 74. Open source, MIT or Apache-2.0, and two things that block it

**Status:** Accepted.

**Chosen:** the project goes open source under **MIT OR Apache-2.0**, binaries
are published from GitHub Releases first and app stores later if at all, and
the site is GitHub Pages. Patrick's call on the licensing question (#24),
which has gated the release chain since it was opened. The full working is
`docs/licensing.md`.

**Two blockers were found while researching it, and both exist today.**

**The default voice forbids redistribution.** `en_US-lessac-medium` is trained
on the Blizzard Challenge 2013 Lessac corpus, whose licence grants use "for
Research Purposes only", bars any "commercial purpose, including the
development... of voice synthesis... products", and bars distribution outright.
We stage that `.onnx` into the app bundle. The Hugging Face repository is
labelled `License: mit` and that label is not the operative licence — the model
card links the corpus terms precisely because they differ, which is a trap for
anyone who reads the badge and stops.

**We ship GPL-3.0 code with no licence text and no offer of source.** This
half was already in `docs/releasing.md`, which named espeak-ng's licence, said
the archive carries no licence text, and then named the actual gap — *"Nobody
has read the terms."* What is new is that somebody has, and that reading is
where the App Store consequence and the voice corpus came from; neither is
visible from knowing a licence name.

`espeak-ng` and `libespeak-ng.so` are in the bundle; eSpeak NG is
GPL-3.0-or-later. There is no `LICENSE` or `COPYING` anywhere under
`app/src-tauri/speech/` — that is a compliance gap that exists now, before
anything is published. It also closes the iOS App Store, whose terms impose
restrictions GPL forbids adding; this is why VLC was pulled in 2011.

**Our own licence is not forced to GPL, and the reason is architectural.**
Piper and espeak are *spawned as separate processes*, not linked —
`voicecast-engine/src/piper.rs` opens by saying so. Arm's-length process
invocation is the standard basis for aggregation rather than combination. A
design decision taken for other reasons turns out to be what keeps the licence
choice open.

**One change fixes both blockers: stop bundling the speech payload and fetch
it on first run.** `cargo xtask piper` already downloads it, and decision 52
already made the engine tolerate one that is absent and appears later. The
distributed artefact then contains no GPL code and no restricted voice. The
cost is an app that cannot speak until a first-run download finishes, which is
a real regression in first impressions and the honest price.

**Why not copyleft.** GPL would trade the App Store for a protection this
project does not need: the value is the network of your own devices, not the
code, so nobody can take voicecast proprietary in a way that hurts us. AGPL
targets network services and there is no server here.

**Measured rather than assumed:** 703 of 703 crates are permissively licensed
with no GPL, AGPL or SSPL anywhere; the six MPL-2.0 crates impose nothing since
we do not modify them; and all nine of our own crates currently have no
`license` field at all.

**Costs.** A first-run download. An annual target-API commitment if Play is
ever used. `cargo-deny` becomes another gate. And the licence text is a careful
reading by someone who is not a lawyer — the two blockers are worth thirty
minutes of one before a public release, which is written into the document
rather than left implied.
