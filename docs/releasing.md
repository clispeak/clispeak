# Releasing

**This is the chain that runs.** A `v*` tag builds four platforms, stops twice
for a person to approve the signing keys, and leaves a draft. A person writes
the notes and publishes it. Seven releases have gone out this way.

**This file has now been stale twice, and the second time it said it was
not.** It was written before the release chain existed and went on describing
a future for months after it arrived — which is how an agent came to start
building the chain a second time. It was corrected, gained the sentence *"it
now describes what runs"*, and then drifted again while carrying that
sentence: it still said the repository was private, that nothing had been
published, and that a download page was outstanding. All three were false, in
the document that describes publishing, on the day of the seventh release.

The lesson is not "keep it current". It is that **a document asserting its own
freshness is the one nobody re-reads**, because the assertion answers the
question a reader would otherwise ask.

## What we are aiming at

Free binaries for Linux, Android, macOS and Windows, built automatically, with
a website offering the latest of each. All of that exists.

iOS is excluded from releases because Apple's distribution route is not a
download link — TestFlight or the App Store, both of which need an account and
a review. It builds and it is tested; it is not shipped (decision 104).

**The source is public and the licence is MIT OR Apache-2.0** (decision 74).
This section used to say "the source stays closed for now", and a great deal
below it was reasoning about how a private repository could serve public
downloads. That question is gone, not answered.

## What a release does

```
git tag -a vX.Y.Z -m "X.Y.Z"   and push it
   -> four platform jobs build
   -> macos-signing   waits for a named reviewer
   -> android-signing waits for a named reviewer
   -> a draft release with four artefacts and SHA256SUMS
   -> a person writes the notes and publishes
```

**Both approvals are the point, not friction.** Each gate stands in front of a
signing key, and it exists so a person watches the key get used. An agent
holding the ability to approve is the same as having no gate.

