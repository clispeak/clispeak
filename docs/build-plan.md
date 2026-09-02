# Build plan

**Phase 1 targets Linux and Android.** They get real runtime testing. macOS,
Windows, and iOS are **build-verified from day one but not exercised** — the
point is that adding them later is a matter of testing, never of untangling
architecture.

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

CI enforces this from the first commit, even though three targets are never
run. The failure mode this prevents is well known: six months of Linux-shaped
assumptions quietly accumulating in shared code, discovered the week someone
tries to build for iOS.

Concretely:

- `voicecast-proto`, `voicecast-text`, and `voicecast-core` contain **no** `#[cfg(target_os)]`.
- All platform divergence lives in `voicecast-engine` and the Tauri shell.
- Anything that can't be expressed portably gets a trait in `voicecast-core` and an
  implementation in `voicecast-engine`, not a conditional compile in the middle of
  business logic.

## Repo layout

```
voicecast/
├── Cargo.toml                  workspace
├── crates/
│   ├── voicecast-proto/        CBOR wire types + IPC types. Pure data.
│   ├── voicecast-daemon/       the node process. Desktop only.
│   ├── voicecast-text/         validation, protection, chunking. Pure fns.
│   ├── voicecast-engine/       SpeechEngine trait + per-platform impls.
│   ├── voicecast-core/         iroh transport, roster, queue, playback.
│   └── voicecast-cli/          the `voicecast` binary. Thin: proto + text.
├── app/
│   ├── src/                    Tauri frontend
│   └── src-tauri/              Tauri shell, depends on voicecast-core
├── xtask/                      build automation
└── docs/
```

Each crate has an obvious test story, which is the reason for the split:

| Crate | Testable how |
|---|---|
| `voicecast-proto` | Round-trip encode/decode, version-skew cases |
| `voicecast-text` | Table-driven unit tests. Every ugly case in `text.md` is a row. |
| `voicecast-engine` | Per-platform smoke tests; trait conformance |
| `voicecast-core` | Two nodes in one process over an in-memory transport |
| `voicecast-cli` | Golden-file tests on stdout/stderr and exit codes |

`voicecast-cli` depending only on `voicecast-proto` and `voicecast-text` is deliberate — it keeps
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

### M2 — `voicecast-text`  ✅ DONE

Validation and chunking. Pure functions, no I/O, no async. Every ugly case in
`text.md` becomes a test row.

Deliberately first among the real crates: highest test density, zero risk, and
it needs no decisions from anywhere else.

*Exit:* markdown and URL rejection with correct spans and suggestions;
protection-pass splitting handles `10.0.0.1`, `src/main.rs:42`, `Dr.`,
`v1.2.3`; fallback cascade never splits mid-word.

### M3 — Speak locally  ✅ DONE

CLI → unix socket → node → `voicecast-engine` → sound. No network, no identity, no
roster.

Requires `espeak-ng` installed (`pacman -S espeak-ng`) — a runtime
dependency, not a build one.

*Exit:* `voicecast "hello world"` speaks on the same Linux machine through
espeak-ng. `echo x | voicecast` works. Exit codes correct. **First demoable thing.**

### M4 — Identity and a space of one  ✅ DONE

Keypair generation, keyring storage, `voicecast init`, `voicecast status`.

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

### M8 — The rest of the CLI  🟡 IN PROGRESS

Priority and queue, `stop`/`skip`/`pause`, groups, multiple spaces, quiet
hours, `--wait`/`--json`, rotation.

*Exit:* `cli.md` is fully implemented.

---

## Testing strategy

**Unit** — heaviest in `voicecast-text` and `voicecast-proto`. Roster CRDT merge is a good
property-test target: merge must be commutative, associative, idempotent.

**Integration** — two `voicecast-core` nodes in one process over an in-memory
transport. Covers roster convergence, queue behavior, and cancellation without
touching a network.

**Manual, and unavoidable** — real devices for CGNAT traversal, doze mode,
network switching, and audio output. No CI substitute exists for these, which
is exactly why M0 comes first.

## Deferred past phase 1

Full macOS and Windows test coverage (they get smoke tests only) · **all** iOS
runtime testing, pending hardware · `iroh-blobs` voice sync between devices ·
cloud/API voices · smart speakers.

All of these are additive. None require revisiting a phase 1 decision — which
was the goal of the compile-for-five rule.
