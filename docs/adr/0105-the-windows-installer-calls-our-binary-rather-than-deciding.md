# 105. The Windows installer calls our binary rather than deciding anything

**Status:** Accepted.

The bar Patrick set for Windows is double-click and the machine speaks: no
terminal, for anything. Moving to SAPI (decision 96) deleted most of what that
required — no Piper, no voice model, no Visual C++ redistributable — and left
one job. `clispeak` has to be a command, which on Windows means editing a PATH
that lives in the registry under `HKCU\Environment`.

**An installer is the normal place to do that, and we do it in Rust instead.**
The hook file is six lines and contains no logic: it runs the app binary with
`--add-to-path`, and `--remove-from-path` before uninstalling.

**Why, and this is not a style preference.** NSIS reads a registry string into
a fixed buffer — 1024 characters in the standard build. A developer's PATH is
routinely longer than that. The read does not fail. It **truncates**, and the
installer then writes the truncated value back, destroying the rest of the
PATH on a machine we do not own, silently, while the install reports success.
Every entry in `CLAUDE.md`'s table is that shape, and this one arrives with a
person's working environment attached.

There is a second reason, which we would have got wrong without looking. A
registry value has a *type*, and `HKCU\Environment\PATH` is normally
`REG_EXPAND_SZ` so that `%USERPROFILE%\bin` expands. NSIS cannot read a value's
type. Writing it back as `REG_SZ` — what you get if you do not think about it —
turns every `%VAR%` entry on the machine into a literal string naming no
directory, and nothing reports an error; unrelated tools just stop being found.
The registry API reads the type and writes it back unchanged.

**What this buys is testability, which Windows has none of.** `cargo test` runs
on `ubuntu-latest` and nowhere else, so a test behind `cfg(windows)` never
executes. The part that decides what the new PATH should be is therefore a
pure function compiled on every platform — the same shape as `sapi_rate` in
decision 97 — and nine tests run against it on Linux: adding twice, a
different case, a trailing backslash, quotes, a trailing separator that must
not become an empty entry (Windows reads one as the current directory), and an
uninstall that preserves everything it did not write, including entries it
would call malformed.

**What it costs.** Three things.

The hook names the app binary through Tauri's own `${MAINBINARYNAME}` define
rather than writing it out, because `nsExec` against a path that does not
exist returns an error nobody reads and the install finishes looking exactly
like a success. That was nearly a hardcoded `clispeak-app.exe` — correct
today, and a silent failure the day `mainBinaryName` is set.

The release build is a `windows_subsystem = "windows"` binary, so it has no
console and everything it prints into the installer log goes nowhere. The exit
status still arrives; the explanation does not.

And the app repeats the add on every launch. That is deliberate — it is a
registry read, it is idempotent by construction, and it is the only thing that
repairs an install whose hook never ran, which an antivirus blocking a child
process is enough to cause. What that does not repair is the removal on
uninstall, which has no second chance.

**None of this has been run.** Written, unit-tested on Linux, compiled for
`x86_64-pc-windows-msvc`. Three claims that stop short of the one that matters,
which is the distinction `CLAUDE.md` opens with.
