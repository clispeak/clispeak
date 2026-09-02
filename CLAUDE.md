# Working on voicecast

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

`cargo run -p xtask -- portability` fails if `voicecast-proto`,
`voicecast-text` or `voicecast-core` grows a `#[cfg(target_os)]`. Platform
divergence belongs in `voicecast-engine` or the Tauri shell. Anything that
cannot be expressed portably gets a trait, not a conditional in the middle of
business logic.

## Where things live

| Crate | Holds | Must stay portable |
|---|---|---|
| `voicecast-proto` | Wire and IPC types. Pure data, CBOR | yes |
| `voicecast-text` | Validation, chunking. Pure functions | yes |
| `voicecast-core` | Transport, roster, spaces, queue, policy, history | **yes** |
| `voicecast-engine` | `SpeechEngine` trait and per-platform impls | no |
| `voicecast-cli` | The `voicecast` binary. Depends only on proto + text | — |
| `voicecast-daemon` | Headless node, for machines with no desktop | — |
| `voicecast-keystore` | Platform keyring | — |
| `app/src-tauri` | The app: owns a `Node`, adds a tray and a window | no |
| `app/src` | Frontend. Plain HTML and JS, Tailwind build step | — |

`voicecast-cli` depending on only two crates is deliberate — it keeps startup
at ~3ms, which is the whole premise of the thin-client design. Do not reach
for `voicecast-core` from it; duplicate the handful of bytes instead, as
`frame.rs` and the socket name already do, and keep them in step by hand.

## Checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p xtask -- portability
cargo test --workspace
cd app && npx tailwindcss -i src/input.css -o src/styles.css --minify
```

The CSS is committed and the app loads it directly — editing `index.html`
classes without rebuilding leaves them inert, and the symptom is silent.

**On a memory-constrained machine**, wrap cargo so an OOM kills the build and
not your session: `systemd-run --user --scope -p MemoryMax=3G -p
MemorySwapMax=0 cargo …`. `.cargo/config.toml` already limits jobs and
debuginfo for the same reason.

## Running it

```bash
cargo run -p voicecast-daemon          # a node
cargo run -p voicecast-cli -- status
```

Two nodes on one machine: override `VOICECAST_SOCKET` and
`VOICECAST_CONFIG_DIR`. Both are read by the CLI *and* the node, and
forgetting the socket override makes every command talk to the first node,
which looks like two unrelated bugs.

Platform packages: `packaging/flatpak/` for Linux, `cargo xtask bundle` for a
macOS `.app`, and `npx @tauri-apps/cli android build --apk` for Android.
`cargo xtask piper` puts Piper and a voice where the engine looks.

## Conventions

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
