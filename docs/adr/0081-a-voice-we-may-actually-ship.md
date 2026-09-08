# 81. A voice we may actually ship

**Status:** Accepted.

**Chosen:** the default voice is `en_US-ljspeech-medium`, trained on the LJ
Speech corpus, which is **public domain**. It replaces
`en_US-lessac-medium` in `xtask` and in the Flatpak manifest, and it is the
same quality tier and within a megabyte of the same size — 63.5MB against
63.4MB — so nothing about the download or the bundle changes shape.

**This closes the first of decision 74's two blockers**, which is the one that
could not be worked around. The Blizzard Challenge 2013 Lessac corpus grants
use "exclusively for Research Purposes only", bars any "commercial purpose,
including the development... of voice synthesis... products", and bars
distribution outright. We staged that model into the bundle. Unbundling it
would not have helped much either: a first-run download that fetches it *for*
the user is automating a use the grant does not cover.

**The licence was verified at the corpus, not at the model card.** LJ Speech's
own page says: *"There are no restrictions on its use... you may use it
without attribution"*, and cites LibriVox's public-domain status. The Hugging
Face repository is labelled `License: mit` for every voice in it, including
the one that bars redistribution, so the badge is worth nothing — the model
card's dataset link is the thing to follow, which is how the original problem
was found.

**And it was heard before it was pinned.** Downloaded, hashed, the pins
written, `cargo xtask piper` run against them, then a node started in an
isolated home and made to speak. `spoken` in 2.8 seconds for a two-and-a-half
second sentence, which is a number that could have come out wrong.

**The isolation needed fixing first, which is worth recording.** The first
attempt overrode `HOME` and the node found the *real* voices anyway, because
`directories` reads `XDG_DATA_HOME` first and the systemd user manager had it
set. So the test reported the old voice and looked like a failure of the
change. The instrument, again.

**`pretty_voice_name` gains one exception.** Capitalising the first letter is
right for every ordinary given name upstream ships and turns this one into
"Ljspeech", which reads as a typo — and it is now the first thing every user
sees in `status` and in the voice picker. A table of one is a poor thing to
grow; if it reaches half a dozen the answer is a display name in the sidecar
config rather than a longer table.

**Costs.** A different voice: LJ Speech is a single US female reader from
LibriVox, and anyone used to Lessac will notice. Existing installs keep
whatever they already have, since `install_roots` prefers a user-installed
copy — so this changes what a *new* install gets, not what an old one uses,
and nobody is upgraded into a new voice without asking.

**Still open from decision 74:** the GPL-3.0 espeak-ng inside the Piper
payload. Unbundling on Linux and Windows is the remaining half, and macOS no
longer has the problem at all now that it uses the platform engine (#132).
