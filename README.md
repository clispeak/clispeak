# clispeak

Speak text aloud on any of your devices, from the command line.

```
$ clispeak "build finished"
$ clispeak --to pixel "needs your input"
$ clispeak --to all --priority high "deploy failed"
$ cat CHANGELOG.md | clispeak --strip --to laptop
```

Built for agents to notify you — on your desk, or on the phone in your pocket
while you're out. Devices connect **peer to peer**. There is no server to run
and no account to create.

## Get it

**[clispeak.com](https://clispeak.com)** — downloads for Linux, Windows,
Android and macOS, with a sentence for each about what the operating system
will say when you open it, because every one of them says something alarming
the first time.

Send anybody who just wants to *use* this there. The rest of this file is for
building it yourself and understanding how it works.

## What it does

**Speaking.** Any length up to 100,000 characters, about two hours of speech;
past that the receiving device refuses the message and says so, rather than
tying up its speaker for the afternoon. Markdown and bare URLs are *rejected*
with a suggested rewrite rather than silently mangled, so an agent can correct
itself — `--strip` converts instead, `--raw` skips the check.

**From the app as well as the terminal.** The **Speak** tab picks a device, or
everyone, and sends, then reports back a line per device. It is how a phone
sends at all, and it is the quickest check that a new pairing carries sound.

**Targeting.** One device, a comma-separated list, a locally-defined group,
`all`, or `here`. Several devices are reached at once rather than one after
another.

**Priority.** `high` interrupts what is playing, says its piece, and then the
interrupted message resumes from the sentence it was cut off in. `low` is
dropped when the queue is already deep.

**Quiet.** Each device decides for itself whether it will make noise: mute, and
a daily quiet window. `high` breaks through quiet hours only if that device
allows it, and nothing breaks through mute. A sender cannot change either.

**History.** Every message a device is asked to speak is recorded, spoken or
not. A message refused while the device was muted is kept and marked unheard,
to be read or played later — playing works through mute, because pressing play
is the ask.

**Control.** `stop`, `skip`, `pause`, `resume` and `queue`, on this device or
on any other, plus in the app for whatever is playing right now.

**Spaces.** A device can belong to several separate sets of your own devices.
Bare names resolve in the default one; `work/laptop` reaches anywhere else.
There is deliberately no selector meaning "everywhere". `rotate` replaces a
space outright, which locks a lost device out immediately rather than
eventually.

A space's name is local. It is how *this* device writes `work/laptop`, nothing
is sent when it changes, and two devices in one space may call it different
things.

See [cli.md](docs/cli.md) for the full surface and exit codes.

## Platforms

| | Speech | Background |
|---|---|---|
| Linux | Piper, falling back to espeak-ng if the host has it | tray app |
| macOS | Apple's own | tray app |
| Android | system text-to-speech | foreground service + battery exemption |
| Windows | SAPI 5 | tray app |
| iOS | Apple's own | **foreground only** — see below |

iOS is built and not shipped. The Android build is **`arm64-v8a` only**, which
is every phone made since about 2019 and no emulator.

**iOS only speaks while the app is on screen**, and that is the platform
rather than an unfinished corner. Backgrounded, it stops answering somewhere
between five and ten minutes. Nothing available fixes it: the audio background
mode keeps an app alive while it is *playing*, not while it is *waiting*, and
everything that wakes a suspended iOS app needs a push server. There is no
server here, which is the point of the project. So a phone in a pocket is
exactly the case iOS cannot serve. Android can, and does.

A message does not sound identical on every platform. Each device speaks in
whatever voice it is configured with, using its own system's synthesiser where
there is a good one.

## How it works

Each install is a **node** — both sender and receiver, one small Tauri app.
Devices join a **space** by scanning a QR code once, and stay reachable
afterwards even as they move between networks, because they address each other
by public key rather than by IP.

**Only text crosses the wire.** The receiving device synthesises it locally, so
five minutes of speech costs ~50KB instead of tens of megabytes.

**There is no shared group secret.** Authorisation is "is this public key in my
roster?", so compromising one device leaks nothing that decrypts another's
traffic. The roster is an add-only set with tombstones, signed by whoever
invited each member, which is what lets a device admit a peer it has never met.

The riskiest assumption — that peer-to-peer connections survive carrier-grade
NAT and network changes — was [measured on real hardware](docs/m0-results.md)
before anything was built on top of it. 91% of connections went direct, and
switching between wifi and cellular caused zero reconnects.

## Pairing

```bash
clispeak invite             # on one device — prints a ticket, app shows a QR
clispeak preview <ticket>   # on the other — what that code would join
clispeak join <ticket>      # ...and join it
clispeak devices
```

The destination is written *into* the ticket by whoever minted it, so the
joining device does not get to choose it. `preview` reads it out first —
locally, contacting nobody — so a code can be checked before it is used. In the
app, joining goes through the same two steps: paste or scan, see which space it
joins, then confirm. `join --name` picks what to call it here.

## Using it from an agent

`skills/clispeak/SKILL.md` is an agent skill. Install it with

```bash
clispeak skill --install                          # Claude Code's default location
clispeak skill --install --path <dir>/SKILL.md    # anywhere else
clispeak skill                                    # print it, to pipe somewhere
```

or from the app's Settings tab on a desktop.

The skill gives an agent the judgement `--help` cannot: when speaking is worth
doing at all, which device suits which kind of message, what each exit code
means for what to do next, and that a `muted` device is a decision to respect
rather than a failure to retry.

It also walks you through a one-time working agreement — what the agent should
call itself when it speaks, which device is the default, and where the line is
between speaking and printing. The naming matters once more than one agent can
reach the same phone: a voice from a pocket that does not say whose it is makes
you guess.

## Building it yourself

Rust 1.98 or newer, and `npm install` in `app/` once.

**Linux.** The Flatpak carries Piper and a voice, so it speaks the moment it is
installed:

```bash
git submodule update --init                 # the manifest's shared-modules
npm --prefix app ci
npm --prefix app run build:css              # styles.css is generated, not committed
cargo build --release -p clispeak-app -p clispeak-cli
cd packaging/flatpak
flatpak-builder --force-clean --user --install build-dir org.clispeak.app.yml
flatpak run org.clispeak.app
```

All four of the first lines are needed. The manifest takes the two binaries
from `target/release`, and `cargo build` — unlike `tauri build` — does not run
Tailwind, so without the CSS step the app comes up unstyled with nothing to say
why.

**macOS.** The bundle carries the command-line tool and no speech payload,
since macOS speaks through the platform synthesiser, so a drag to
`/Applications` is the whole install:

```bash
cargo xtask bundle
open target/release/bundle/dmg/clispeak_*_aarch64.dmg
```

A locally built `.app` is ad-hoc signed, which runs but derives its identity
from the binary's own hash — so every rebuild looks like a different program to
macOS and the keychain grant holding the device identity is asked for again.
Any stable certificate, self-signed included, ends that; `docs/signing.md` has
the steps. Release builds are signed and notarised, so a downloaded `.dmg`
opens without Gatekeeper's warning.

**Android.** Build and install over USB:

```bash
cd app
ANDROID_HOME=~/Android/Sdk NDK_HOME=~/android-ndk-r29 \
  npx @tauri-apps/cli android build --apk --debug --target aarch64
adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

**Without a desktop.** `clispeakd` is a headless node. On Linux it needs Piper
where the engine looks for it, which `cargo xtask piper` arranges; it can
otherwise fall back to `espeak-ng` on `PATH`. On macOS and Windows it uses the
platform synthesiser and needs nothing installed.

**The `clispeak` command** is installed to `~/.local/bin` by the app on first
launch, and re-installed whenever the app is updated. On Windows the installer
puts it on your PATH — open a *new* terminal afterwards, since an existing one
keeps the environment it started with.

**Checks.** One command runs all of them:

```bash
cargo run -p xtask -- check
```

Two nodes can share one machine for testing by overriding `CLISPEAK_SOCKET` and
`CLISPEAK_CONFIG_DIR`. `CLISPEAK_SOCKET` is a *name*, not a path, and a value
with a separator in it is refused rather than reinterpreted.

## Who can drive your node

The CLI talks to the node over a local socket, and the two prove themselves to
each other before anything is sent — with a secret in the config directory,
which is kept readable only by you.

That matters because the socket *name* has no permissions on any platform here:
Linux's abstract namespace has none by design, and on macOS the socket lands in
`/tmp`. So another user on the same machine cannot make your devices speak,
read your history, or mint an invite to your space. They can still take the
name before your node does, which stops it starting — a nuisance, not a leak.

On Windows a named pipe carries access rules instead: only the account that
created it, LOCAL SYSTEM and Administrators can open it.

Anyone who can read your config directory can drive your node. That is the same
directory that holds your identity key, so the boundary is the same one.

**The Flatpak is packaging, not containment.** It grants itself write access to
`~/.local/bin`, which is on your PATH ahead of `/usr/bin`, and to `~/.claude`,
which holds hooks your agent runs. Either is enough to escape the sandbox, and
they are there because an app that offers to install a tool and then silently
fails to is worse than one that says what it can reach. Install it because it
is convenient, not because it is contained.

Reporting a security issue: [`SECURITY.md`](SECURITY.md).

## Docs

| | |
|---|---|
| [architecture.md](docs/architecture.md) | The system: identity, discovery, membership, security |
| [setup.md](docs/setup.md) | What setting up four devices actually looks like |
| [cli.md](docs/cli.md) | Command surface, exit codes, targeting |
| [protocol.md](docs/protocol.md) | Wire format and stream model |
| [text.md](docs/text.md) | Validation and chunking rules |
| [m0-results.md](docs/m0-results.md) | Measured transport results on real devices |
| [licensing.md](docs/licensing.md) | What may be redistributed, and the working behind it |
| [adr/](docs/adr/) | Every decision, one file each, with its rationale and cost |

Start with `docs/adr/` if you want to know *why* rather than *what*.

## Contributing

Bug reports and pull requests are welcome.
**[`CONTRIBUTING.md`](CONTRIBUTING.md)** covers what is unusual about this
repository before you spend time on a change — chiefly that it has to compile
for all five targets, and that a green CI run means *compiled on five, tested
on one*.

## Licence

**MIT OR Apache-2.0**, at your option — the Rust ecosystem's usual pair.
Apache-2.0 carries an explicit patent grant; MIT is there for anyone who
prefers the shorter terms. See [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE).

**The speech payload is not ours, and is not shipped everywhere.** macOS, iOS,
Windows and Android use the platform's own synthesiser and carry no payload at
all. Linux ships Piper — MIT, as are its phonemiser and ONNX Runtime — with
espeak-ng, which is GPL-3.0-or-later, and the LJ Speech voice, which is public
domain.

A voice model carries the terms of the corpus it was trained on, and those are
frequently *not* the licence label on the model. If you change the default
voice, read the corpus terms rather than the model card's licence field;
[`docs/licensing.md`](docs/licensing.md) shows the working, and is a careful
reading of licence text by people who are not lawyers.
