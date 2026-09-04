# Security

## Reporting a vulnerability

**Please do not open a public issue.** Use GitHub's private reporting —
**Security → Report a vulnerability** on this repository — which reaches the
maintainers without disclosing anything.

Expect an acknowledgement within a few days. This is a small project; there is
no security team and no on-call rota, and saying so is more useful than a
service level nobody staffs.

## What is worth reporting

The design puts a few things in the trust boundary, and those are where a
vulnerability would hurt most:

- **Anything that lets a device speak on another without being in its space.**
  Authorisation is membership of a signed roster and nothing else, so a way
  round that is the most serious class here.
- **Anything that forges, replays or survives a revocation.** Membership
  records are signed; tombstones are not, and their limits are deliberate and
  documented (`docs/architecture.md`).
- **Anything that lets another local user on the same machine drive a node,
  read what was meant for it, or take its socket name.** The CLI and the node
  prove themselves to each other with a per-boot token.
- **Anything a peer-supplied string can do to a terminal, a log or an agent's
  transcript.** Device names and space labels are chosen by whoever owns that
  device, and this tool exists to be read by an *agent* — so text that forges
  a line of output is a real attack here, not a cosmetic one.

## What is already known, and is not a vulnerability

These are documented limitations rather than surprises. A report of one is
welcome but will be closed as known:

- **The local socket name is not protected by the operating system.** Another
  local user can take it before the node does and deny service. Nothing leaks
  when they do — a squatter cannot prove it knows the token — and the node
  says so rather than blaming itself. Issue #128.
- **A revoked device keeps working until it syncs.** Revocation is eventually
  consistent by design; `rotate` is the answer when a device is out of your
  hands, and it says so in its own help text.
- **Any member may vouch for any device.** That is the design, not an
  oversight — decision 39 in `docs/decisions.md` explains why enforcing the
  inviter would stop nothing and would orphan devices.
- **iOS stops answering when backgrounded**, between five and ten minutes.
  That is the platform. Issue #137.

## Scope

This repository, the `clispeak` command, and the desktop and mobile apps built
from it. The speech engines are third-party — Piper, espeak-ng, and each
platform's own synthesiser — and vulnerabilities in those belong upstream.

## Cryptography

Device identity is an ed25519 key, held in the platform keyring where there is
one. Transport is QUIC with TLS, through `iroh`. Membership records are signed
with the inviter's key over a payload with a fixed domain separator, and that
separator is pinned by a test precisely because changing it silently voids
every signature in existence — decision 83 records the day that cost.

None of this has been audited.
