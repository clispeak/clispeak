# Releasing

**Most of this is built.** `.github/workflows/release.yml` packages four
platforms on a `v*` tag and leaves the result as a draft. What is left is not
engineering: licensing (#24), how a private repository serves public downloads
(#23), a download page (#25), and the two signing credentials (#29, #31).
The macOS half of that is written down in `docs/signing.md`: what the
certificate fixes, how to make one, and why the self-signed one belongs on a
development machine and not in a repository secret.

This file was written before any of it existed and said so for months after it
did, which is how an agent came to start building the release chain a second
time. It now describes what runs.

## What we are aiming at

Free binaries for Linux, Android, macOS and Windows, built automatically, with
a website offering the latest of each. iOS is excluded: it has never been run,
and Apple's distribution route is not a download link.

**The source stays closed for now.** That is compatible with free binaries, but
it is not free of consequences — see below, twice.

## What 1.0 means

A thing someone can download and use on Linux, Android, macOS and Windows.
Not iOS: it has never been run, and Apple's route is not a download link.

The milestone holds two kinds of thing, and nothing else:

**The release chain** — everything between a tag and a binary in someone's
hands. Building on tags, publishing from a private repository, the licensing
that has to be settled first, the download page, and the three signing
questions: a macOS identity, an Android release key, and Microsoft's
redistribution terms.

**First run** — what someone hits in the first five minutes. A node that
blocks silently on a keychain prompt while the CLI blames the wrong thing. A
Mac with no working Piper being told to install what it already has. Two
commands the docs promise that do not exist. Adding a device.

Deliberately outside it: iOS, which has no hardware; branch protection, which
is a repository setting rather than work; and the architecture matrix, which
has a pull request open already.

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

**What it actually costs.** This repository is private, so Actions minutes come
out of an allowance. From the usage page for 2 September 2026 — one day, during
which three agents pushed to `main` repeatedly:

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

## Publishing from a closed repository

**A release on a private repository is not a public download.** Its assets
need the same authentication the source does, so a website cannot simply link
to them. Three ways round it, none free of trade-offs:

1. **A separate public repository holding only releases.** No source, no
   history — just tags and attached binaries, published to by a workflow from
   the private one. Keeps everything inside GitHub and costs nothing.
2. **Object storage** — R2, S3 or similar — uploaded by the release workflow.
   More control, a bill, and one more credential in CI.
3. **A server we run.** Contradicts the premise of the project, which is that
   there is no server to run.

Option 1 is the obvious starting point. It should be a deliberate choice
rather than a default, because a public releases repository is a public
statement that this project exists.

## Licensing: settled, with two things to change first

**Settled on 3 September 2026.** The project is **MIT OR Apache-2.0** and goes
open source; binaries are published from GitHub Releases first, with app
stores later if at all, and the site is GitHub Pages. Decision 74, with the
full working in `docs/licensing.md`.

**Two things still have to change before a public download exists**, and both
are about other people's software rather than ours:

- ~~The default voice must change.~~ **Done** (decision 81). The default is
  `en_US-ljspeech-medium`, trained on the LJ Speech corpus, which is public
  domain with no restrictions on use and no attribution required.
- **The speech payload should stop being bundled** and be fetched on first run
  instead. That removes the GPL-3.0 espeak-ng from the artefact, which is what
  otherwise closes the iOS App Store, and removes the voice from our
  distribution as well.

Nothing has been published yet — no releases, no tags, and the voice is not in
git — so this is a problem to avoid rather than one to unwind.

**More pressing: we redistribute other people's software inside our packages.**
The Flatpak, the macOS bundle and the planned Windows installer all carry
Piper, which is not one program. The archive we ship contains, at least:

- `piper` itself
- `libonnxruntime` — Microsoft's ONNX Runtime
- `libpiper_phonemize`
- **`espeak-ng`, its shared library and its data** — espeak-ng is
  GPL-3.0-or-later
- a voice model, whose own terms are separate again
- on Windows, Microsoft's Visual C++ runtime (issue #20)

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
