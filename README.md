# voicecast

Speak text aloud on any of your devices, from the command line.

```
$ voicecast "build finished"
$ voicecast --to pixel "needs your input"
$ cat CHANGELOG.md | voicecast --strip --to laptop
```

Built for agents to notify you — on your desk, or on the phone in your pocket
while you're out. Devices connect **peer to peer**. There is no server to run
and no account to create.

## Running it today

Linux needs `espeak-ng` on `PATH` — the guaranteed floor engine:

```bash
sudo pacman -S espeak-ng     # Arch;  apt install espeak-ng  on Debian
```

```bash
voicecastd &                 # the node
voicecast "hello world"

voicecast invite             # on one device
voicecast join <ticket>      # on the other
voicecast devices
voicecast rename "Phil's Laptop"
voicecast --to laptop "build finished"
```

Two nodes can be run on one machine for testing by overriding
`VOICECAST_SOCKET` and `VOICECAST_CONFIG_DIR`.

## Status

**Design complete. Transport validated. Implementation not started.**

Every architectural question has been worked through and recorded. The riskiest
assumption — that peer-to-peer connections survive carrier-grade NAT and
network changes — has been [measured on real hardware](docs/m0-results.md)
rather than assumed. It holds.

## How it works

Each install is a **node** — both sender and receiver, one small Tauri app on
Linux, macOS, Windows, Android, and iOS. Devices join a **space** by scanning a
QR code once, and stay reachable afterwards even as they move between networks,
because they address each other by public key rather than by IP.

**Only text crosses the wire.** The receiving device synthesises it locally, so
five minutes of speech costs ~50KB instead of tens of megabytes, and each
device speaks in whatever voice it is configured with.

## Docs

| | |
|---|---|
| [architecture.md](docs/architecture.md) | The system: identity, discovery, membership, security |
| [setup.md](docs/setup.md) | What setting up four devices actually looks like |
| [cli.md](docs/cli.md) | Command surface, exit codes, targeting |
| [protocol.md](docs/protocol.md) | Wire format and stream model |
| [text.md](docs/text.md) | Validation and chunking rules |
| [build-plan.md](docs/build-plan.md) | Milestones, repo layout, CI |
| [m0-results.md](docs/m0-results.md) | Measured transport results on real devices |
| [decisions.md](docs/decisions.md) | Every decision, with its rationale and cost |

Start with `decisions.md` if you want to know *why* rather than *what*.
