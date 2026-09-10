# Build plan

**Phase 1 targets Linux and Android.** They get real runtime testing. Windows
and iOS are **build-verified from day one but not exercised** — the point is
that adding them later is a matter of testing, never of untangling
architecture.

macOS has since been exercised, and bore that out: it needed a speech engine
wired up and a packaging story, and no change to a portable crate beyond
[one bug the abstract-socket assumption had hidden](#m9--macos). Its runtime
coverage is still a smoke test rather than the full pass Linux gets.

Windows has since been exercised too, and bore it out again: a speech engine,
an audio path, and no change to any portable crate at all. What it added to
the pattern is that a platform can be *build-verified and installed correctly*
and still not speak — Piper links a runtime Windows does not ship — so
discovery now checks that the binary starts rather than only that it exists.
Its packaging story is still open, and its runtime coverage is a smoke test.

---

## Available hardware

| Device | Platform | Role in phase 1 |
|---|---|---|
| Linux laptop | linux | Primary dev and test target |
| Android phone | android | Primary mobile target; the only cellular endpoint |
| M4 Mac | macos (arm64) | Opportunistic smoke tests. **Required for iOS builds later.** |
| Windows PC | windows | Opportunistic smoke tests |

Four of five platforms are physically available, so macOS and Windows are not
purely theoretical — they get **CI build verification plus an occasional
manual smoke test**, which is nearly free and catches gross breakage without
committing to full test coverage.

**iOS is the only platform with no hardware.** It stays build-only until a
device exists; the Mac means that path is open whenever one does.

**One carrier only.** There is a single phone, so cellular results generalize
to one network's NAT behavior rather than to carriers broadly. See M0 for what
that does and does not tell us.

## The rule that keeps later targets cheap

> **If it doesn't compile for all five targets, it doesn't merge.**

**Local gates are not this gate.** `cargo test`, `cargo clippy` and
`cargo fmt` only ever exercise the machine they run on, so passing them says
nothing about four of the five targets. Windows was broken for fifteen commits
while local checks reported green every time — see issue #5.

CI enforces this from the first commit, even though three targets are never
run. The failure mode this prevents is well known: six months of Linux-shaped
assumptions quietly accumulating in shared code, discovered the week someone
tries to build for iOS.

Concretely:

- `clispeak-proto`, `clispeak-text`, and `clispeak-core` contain **no** `#[cfg(target_os)]`.
- All platform divergence lives in `clispeak-engine` and the Tauri shell.
- Anything that can't be expressed portably gets a trait in `clispeak-core` and an
  implementation in `clispeak-engine`, not a conditional compile in the middle of
  business logic.

## Repo layout

```
clispeak/
├── Cargo.toml                  workspace
├── crates/
│   ├── clispeak-proto/        CBOR wire types + IPC types. Pure data.
│   ├── clispeak-daemon/       the node process. Desktop only.
│   ├── clispeak-text/         validation, protection, chunking. Pure fns.
│   ├── clispeak-engine/       SpeechEngine trait + per-platform impls.
│   ├── clispeak-core/         iroh transport, roster, queue, playback.
│   └── clispeak-cli/          the `clispeak` binary. Thin: proto + text.
├── app/
│   ├── src/                    Tauri frontend
│   └── src-tauri/              Tauri shell, depends on clispeak-core
├── xtask/                      build automation
└── docs/
```

Each crate has an obvious test story, which is the reason for the split:

| Crate | Testable how |
|---|---|
| `clispeak-proto` | Round-trip encode/decode, version-skew cases |
| `clispeak-text` | Table-driven unit tests. Every ugly case in `text.md` is a row. |
| `clispeak-engine` | Per-platform smoke tests; trait conformance |
| `clispeak-core` | Two nodes in one process over an in-memory transport |
| `clispeak-cli` | Golden-file tests on stdout/stderr and exit codes |

`clispeak-cli` depending only on `clispeak-proto` and `clispeak-text` is deliberate — it keeps
the binary small and its startup instant, which is the whole premise of the
thin-client design.

## CI from the first commit

| Runner | Builds | Tests |
|---|---|---|
| ubuntu | linux, android (`cargo-ndk`) | full |
| macos | macos, ios | build only |
| windows | windows | build only |

macOS and Windows additionally get **manual smoke tests** on the hardware
above whenever a milestone lands — cheap, and enough to catch gross breakage
long before those platforms get real attention.

Three runners cover five targets. macOS/Windows/iOS turn red the moment
someone writes non-portable code, which is the entire point.

---

## Milestones

### M0 — Spike: how well does iroh behave on cellular?  ✅ PASSED

> **Done 2026-09-01. See [m0-results.md](m0-results.md).** CGNAT traversal
> went direct (91%), and a wifi→cellular switch caused *zero* reconnects —
> QUIC migration carried the connection across. Both better than assumed.
> Cost is a ~16s stall on the in-flight message at the moment of the switch.

Pair-once, off-wifi reachability, and "no server you run" all rest on iroh's
hole-punching and pkarr discovery working on real mobile networks, which are
almost universally CGNAT.

**But the bar is lower than it first appears.** Relay fallback does not depend
on NAT traversal at all, so "cannot connect" is largely ruled out by
construction. The real question is **how often a direct path is achieved versus
relayed, and whether relayed latency is acceptable** — which a single carrier
answers perfectly well.

Throwaway code. **Do not build on it.**

| # | Side A | Side B | What it tests |
|---|---|---|---|
| 1 | Linux, home wifi | Android, same wifi | mDNS + direct. Baseline. |
| 2 | Linux, home wifi | Android, **cellular** | CGNAT traversal. The critical case. |
| 3 | Linux, **public wifi** | Android, cellular | Both ends hostile. Worst realistic case. |
| 4 | Linux, home wifi | Android, cellular, **relay forced** | Relay latency, deterministically. |
| 5 | Linux, home wifi | Android switching wifi ↔ cellular | Re-resolution after a network change. |

Row 4 matters more than it looks: forcing the relay path removes the need for
a hostile network to measure relay latency. Row 5 is the one that validates
pair-once — an address change must not require re-joining.

*Measure:* connection success rate, direct-vs-relay ratio, time to first byte,
reconnect time after a network switch.

*What would change course:* connections failing **even over relay** would
force a transport rethink. Frequent relaying is **an expected and acceptable
result** on cellular — record the latency and continue. Slow or unreliable
re-resolution after a network change is the finding most likely to require
design work, since pair-once depends on it.

*Optional, if more confidence is wanted:* a prepaid SIM or a free MVNO eSIM
trial gives a second carrier for £10–20. Not required — the relay path is the
mitigation for carrier variation, and it is being measured in row 4 regardless.

### M1 — Skeleton and CI  ✅ DONE

Workspace, five-target CI matrix, `xtask`, licence, lint and format gates.
Nothing functional.

*Exit:* an empty workspace builds green for all five targets.

### M2 — `clispeak-text`  ✅ DONE

Validation and chunking. Pure functions, no I/O, no async. Every ugly case in
`text.md` becomes a test row.

Deliberately first among the real crates: highest test density, zero risk, and
it needs no decisions from anywhere else.

*Exit:* markdown and URL rejection with correct spans and suggestions;
protection-pass splitting handles `10.0.0.1`, `src/main.rs:42`, `Dr.`,
`v1.2.3`; fallback cascade never splits mid-word.

### M3 — Speak locally  ✅ DONE

CLI → unix socket → node → `clispeak-engine` → sound. No network, no identity, no
roster.

Requires `espeak-ng` installed (`pacman -S espeak-ng`) — a runtime
dependency, not a build one.

*Exit:* `clispeak "hello world"` speaks on the same Linux machine through
espeak-ng. `echo x | clispeak` works. Exit codes correct. **First demoable thing.**

### M4 — Identity and a space of one  ✅ DONE

Keypair generation, keyring storage, `clispeak init`, `clispeak status`.

*Exit:* identity survives a restart; the keyring is used, not a bare file.

### M5 — Two Linux machines  ✅ DONE

iroh transport, control stream, `Hello`, roster, `SpeakBegin`/`Chunk`/
`SpeakEnd`, `Status` back. Join by pasted ticket — QR can wait.

*Exit:* two Linux boxes on a LAN join a space and speak to each other; then
the same across networks, exercising the relay path from M0.

### M6 — Android  ✅ DONE

Tauri v2 mobile shell, `TextToSpeech` engine, foreground service, QR scanning,
join flow, receiver settings UI.

**This is where cellular testing happens for real** — M0 proved the transport,
this proves it inside a real app with a real lifecycle: doze mode, network
changes, process death, app backgrounding. Android's lifecycle is a different
and harder question than NAT traversal, which is why the two are separate
milestones.

Must call `install_android_jni_context` — see M0 finding 5.

*Exit:* the four-device walkthrough in `setup.md` works for Linux + Android;
the phone receives on cellular after being idle for hours.

### M7 — Voices and fallback

Piper download with pinned URLs and checksums, engine tiering, reason codes,
fallback UI on both platforms, `Presence` reporting.

*Exit:* a fresh Linux install speaks via espeak immediately, shows *why* it is
in fallback, and upgrades to Piper without a restart.

### M8 — The rest of the CLI  🟢 ALL BUT ONE ITEM

Priority and queue, `stop`/`skip`/`pause`, groups, multiple spaces, quiet
hours, `--wait`/`--json`, rotation. All built and exercised between a Linux
laptop and an Android phone, plus message history, which was not in the
original plan.

*Exit:* `cli.md` is fully implemented, apart from volume, which does not exist
at any level. Per-space mute and quiet hours landed later, with issue #2 — a
device policy that acts as a floor and an optional override per space. See
[decision 29](docs/adr/0029-a-space-may-be-quieter-than-its-device-never-louder.md) for why an override can only add silence.

### M9 — macOS

Piper on a Mac, a bundled `.app` that installs by dragging, and the CLI put
somewhere a shell — and therefore an agent — will actually find it.

Confirms what the compile-for-five rule was for: the portable crates needed no
platform conditionals. The one real defect it surfaced was in `clispeak-core`
and had been latent on every platform, hidden because Linux's abstract sockets
disappear with the process that held them — a node killed anywhere else left a
socket file that stopped every later node from starting.

*Exit:* installing the dmg on a Mac with nothing else installed gives a device
that speaks, and `clispeak` on the PATH. **Done, apart from signing.**

A Mac has since joined a space and been spoken to from the laptop, so both
legs are proven. The earlier timeouts were that machine being offline — and
worth knowing, an ad-hoc signed bundle is a new identity to macOS on every
rebuild, so the keychain prompt blocks node startup until someone clicks it.
A Mac waiting on that dialog is indistinguishable from one that is off.

### M10 — Windows

Piper, an installer that does the whole job, and the command-line tool on the
PATH without anyone opening a terminal.

**The bar is higher here than on the other desktops, on purpose.** Linux and
macOS are installed by people comfortable in a shell; a Windows user is
assumed not to be, and anything left for them to do by hand will not get done
— and will look like a broken app rather than an unfinished install. So the
installer carries Piper, a voice *and* the Visual C++ runtime, sets the PATH
itself, and checks the result before it closes.

The runtime matters more than it sounds. A clean Windows install has none, and
without it `piper.exe` installs correctly, is found correctly, and exits
`0xC0000135` with no message and no named library. Bundling the three CRT DLLs
beside it removes the failure rather than reporting it.

*Exit:* a double-click on a clean Windows machine gives a device that speaks
and a `clispeak` a **new** shell can find, with no terminal step and no admin
prompt at any point.

**Where it stands: done.** Windows speaks through SAPI 5 rather than Piper, so
the installer carries no speech payload and no MSVC runtime. It has shipped in
seven releases and been installed on a clean machine with no admin prompt.

**Build in CI, test a downloaded artefact.** Any Windows machine able to
*build* an installer has the MSVC Build Tools, and those install the very
runtime whose absence was the bug — so a build machine is by construction one
where that failure cannot reproduce. That is how #201 survived: it was found
by a person on a clean VM, and was visible in the artefact's import table the
whole time.
Tracked as #30 and #20, the latter being whether we may redistribute the CRT
at all.

### M11 — Publishing  ✅ DONE

**Anyone can download it and be spoken to.** Seven releases have gone out, the
site offers the latest build for four platforms, and both signing credentials
exist and are held behind an approval a person gives.

This section said "Every platform builds. Nothing can be downloaded. That gap
is the whole of what is left before anyone but us runs this" for as long as it
took to close every item under it. Each of the seven issues below is closed;
the paragraph naming them as open outlived all of them.

| | |
|---|---|
| ~~#24~~ | settled: MIT OR Apache-2.0, open source — decision 74 |
| ~~#23~~ | gone rather than answered: the repository is public, so a release asset is a public URL |
| ~~#25~~ | clispeak.com, published from `site/` by a workflow |
| ~~#29~~ | a Developer ID certificate; releases are signed and notarised |
| ~~#31~~ | an Android release key, held in a protected environment |
| ~~#20~~ | gone with Piper: Windows moved to SAPI 5 and imports no MSVC runtime |
| ~~#5~~ | branch protection requires the six named checks |

*Exit, met:* a person who has never seen this repository can install it on
their own machine, from a link, and be spoken to.

### M12 — Audit findings

A full code and security review on 2026-09-02, five reviewers by area (core,
proto/text/CLI, engine and Android, the app, CI and packaging), every finding
verified against the code before it was filed. Thirty-seven issues, all
carrying the `audit` label; the ones that must close before 1.0 are on the
milestone.

What it found, in one paragraph: the roster merge trusts any signed timestamp,
so a member can make itself unrevokable (#48) and the panic button has a
five-minute hole (#50); a departed space's messages fall into another space's
roster (#51); a receiver speaks any size of chunk a member sends, and one long
chunk deadlocks Piper (#53); the local socket is unauthenticated (#54); peer
names reach the agent's terminal unescaped (#55); the engine's `stop` cannot
interrupt anything (#58); `rename` reverts within a minute (#62); a flag after
the text is a usage error (#64); and the release workflow has been building a
debug APK all along (#68). Nothing was found in the webview: one `innerHTML`,
correctly trusted, and an empty capability set.

What it says about the process: every high finding sits at a boundary no test
crosses (#80), and three of the worst are the same fact held in two places
(#79). The fix for the crate is a seam, not a rewrite.

*Exit, met:* every `security` issue closed, #80's harness exists — two nodes
talk inside one process — and the release APK is the release build.

---

## Testing strategy

**Unit** — heaviest in `clispeak-text` and `clispeak-proto`. Roster CRDT merge is a good
property-test target: merge must be commutative, associative, idempotent.

**Integration** — two `clispeak-core` nodes in one process over an in-memory
transport. Covers roster convergence, queue behavior, and cancellation without
touching a network.

**Manual, and unavoidable** — real devices for CGNAT traversal, doze mode,
network switching, and audio output. No CI substitute exists for these, which
is exactly why M0 comes first. Every bug found by a person running the app on
real hardware has been of this kind, and none of them was reachable by any
gate in this repository.

## Deferred past phase 1

`iroh-blobs` voice sync between devices · cloud/API voices · smart speakers ·
shipping iOS, which builds and is tested but has no distribution route that is
a download link (decision 104).

macOS and Windows were on this list as "smoke tests only". They are not: both
are installed from release artefacts and run by a person, and both produced
bugs that way — two on Windows in the first hour, three on macOS in a morning.

All of these are additive. None require revisiting a phase 1 decision — which
was the goal of the compile-for-five rule.