**Verify the artefacts before handing anyone a link.** What a file contains is
checkable without the platform it runs on: the checksums, the APK signing
block and its `versionName`, the `.app`'s `Info.plist` version and
`_CodeSignature`, and the Windows import table. That last one is not
theoretical — `clispeak.exe` once shipped importing `VCRUNTIME140.dll` and
died before `main` on a clean machine, and the evidence sat in a downloadable
artefact for two days while nobody opened it (#201).

**Publish as a normal release, never a pre-release.** GitHub resolves
`/releases/latest/download/...` to the newest release that is neither a draft
nor a pre-release, and those URLs are what clispeak.com links to. A beta
marked pre-release is a site with four dead buttons and nothing saying why.

## Builds run on version tags, not on every push

CI runs five jobs on every push to `main` and every pull request: compile
checks for the five targets, one of which also runs fmt, clippy, the
portability gate and the tests. That is the portability rule doing its job and
it should stay.

**Packaging is different and should not be on that trigger.** Building a
Flatpak, a signed `.app`, an APK and a Windows installer is minutes of macOS
and Windows runner time, and neither is needed to know whether a commit
compiles. So:

| Trigger | Runs |
|---|---|
| push to `main`, pull request | compile for five targets, fmt, clippy, portability, tests |
| tag `v*` | package every platform, checksum the artefacts, attach them to a draft |

**A tag does not re-run the compile checks.** `ci.yml` triggers on pushes and
pull requests, not on tags, so a tag packages whatever `main` already proved.
That is deliberate and worth knowing: it means a tag pushed to a commit CI
never saw is packaged without ever being checked.

**What it actually costs.** From the usage page for 2 September 2026 — one
day, during which three agents pushed to `main` repeatedly:

| | Minutes | Rate | Gross |
|---|---|---|---|
| Linux | 501 | $0.006 | $3.01 |
| Windows | 214 | $0.010 | $2.14 |
| **macOS** | **171** | **$0.062** | **$10.60** |

Only $0.16 was billed — storage. The minutes were inside the allowance. But
the shape is the point: **macOS was two thirds of the gross on a fifth of the
minutes.**

In the units the allowance is actually consumed in, that day cost about
**2,639 Linux-equivalent minutes** — a whole month of a 3,000-minute plan, in
roughly one day.

Two things follow, and only one of them is about the matrix.

The Apple targets used to be two jobs on `macos-latest`, each paying its own
checkout, toolchain install and cache restore on the most expensive runner
GitHub sells. They are one job now, building both targets. Same coverage,
half the macOS jobs.

**The larger driver is push frequency, not the matrix.** Forty pushes to `main`
in a day is forty runs. Work that goes through a branch and a pull request
costs about the same per run, but there are far fewer of them. That is a
working-habit lever rather than a configuration one, and it is the bigger of
the two.

What should *not* be traded away is the five-target rule itself. It is why the
Windows break was caught after fifteen commits rather than at some point after
that.

## How a public download actually works

The repository is public, so a GitHub release asset is a public URL and the
site links straight to it:

```
https://github.com/clispeak/clispeak/releases/latest/download/<stable name>
```

The names are stable on purpose — `clispeak-macos.dmg`, not
`clispeak_0.9.6_aarch64.dmg` — so the page never has to know a version number.
It reads the current one from the API to display it, and the buttons work
regardless.

This section used to weigh three ways for a *private* repository to serve
public downloads: a separate releases-only repository, object storage, or a
server. None was taken. The repository went public and the question stopped
existing, which is worth recording because the three options were carefully
argued and are now noise.

## Licensing: settled

**Settled on 3 September 2026.** The project is **MIT OR Apache-2.0** and goes
open source; binaries are published from GitHub Releases first, with app
stores later if at all, and the site is GitHub Pages. Decision 74, with the
full working in `docs/licensing.md`.

**Two things had to change before a public download existed**, and both were
about other people's software rather than ours. Both are now closed:

- ~~The default voice must change.~~ **Done** (decision 81). The default is
  `en_US-ljspeech-medium`, trained on the LJ Speech corpus, which is public
  domain with no restrictions on use and no attribution required.
- ~~The speech payload should stop being bundled.~~ **Moot for four of five
  platforms** (decisions 96 and 102): each speaks through the system
  synthesiser and carries nothing. The Flatpak still bundles Piper, and
  fetching on first run remains the option if an iOS App Store submission ever
  needs it.

**Both are now settled.** The voice changed before the first release, and the
payload question resolved itself by platform rather than by unbundling: macOS,
Windows, iOS and Android all speak through their own synthesiser and carry no
payload at all, so the Flatpak is the only artefact that ships any of it. What
follows is Linux's alone.

**More pressing: we redistribute other people's software inside our packages.**
**One package, now: the Flatpak.** macOS and Windows both moved to the
platform synthesiser (decisions 96 and 102), and the bundler stages the
payload only where Piper is the engine — so the macOS `.dmg` and the Windows
installer carry none of what follows, and the licensing question below is
Linux's alone. That was not a size optimisation. Shipping espeak-ng is a
redistribution with obligations, and doing it for a platform that never calls
it is all of the cost and none of the use.

Piper is not one program. The archive the Flatpak ships contains, at least:

- `piper` itself
- `libonnxruntime` — Microsoft's ONNX Runtime
- `libpiper_phonemize`
- **`espeak-ng`, its shared library and its data** — espeak-ng is
  GPL-3.0-or-later
- a voice model, whose own terms are separate again

Issue #20 was the Windows half of this — Microsoft's Visual C++ runtime, which
Piper links against and a clean Windows does not have. It is gone with Piper
rather than solved.

Two things about that are worth stating plainly rather than assuming.

**The archive as downloaded carries no licence text at all.** There is no
`COPYING` or `LICENSE` anywhere in the extracted tree. Whatever the
obligations turn out to be, redistributing GPL software with no licence file
is unlikely to meet them.

**The terms have now been read**, and this paragraph used to say nobody had.
espeak-ng is GPL-3.0-or-later; Piper, `libpiper_phonemize` and ONNX Runtime are
MIT; the voice model's corpus is research-use-only and bars redistribution,
which is the one that changes what we ship. Microsoft's redistributable clause
is still unread and is #20's, not this page's.

Writing that down turned out to matter more than the licence names did: the
espeak-ng name was already here, and the consequences — the App Store, the
voice — only appeared once somebody opened the actual terms. `docs/licensing.md`
has them.

None of this is an argument against distributing. Bundling a GPL program as a
separate executable that we invoke, unmodified, is ordinary and common. It
simply has requirements, and requirements that nobody has looked up are
requirements that will not be met.
