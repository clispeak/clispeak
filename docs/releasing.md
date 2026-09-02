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

**A caution about cost, which we have not measured.** This repository is
private, so Actions minutes come out of an allowance rather than being free.
macOS runners bill at **ten times** the rate of Linux and Windows at **two
times**, and the current matrix runs both on every push. The GitHub timing API
reported zero billable minutes for our runs when asked, which is not a figure
worth trusting either way — the usage page in account settings is the thing to
read. Worth reading it before deciding how much of the current matrix to keep
on every push.

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
