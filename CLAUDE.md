# Working on clispeak

A command-line tool that speaks text aloud on your devices, peer to peer, with
no server. Each install is a node — sender and receiver both. The CLI is a
thin client that hands text to the local node and exits.

**The point of the project is that an *agent* drives the CLI.** That shapes
almost every decision: distinct exit codes, errors that carry a fix, and
receivers that report a reason rather than swallowing a failure.

## The one rule that matters most

> **If it doesn't compile for all five targets, it doesn't merge.**
> linux · android · macos · ios · windows

**Local gates are not that gate.** `cargo test`, `cargo clippy` and
`cargo fmt` only exercise the machine they run on. Passing them says nothing
about four of the five targets. Windows was broken for fifteen commits while
local checks reported green every time — see issue #5. Check the CI run, not
your terminal, before claiming a change is safe.

`cargo run -p xtask -- portability` fails if `clispeak-proto`,
`clispeak-text` or `clispeak-core` grows a platform conditional — any of
`target_os`, `target_family`, `target_arch`, `target_env`,
`target_pointer_width`, `unix` or `windows`, however it is wrapped. Platform
divergence belongs in `clispeak-engine` or the Tauri shell. Anything that
cannot be expressed portably gets a trait, not a conditional in the middle of
business logic.

For a long time it checked only the first two of those, so `cfg(unix)` sat in
`clispeak-core` while the gate printed "3 crates clean" (#88). Where there is
genuinely no portable spelling — setting a file mode is the only case so far —
say so on the line above and the gate counts it instead of failing:

```rust
// portability-exception: a file mode has no portable spelling
#[cfg(unix)]
```

The reason is required, and the count is printed, because an exemption nobody
reads is the same silence the gate exists to break.

**The divergence that has actually bitten was undeclared.** `portability`
finds platform code that *announces itself* with a `cfg`. Every one of these
compiled on all five targets and passed every gate:

| | |
|---|---|
| `/etc/hostname` | a Linux file, so macOS and Windows called every device "this device" (#38) |
| `Command::new("npx")` | resolves `npx.cmd` on no Windows box |
| `target/release/clispeak` | has no `.exe` |
| `sun_path` | a socket *name* over ~83 bytes refuses to bind on macOS, and the platform prepends a prefix you did not write |
| `isMinifyEnabled` | true only for release, so R8 deleted the Kotlin the engine calls (#41) |
| `file("keystore.properties")` | resolves against the Gradle *module*, so a copy one directory up is read as absent and the APK ships unsigned (#31) |
| `base64 -d` | decodes on Linux; macOS spells the short flag `-D`, so the same line writes an empty file on the runner that signs the app |

The socket row says *name*, not path, on purpose: it is a namespaced name, the
platform decides where it lands, and on Linux there is no file anywhere (#43,
decision 35). The budget is measured rather than documented — 83 bytes bound
and 84 refused on a Mac, plus a `/tmp/` prefix confirmed with `lsof`, against a
documented `sun_path` of 104. Measure against 83; it is the number that decides
whether a node starts.

**The 16 bytes that decision 35 called unexplained are not headroom, and the
guess was the wrong shape.** Measured on a Mac on 3 September 2026 by binding
at increasing lengths until refusal: a *namespaced name* clears at 83 and
refuses at 84, and a *filesystem path* clears at **104** and refuses at 105 —
the whole of `sun_path`. So there are two budgets, not one budget with a
mystery in it. 83 is what a name costs once the library has reserved room for
the prefix it adds; 104 is what a path costs, because nothing is added to it.
Raw `AF_UNIX` through Python gives 103 and 104, one byte tighter, which is the
terminator.

The lesson is not the numbers. It is that "the missing 16 bytes are
unexplained, probably headroom" sat in this file for weeks reading like a
fact with a hedge on it, and the hedge was doing the work — nobody had asked
the second question, so nobody had noticed it was a different question.

A file path, a binary name, a separator, a length limit, a build flag, a
relative path resolved against a directory you were not thinking about. None
is platform-*shaped*; a `cfg` on any of them would have been louder and would
have been caught. When you write one, ask what the other four targets have
there — and note the last two rows are not platform differences at all. One is
a difference between two builds for the same platform, and it fooled us the
same way because everything anyone had run on a real phone was a debug build.
The other is a difference between two directories, and `signingReport` reported
it as `Config: null` rather than "no such file" — which is what the whole table
is about. A thing that is looked for in the wrong place does not say so; it
says nothing, and the build carries on and produces something subtly wrong.

**Compiling on five is a weaker claim than it reads as.** `cargo test`
runs once, on `ubuntu-latest`. The five matrix jobs only build. So a test
asserting cross-platform behaviour is executed on Linux and nowhere else, and
"CI green" means *compiles* on five, *tested* on one. That is a deliberate
trade — macOS runners bill at ten times Linux — but say "build-verified" when
that is what happened.

**And it is weaker again than that, because compiling is not linking.**
An `rlib` never resolves its external symbols; only linking an executable
does. The matrix links exactly one binary for iOS — `clispeak` — and that
crate depends on `clispeak-proto` and `clispeak-text` and nothing else, by
design, so it reaches no network code at all. `clispeak-app`, which pulls
`clispeak-core` and therefore iroh, is **excluded** from the second Apple
target.

So an iOS build linked 28 undefined `_SCDynamicStore*` symbols the first time
anyone tried it on a real machine — iroh's transitive dependencies read the
system network configuration, and the Xcode project Tauri generates does not
link `SystemConfiguration`. Every green iOS check had said nothing about it.
The same run *did* catch a missing blake3 NEON symbol, which is the tell: the
CLI links, so what the CLI touches is checked, and what only the app touches
is not.

Then it launched and panicked before its first frame, on `No rustls crypto
provider is configured` — every other target resolves one through feature
unification and iOS does not.

**Three claims, and they are not the same claim:**

| | |
|---|---|
| **compiled on five** | what a green matrix means |
| **linked on four** | iOS links only a binary with no networking in it |
| **launched on three** | Linux, Android and macOS have been started by a person |

Say which one you mean. "It builds on iOS" was true for months and cost three
bugs that no gate could have caught, because nobody had run the target the
rule counts.

## Where things live

| Crate | Holds | Must stay portable |
|---|---|---|
| `clispeak-proto` | Wire and IPC types. Pure data, CBOR | yes |
| `clispeak-text` | Validation, chunking. Pure functions | yes |
| `clispeak-core` | Transport, roster, spaces, queue, policy, history | **yes** |
| `clispeak-engine` | `SpeechEngine` trait and per-platform impls | no |
| `clispeak-cli` | The `clispeak` binary. Depends only on proto + text | — |
| `clispeak-daemon` | Headless node, for machines with no desktop | — |
| `clispeak-keystore` | Platform keyring | — |
| `app/src-tauri` | The app: owns a `Node`, adds a tray and a window | no |
| `app/src` | Frontend. Plain HTML and JS, Tailwind build step | — |

`clispeak-cli` depending on only two crates is deliberate — it keeps startup
at ~3ms, which is the whole premise of the thin-client design. Do not reach
for `clispeak-core` from it; duplicate the handful of bytes instead, as
`frame.rs` and the socket name already do, and keep them in step by hand.

## Checks

```bash
cargo run -p xtask -- check     # conflicts, workflows, fmt, clippy, tests, portability
cd app && npx tailwindcss -i src/input.css -o src/styles.css --minify
```

**The first two are not cargo, and they run first on purpose.** `conflicts`
fails on an unresolved merge marker in any tracked file, and `workflows` fails
on a multi-line `run:` step that is not a literal `|` block. Both exist
because a gate ran, passed, and said nothing about the thing it protects:
`check` used to report `all gates passed` on a tree holding a live
`<<<<<<< HEAD`, since the numbering check reads headings and nothing else
opens a `.md` file (#103). And YAML folds both the plain form and `>` into one
line, so a shell continuation survives as a literal backslash and the next
path becomes an argument with a leading space — which left a keystore's
passwords on disk while `rm -f` exited 0 (#102). Markers first, because every
other gate's verdict is meaningless on a half-merged tree and would be
*reported* anyway.

**Run the one command, not the four it wraps.** Assembling the chain by hand
has failed three times in one week, each differently and each silently: a
`&&` that short-circuited so an edit never ran while the previous output still
printed; a `;` where `&&` was meant, so a failing gate did not stop a push; and
a chain that simply omitted clippy, which is how a lint reached `main` (#91).
The gates were fine every time. The wiring around them was not.

Where it matters, the individual commands are still what CI runs:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p xtask -- portability
```

**Open a pull request as a draft, and mark it ready when you want the
verdict.** The five-target build skips draft pull requests; the Linux job —
fmt, clippy, portability, the whole test suite — runs on every push either
way. So push as often as you like and pay for the expensive answer once, at
the point the rule actually applies. The rule has not moved: nothing merges
without all five green, and marking ready is what asks.

The number that forced this: one day cost 3,649 billed Linux-equivalent
minutes, more than a month's included allowance, and macOS alone was 59% of
it while doing under two minutes of real work per job. A macOS minute bills at
ten times a Linux one. Cutting the number of runs is the only lever that
reaches that; making the jobs faster is not (#101).

**Do not mark ready in the same breath as a push.** Marking ready about a
second after a push produced no run at all — the push's own run had already
evaluated the pull request as a draft and skipped the matrix, and the
`ready_for_review` event never became a run. Toggling draft and back with
half a minute between produced one immediately, so the trigger is fine and
the race is real. It fails the way everything in this file fails: the pull
request reads as ready, the checks read as passing, and four of the five
never ran. **Before merging, read the check rollup and count five named targets.**

Count the **distinct names**, and check each one you need is there:

```
x86_64-unknown-linux-gnu   aarch64-linux-android
aarch64-apple-darwin       x86_64-pc-windows-msvc
fmt · clippy · portability · test
```

Not the number of green rows. `matrix.target: SKIPPED` is the draft run's
result and it **stays in the rollup forever**, beside the four real names on a
fully verified pull request — and so does the draft run's own Linux job, so
the rollup routinely shows seven rows for five jobs, two of them the same
name. Counting rows to five is satisfied by the duplicate plus three targets
while the fourth is still running. That is how it read green here with Android
in flight, on a rule written the same afternoon by the person it fooled.

It is a history, not a status. "There is no SKIPPED row" would reject every
correctly-verified pull request from here on. "Five green rows" passes an
unfinished one. Only the set of names answers the question.

**And where one name has two conclusions, take the latest.** That is not a
rare case, it is what `cancel-in-progress` produces on every pull request that
was ever a draft: marking one ready supersedes the draft run, so the gate job
appears as `CANCELLED` from the run that was killed and `SUCCESS` from the one
that replaced it. Requiring *every* conclusion for a name to be green rejects a
correctly verified pull request. Accepting *any* green conclusion is worse — a
stale pass then masks a fresh failure, which is the hole counting rows already
has. The last entry per name is the status, for the same reason the SKIPPED row
is not one: it is a history.

**And before any of that, check the results are for the commit you are
merging.** `statusCheckRollup` lags `headRefOid`. After a push the pull
request reports the new head while the rollup is still serving the previous
head's results, so a rollup that is green, complete and consistent with your
local sha can be describing the commit before it:

```
03:25:51  3875aae  in_progress    <- the actual head
03:15:59  f7beddb  success        <- what the rollup was reporting
```

Every other guard here passes that. Matching the sha is not enough, because
the sha matches. Read the Actions API, match runs by `head_sha`, and refuse
while any run for *that* sha is incomplete.

So the rule is four parts, each learned separately and all four in one day:

1. **runs matched by `head_sha`** — the rollup may be reporting the previous
   commit
2. **mergeable**, or no runs are produced at all and zero checks looks like
   early rather than empty
3. **five distinct names**, not five green rows
4. **the latest conclusion per name**, since a superseded draft run leaves
   `CANCELLED` beside `SUCCESS`

The first is listed first because it defeats the other three silently, while
they at least fail visibly.

**What all four have in common is the thing this file is about.** Each is a
tool answering honestly about something *adjacent* to the question asked.
Mergeable is not checks-passing. A row is not a name. A history is not a
status. A rollup keyed on a pull request is not a run keyed on a commit. None
of them lies; each answers a question next to the one you meant, and the
answer looks exactly like the answer you wanted.

**And before counting, check the pull request is mergeable — a conflicting
one produces nothing to count.** GitHub runs a `pull_request` workflow against
the *merge* of head into base, and when that merge cannot be computed it
creates no run rather than failing one. So a conflicting pull request sits
with **zero** checks, which is indistinguishable from one whose checks have
not started, and the counting rule above is blind to it: it catches a partial
run and cannot tell an empty one from an early one.

Found on #136, which sat with no checks while four separate trigger events
were fired at it — `opened`, `reopened`, `ready_for_review`, and a
`synchronize` from an empty commit — with Actions demonstrably alive on other
branches the whole time. `gh pr view --json mergeable` said `CONFLICTING`. A
rebase produced all five checks within twenty seconds.

Four attempts at the trigger before anyone asked whether the thing could
merge. That is the cost of an absence that reads as a state: every other
failure in this file *says* something wrong, and this one says nothing, which
the eye fills in as "not yet".

`app/src/styles.css` is generated and **not** committed. Tauri's
`beforeBuildCommand` rebuilds it, so `tauri build` and `tauri android build`
are covered — but a plain `cargo build -p clispeak-app`, which is how the
Flatpak is packaged, is not. Add a class in `index.html`, package it that way,
and the class is inert with nothing to say so.

**On a memory-constrained machine**, wrap cargo so an OOM kills the build and
not your session: `systemd-run --user --scope -p MemoryMax=3G -p
MemorySwapMax=0 cargo …`. `.cargo/config.toml` already limits jobs and
debuginfo for the same reason.

## Running it

```bash
cargo run -p clispeak-daemon          # a node
cargo run -p clispeak-cli -- status
```

Two nodes on one machine: override `CLISPEAK_SOCKET` and
`CLISPEAK_CONFIG_DIR`. Both are read by the CLI *and* the node, and
forgetting the socket override makes every command talk to the first node,
which looks like two unrelated bugs.

**`CLISPEAK_SOCKET` is a name, not a path, and it must be short** — under
about 83 bytes on macOS, where the platform then prepends `/tmp/`. Point it at
a scratch directory and the node refuses to bind for a reason that names the
limit and not the string. Scratchpad paths here are around 100 bytes on their
own, so this is the default way to hit it, not an edge case. Use
`CLISPEAK_SOCKET=vc-2.sock`, and put only `CLISPEAK_CONFIG_DIR` somewhere
deep.

Platform packages: `packaging/flatpak/` for Linux, `cargo xtask bundle` for a
macOS `.app`, and `npx @tauri-apps/cli android build --apk` for Android.
`cargo xtask piper` puts Piper and a voice where the engine looks.

## Conventions

**No session links in commit messages, pull requests, issues or comments.**
Not `Claude-Session:`, not a `claude.ai/code/session_…` URL, not in a trailer
and not in prose. `Co-Authored-By` is attribution and stays; a session link is
not attribution, it is a pointer to a private conversation that nobody reading
this repository can open.

235 commits accumulated one before anyone looked, and the repository is going
public — at which point they are permanent, because rewriting history after a
fork exists is not a thing you get to do. They were checked and do not resolve
without authentication, so this is tidiness rather than a leak. **The reason
it is a rule is that it was not one**: nothing decided to add them, and by the
time anybody asked, the only fix left was rewriting every commit in the
project.

If you are a tool that appends one by default, the repository's instruction
wins over the default.

**A limitation is a debt in three places.** When something cannot yet be done,
it gets written into the README and said in the interface as well as tracked as
an issue. Whoever removes the limitation owes all three in the same change —
otherwise the code gains a capability while two documents go on denying it. That
has already happened once here, one commit apart, and was caught by luck rather
than process.

**Delete the branch when the pull request merges.** `gh pr merge
--delete-branch` does it; ten merged branches had accumulated before anyone
looked, every one of them fully contained in `main`. They cost nothing and
they hide the ones that matter, which is the whole reason to keep the list
short.

**The project is MIT OR Apache-2.0 (decision 74). The speech payload is
not.** Piper, its phonemiser and ONNX Runtime are MIT; espeak-ng is
GPL-3.0-or-later; and a voice model carries the terms of the corpus it was
trained on, which are frequently not redistributable — the current default's
are not. `cargo xtask piper` fetching that onto your own machine is fine.
Putting it in something we hand to somebody else is a different question, and
`docs/licensing.md` is the answer to it.

**Docs are part of the change.** `docs/decisions.md` is numbered and
append-only: a decision records what was chosen, *why*, and what it costs.
`docs/build-plan.md` tracks milestones. If a change alters behaviour the docs
describe, the docs move with it.

**Errors are written for whoever reads them.** A rejected message shows the
offending span and a rewrite that can be sent verbatim; a device that will not
speak says which policy stopped it. An agent that can only be told "no" will
guess.

**Report what happened.** A receiver that cannot speak returns a status and a
reason, never silence and never a lie. The same applies to saying a change is
done: if the tests were not run, say so.

## Testing on real devices

Linux and Android get real runtime testing; the rest are build-verified. The
things no CI can cover are exactly the ones that have bitten: NAT traversal,
Android doze, network switching, audio actually coming out of a speaker, and
sockets left behind by a node that died. When a change touches those, test it
on hardware and say which hardware.

**Test the build you intend to ship.** Android's release build minifies and its
debug build does not, so for months "tested on a phone" meant a build nobody
would ever download — and the first release APK to reach a real phone died on
launch (#41). Before a release, install the *release* artefact and open it. The
same question applies to any packaged form: the Flatpak is `cargo build`, not
`tauri build`, and does not regenerate the CSS.
