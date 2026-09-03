# Going open source: what it takes, and two things that block it today

Patrick's decision is to open-source the project, publish binaries, and put
the site on GitHub Pages. This is what that costs and what has to change
first.

**Everything below marked "measured" was checked against this repository or
the upstream source on 3 September 2026. Everything marked "needs a lawyer"
is me reading licence text, which is not the same thing.** The distinction
matters because two of today's expensive mistakes were careful reasoning
about something nobody had run.

## The proposal, in one place

| | |
|---|---|
| **Our code** | **MIT OR Apache-2.0**, the Rust ecosystem default |
| **Bundled speech engine** | **stop shipping it** — download on first run instead |
| **Default voice** | **must change**, the current one forbids redistribution |
| **Website** | GitHub Pages, static, no accounts, no data |
| **Terms of use** | not needed for the software; a privacy policy *is* needed for the stores |

Two of those are blockers. The rest is paperwork.

---

## Blocker 1: the voice we ship forbids redistribution

**Measured.** `xtask/src/piper.rs` pins `en_US-lessac-medium` and stages it
into the app bundle. Its Hugging Face model card names the training corpus as
Blizzard Challenge 2013 "lessac", and links its licence. That licence says:

> ...excludes...using the Materials for any commercial purpose, including the
> development, marketing, commercialisation, sale or licencing of voice
> synthesis or speech recognition products

> The User agrees not to lend, hire, sell, distribute or otherwise part with
> the Materials in any manner not consistent with this Agreement

It grants use "exclusively for Research Purposes only".

The Hugging Face repository is labelled `License: mit` at the top level. **That
label is not the operative licence for this voice** — the model card links the
corpus terms precisely because they differ, which is a trap laid for anyone who
reads the repository badge and stops.

Whether a free open-source release is a "commercial purpose" is arguable.
Whether shipping the `.onnx` inside a `.dmg` is "distributing the Materials" is
much less arguable, and the corpus licence bars distribution outright.

**What to do:** pick a default voice whose corpus permits redistribution, and
check it the same way — open the voice's `MODEL_CARD`, follow the dataset
link, read *that* licence rather than the repository badge. Voices trained on
LibriTTS or VCTK are the usual candidates because those corpora are CC-BY, but
I have not verified a specific replacement and will not name one I have not
read. Whichever is chosen, the attribution it requires goes in the app and in
the repository.

This is cheap to fix now and expensive to fix after a download page exists.

## Blocker 2: we ship GPL-3.0 code, and that closes the Apple App Store

**Measured.** `app/src-tauri/speech/piper/` contains `espeak-ng`,
`libespeak-ng.so*` and `espeak-ng-data`. eSpeak NG is **GPL-3.0-or-later**
(confirmed on the upstream repository). Piper itself is MIT and ONNX Runtime is
MIT; espeak-ng is the copyleft one, and `libpiper_phonemize` links it.

Two separate consequences, and they are often confused.

**Our own licence is not forced to GPL.** Piper is spawned as a *separate
process* — `voicecast-engine/src/piper.rs` opens with "Driven as processes
rather than through a library", and `espeak.rs` does the same. Arm's-length
process invocation is the standard basis for treating two programs as
aggregated rather than combined. So MIT/Apache-2.0 for our code stands.

**But redistributing GPL-3.0 binaries carries GPL-3.0 obligations**, whatever
our own licence says. Today we ship those binaries with **no licence text and
no offer of source** — measured: there is no `LICENSE` or `COPYING` anywhere
under `app/src-tauri/speech/`. That is a straightforward compliance gap and it
exists right now, before any of this is published.

**And it closes the iOS App Store.** Apple's terms impose usage and
distribution restrictions that GPL forbids adding; this is why VLC was pulled
in 2011 after a copyright holder objected. Shipping a GPL-3.0 binary inside an
App Store build recreates that situation exactly.

**What to do — one change fixes both:** stop bundling the speech payload.
`cargo xtask piper` already downloads Piper and a voice on demand, and the
engine layer already tolerates an engine that is absent and appears later
(`Rediscovering`, decision 52). So the app can ship with no engine and fetch
one on first run, at the user's request.

Then:

- the distributed artefact contains no GPL code, and the App Store question
  disappears
- the voice-corpus problem disappears from *our* distribution as well, because
  we are no longer the distributor
- Android already uses the platform engine and is unaffected
- Linux and Windows keep Piper, fetched rather than bundled

The cost is a first-run download and an app that cannot speak until it
finishes. That is a real regression in first impressions and it is the honest
price. The alternative — native engines on macOS and Windows, which the
`SpeechEngine` trait already anticipates — is more work and better, and can
come later.

---

## The licence for our own code

**MIT OR Apache-2.0.** Recommended over the alternatives for reasons that
apply here specifically:

- It is what the Rust ecosystem does, so it composes with our dependencies
  without anyone thinking about it. **Measured: 331 of our 703 crates are
  exactly this, and 703 of 703 are permissive** — no GPL, AGPL or SSPL
  anywhere in the tree.
- Apache-2.0 carries an explicit patent grant, which MIT does not. Offering
  both lets a user who dislikes Apache's terms take MIT.
- It is App Store compatible, which GPL and AGPL are not. Since iOS is one of
  the five targets, choosing copyleft would be choosing to abandon it.

Not AGPL: it is aimed at network services, and this project has no server.
Not GPL: it would trade the App Store for a protection this project does not
need — nobody can take voicecast proprietary in a way that hurts us, because
the value is the network of your own devices, not the code.

