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

- **On Windows, another account can take the pipe name before the node does**
  and deny service. The pipe namespace is global and nothing can prevent that.
  Nothing leaks when they do — a squatter cannot prove it knows the token —
  and the node identifies whatever holds the name and says plainly that it is
  not a node you started. On Linux and macOS the socket lives in a directory
  only you can enter, which is checked before binding rather than assumed
  (decision 103).
- **A revoked device keeps working until it syncs.** Revocation is eventually
  consistent by design; `rotate` is the answer when a device is out of your
  hands, and it says so in its own help text.
- **Any member may vouch for any device.** That is the design, not an
  oversight — [decision 39](docs/adr/0039-any-member-may-vouch-for-any-device-and-the-docs-now-say-so.md) explains why enforcing the
  inviter would stop nothing and would orphan devices.
- **iOS stops answering when backgrounded**, between five and ten minutes.
  That is the platform. Issue #137.
- **Who you speak to is visible, even though what you say is not.** Devices
  find each other by publishing and resolving their public key in DNS, so a
  query for `_iroh.<the other device's key>.dns.iroh.link` leaves the machine
  in the clear. Whoever runs the resolver — the network, the provider — can
  see that two particular keys looked each other up, and when. A relay sees
  the same shape of thing from the other side: that two keys exchanged data,
  when, and roughly how much. Neither sees the text. The node also multicasts
  a UPnP `M-SEARCH` on the local network to ask for a port mapping, which
  tells anything on that network that a node is here.

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

**The transport claim has been measured, once.** On 15 September 2026 two
nodes were paired and every byte each one handed to the kernel was logged —
earlier than a wire capture, so anything unencrypted would have been caught
before the network saw it. A uniquely marked message appeared in none of the
600 datagrams that went to the peer or the relay, in any encoding, while the
same log showed that method working: the local socket between the CLI and the
node carried readable device names, the invite ticket and the roster, as it
is meant to, and the cleartext DNS lookups above were plainly there. That was
Linux to Linux on one machine, on a build of 4 September; it says nothing
about the other four platforms.

None of this has been audited.
