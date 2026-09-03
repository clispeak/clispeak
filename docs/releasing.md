# Releasing

**Nothing here is built yet.** This is the plan, written down so the decisions
in it are visible before anyone implements them. Issues #22 to #25 track the
work.

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

CI today runs six jobs on every push to `main` and every pull request:
compile checks for the five targets plus a Linux gate job. That is the
portability rule doing its job and it should stay.

**Packaging is different and should not be on that trigger.** Building a
Flatpak, a signed `.app`, an APK and a Windows installer is minutes of macOS
and Windows runner time, and neither is needed to know whether a commit
compiles. So:

| Trigger | Runs |
|---|---|
| push to `main`, pull request | compile for five targets, fmt, clippy, portability, tests |
| tag `v*` | the above, then package every platform and attach the artefacts |

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

## Licensing has to be settled first

**This blocks the website, not the builds.** Distributing binaries publicly is
the point at which licence terms stop being a formality.

The project's own licence is undecided — that has been an open question since
the first day and is recorded in `decisions.md`.

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

**Nobody has read the terms.** Not espeak-ng's, not the voice model's, not
Microsoft's redistributable clause. That is not a lawyer's job to start with —
it is a matter of finding the files, reading them, and writing down what each
one requires. Until that is done, "free binaries for everyone" is a plan
rather than a decision.

None of this is an argument against distributing. Bundling a GPL program as a
separate executable that we invoke, unmodified, is ordinary and common. It
simply has requirements, and requirements that nobody has looked up are
requirements that will not be met.
