# Contributing

Bug reports and pull requests are welcome. A few things about this repository
are unusual enough to be worth knowing before you spend time on a change.

## The one rule that matters most

**If it doesn't compile for all five targets, it doesn't merge** — linux,
android, macos, ios, windows. Local checks say nothing about four of them;
Windows was broken for fifteen commits while every local gate reported green.

Before anything else:

```bash
cargo run -p xtask -- check
```

That runs, in order: merge-conflict markers, workflow formatting, `fmt`,
`clippy`, the test suite, and a portability gate that fails if a platform
conditional appears in a crate that has to stay portable.

## Open pull requests as drafts

The five-target build skips drafts; the Linux job runs on every push either
way. Mark a pull request ready when you want the expensive verdict. Marking
ready *immediately* after a push produces no run at all — the push's own run
already evaluated it as a draft — so leave half a minute between them.

## Documentation is part of the change

`docs/decisions.md` is numbered and append-only. A decision records what was
chosen, **why**, and what it cost. If a change alters behaviour the docs
describe, the docs move with it — the README and `docs/` are not a separate
task to be done later.

This is the part people skip and the part that makes the rest of the
repository legible. It is also why the file is worth reading before proposing
anything structural: a surprising amount is surprising on purpose.

## Errors are written for whoever reads them

The premise of the project is that an *agent* drives the command line. A
rejected message shows the offending span and a rewrite that can be sent
verbatim; a device that will not speak says which policy stopped it. An error
that says only "no" makes the caller guess, and a caller that guesses is a
worse bug than the one you fixed.

## Say what you actually verified

"Compiles on five targets" is a weaker claim than it reads as: `cargo test`
runs on Linux only, and one of the five links no networking code at all. The
repository distinguishes **compiled**, **linked** and **launched**, and asks
you to say which you mean. "Tested on a phone" meant a debug build nobody
would ever download for several months, and the first release APK to reach a
real device died on launch.

If something is unmeasured, saying so is a complete and acceptable answer.

## Things that are deliberate

- `clispeak-cli` depends on two crates and duplicates a handful of constants
  by hand. That keeps its startup at ~3ms, which is the whole premise of the
  thin-client design.
- `app/src/styles.css` is generated and not committed.
- The speech payload is not shipped on every platform, and `docs/licensing.md`
  explains which and why.