**Action:** every one of our nine crates has no `license` field —
measured — so `cargo metadata` reports them as unlicensed. Add
`license = "MIT OR Apache-2.0"` to each, plus `LICENSE-MIT` and
`LICENSE-APACHE` at the root, and a short licence section in the README.

### Dependency licences, measured

| count | licence |
|---|---|
| 331 | MIT OR Apache-2.0 |
| 149 | MIT |
| 68 | Apache-2.0 OR MIT |
| 24 | MIT/Apache-2.0 |
| 21 | Zlib OR Apache-2.0 OR MIT |
| 18 | Unicode-3.0 |
| 10 | Unlicense OR MIT |
| 7 | Apache-2.0 |
| 6 | BSD-3-Clause |
| 6 | MPL-2.0 |

The six MPL-2.0 crates are `attohttpc`, `cssparser`, `cssparser-macros`,
`dtoa-short`, `option-ext` and `selectors`. MPL is file-level copyleft: using
them is fine and imposes nothing on our code; modifying *their* files would
require publishing those changes. We do not modify them.

`r-efi` lists LGPL-2.1 as one option of three and we can take MIT. Nothing in
the tree forces anything.

**Action:** add `cargo-deny` to the gates so a dependency that changes licence
fails a build rather than being noticed later. This is the same argument as
every other gate in this project: the check that matters is the one that runs.

---

## Publishing binaries

### Google Play

- Developer account: **25 USD once**.
- A **privacy policy URL is required**, whether or not anything is collected.
- The **Data safety** form must be completed and must be accurate. Ours is
  unusual and worth stating plainly: messages are sent peer-to-peer and never
  reach a server we run. The app does use the network, and the pkarr/DHT
  discovery mechanism publishes a public key. Say so.
- Target API level requirements move every year; a published app that falls
  behind gets delisted from search. That is a maintenance commitment, not a
  one-off.
- GPL is acceptable on Play, so blocker 2 is only about Apple — but the fix
  is the same and simpler than shipping two different bundles.

### Apple App Store

- 99 USD a year, already paid (#29).
- Same privacy requirements, expressed as privacy "nutrition labels".
- **No GPL.** See blocker 2.
- App Review will ask what the app is for. A peer-to-peer app with no account
  and no server is unusual enough to expect questions.

### Direct download from GitHub Releases

The simplest path and the one that works today. Signed and notarised on macOS
(#118, #120), signed on Android (#97). No review, no store policy, no annual
target-API treadmill.

**Recommendation: start here.** A store listing is a commitment to keep
meeting someone else's requirements, and there is nothing to gain from it
until people are actually asking for the app.

### Export controls

**Not an obstacle, and worth writing down so nobody worries about it twice.**
The project uses standard cryptography (QUIC/TLS and Ed25519 through iroh).
The BIS email notification that used to be required for publicly available
encryption source code was **removed in 2021** for software using standard
cryptography. Apple's submission form still asks about encryption; the answer
is that it qualifies for the public-availability exemption.

---

## The website

GitHub Pages is the right choice: static, free, HTTPS, and it lives beside the
code. Because it is static and has no accounts, no forms and no analytics by
default, **it needs no terms of service and no cookie banner.** Adding
analytics changes that answer, which is a reason not to add any.

A privacy policy is still needed — not for the site, but because the app
stores require a URL to one — and Pages is the obvious place to host it.

## Terms of use

**The software does not need one.** MIT and Apache-2.0 both carry the warranty
disclaimer and limitation of liability that a terms-of-use document would
otherwise supply. Adding a separate ToS on top of an open-source licence
usually creates ambiguity rather than protection.

What is genuinely worth writing, and is not a legal document:

- **A security policy** (`SECURITY.md`) saying how to report a vulnerability.
  A peer-to-peer app that holds signing keys and device identities will get
  reports; better to say where they go.
- **A contribution policy.** Recommend **DCO** (a `Signed-off-by` line) over a
  CLA: it is a statement of origin rather than a transfer of rights, it needs
  no paperwork, and asking hobby contributors to sign a CLA for a project with
  no commercial arm is friction with nothing behind it.
- **A trademark note.** The *name* is not covered by the code licence. If you
  want to stop a fork shipping a confusingly-similar "Voicecast" in the app
  stores, that is trademark, not copyright, and it is a separate decision.

## Things only Patrick can settle

1. **The name.** I found no obvious conflicting product, but that is a web
   search and not a clearance search. Before a store listing, check the USPTO
   database and both app stores directly. "Voicecast" is descriptive, which
   cuts both ways: harder to protect, less likely to infringe.
2. **Copyright holder.** "Patrick Hogg" or an entity. It goes in every licence
   header and changing it later means asking every contributor.
3. **Whether a store listing is wanted at all**, given the maintenance it
   commits you to.
4. **Whether to keep a bundled engine for the direct download**, where GPL is
   allowed if we comply properly, and drop it only for store builds. Two
   artefacts is more to get wrong; one is simpler and slower on first run.

## Order of work

1. Change the default voice. Blocks everything and is small.
2. Stop bundling the speech payload; fetch on first run.
3. Add licence files, crate `license` fields, README section, `SECURITY.md`.
4. Add `cargo-deny` to `cargo xtask check`.
5. Privacy policy and the Pages site.
6. Store listings, if wanted, last.

Steps 1 to 4 are mine and need nothing from anyone. Step 5 needs the decisions
above.

**None of this is legal advice.** It is a careful reading of licence text by
someone who is not a lawyer, and the two blockers are the kind of thing worth
thirty minutes of one before a public release.
