n# Setup walkthrough

The intended first-run experience, end to end, for someone setting up four
devices. This is a design target, not documentation of built software.

**The four devices:**

| Name | What | Role |
|---|---|---|
| `desk` | Linux desktop | Where the agent runs. Sends most messages. |
| `laptop` | MacBook | Sends and receives. |
| `pixel` | Android phone | Receives. |
| `iphone` | iPhone | Receives (foreground only). |

**What the user never does:** create an account, run a server, forward a port,
configure a static IP, or type an IP address.

---

## Who can invite

**Any member of the space can invite a new device.** There is no privileged
device and no "original" device with special rights. The founder is simply the
first member; once a second device joins, the two are interchangeable.

**Every device can do it from its GUI.** All five platforms run the same app,
so the invite screen exists everywhere. On desktop you also get `tts invite` —
the CLI and the GUI button do the identical thing, mint the identical ticket,
and honor the identical five-minute expiry. The CLI draws the QR in unicode
blocks; the app draws a real one.

| Device | Can invite from | Can join by |
|---|---|---|
| Desktop / laptop | `tts invite`, or the app window | pasting a ticket (CLI or app); camera if it has one |
| Phone / tablet | the app | scanning a QR, or tapping a `tts://join/` link |

Two consequences worth stating plainly:

**Losing the founder device breaks nothing.** Revoke it from any other member
and carry on. Its signature stays valid at the root of the trust chain for
devices it invited, but it holds no ongoing authority.

**Any member can also invite an attacker.** If a device is compromised, it can
add another device to the space. This is the accepted tradeoff for a personal
mesh where the alternative — requiring approval from a second device on every
join — makes the common case worse to save the rare one. If that balance ever
needs to change, it's a per-space toggle rather than a redesign.

## 1. Desktop — install and create the space

```
$ brew install tts        # or: paru -S tts, winget install tts

$ tts init
  Generated identity for this device.
  Device name [desk]:

  Created a new space.
  desk is now ready. Add another device with:  tts invite
```

`init` generates the ed25519 keypair, stores it in the system keyring, and
creates a space containing one member. The tray app is now running and
listening.

The desktop can already talk to itself:

```
$ tts "hello world"
```

## 2. Android — join by scanning

On the desktop:

```
$ tts invite

  █▀▀▀▀▀█ ▀▄█▀▄ █▀▀▀▀▀█
  █ ███ █ ▀█ ▄▀ █ ███ █
  █ ▀▀▀ █ █▄▀▄█ █ ▀▀▀ █
  ▀▀▀▀▀▀▀ █▄▀▄█ ▀▀▀▀▀▀▀
  ...

  Or paste this on the other device:
    tts://join/AXQm9Rk2...vB7z

  Expires in 5:00.  Waiting...
```

On the phone: install the app, open it, tap **Join a space**, point the camera
at the terminal.

```
  ┌─────────────────────────────┐
  │  Joining space...           │
  │                             │
  │  Device name:  [ pixel    ] │
  │                             │
  │  Safety code:   4821-9903   │
  │  Confirm this matches the   │
  │  code on the other device.  │
  │                             │
  │        [ Confirm ]          │
  └─────────────────────────────┘
```

Back on the desktop:

```
  pixel joined.  Safety code: 4821-9903
  2 devices in this space.
```

The invite ticket is now spent. Anyone who photographed that QR gets nothing.

## 3. Laptop — join by pasting

The MacBook has a camera, but aiming a laptop at another screen is awkward. So
paste instead — same payload, different presentation:

```
$ tts invite --print-only
  tts://join/BXn4Tp8...kQ2m
  Expires in 5:00.  Waiting...
```

On the laptop, after installing:

```
$ tts join tts://join/BXn4Tp8...kQ2m
  Device name [laptop]:
  Safety code: 7734-2216  — confirm this matches the inviting device.
  Joined. 3 devices in this space.
```

Because `laptop` also installed the CLI, it can send too. Nothing extra to
configure — it learned about `desk` and `pixel` from the roster it received on
join.

## 4. iPhone — join from the phone in your hand

The desktop is asleep upstairs. It doesn't matter: **any member can invite.**

On the Pixel, open the app → **Devices** → **Invite a device**. It displays a
QR. Scan it with the iPhone.

```
  iphone joined.  4 devices in this space.
```

The desktop learns about the iPhone when it next wakes — the join record is
signed by the Pixel, which the desktop already trusts, so the iPhone is
admitted without the desktop ever having seen it. (See *Membership* in
`architecture.md`.)

## 5. Everything sees everything

```
$ tts devices

  NAME     PLATFORM   STATUS      VOICE            LAST SEEN
  desk     linux      online      piper/amy        now
  laptop   macos      online      Samantha         now
  pixel    android    online      en-us-x-tpf      12s ago
  iphone   ios        background  Samantha         4m ago

  4 devices · space created 8 minutes ago
```

Three joins. Four devices. No pairwise setup, no server, no accounts.

## 6. First real use

```
$ tts --to all "setup complete"

$ tts --to pixel,iphone "build finished"

$ cat CHANGELOG.md | tts --strip --to laptop
```

---

## What the user should notice

**Connections are direct when they can be.** All four on the same wifi means
mDNS discovery and direct sockets — nothing leaves the house.

**Leaving the house changes nothing.** The Pixel on cellular is still
reachable. Its signed address record updates automatically; the desktop
resolves it by NodeId. No re-joining, no reconfiguration. This is the payoff
for pairing once against a public key rather than an address.

**`iphone` says `background`, not `online`.** That's the iOS restriction
surfacing honestly in the UI rather than as a mysterious failure. A message
sent to a backgrounded iPhone reports `unreachable`, and the agent sees it —
see *Feedback* in `cli.md`.

## Adding a fifth device later

Same one step, from any device:

```
$ tts invite
```

N devices requires N-1 joins. It does not grow quadratically.

## Removing a device

```
$ tts revoke pixel
  pixel removed from the space. Revocation gossiped to 3 devices.
```

The revocation propagates as members reconnect. **It is eventually
consistent**: a peer that has been offline since the revocation was issued may
still accept the revoked device until it syncs. Acceptable for a personal
device mesh; it would not be for a multi-tenant system.

## Adding a second space

Some devices belong in more than one group — a personal phone that should hear
both work and home, while the work laptop hears only work.

From the phone (or any device already in a space):

```
$ tts space new home
  Created 'home'. This device is a member of 2 spaces.

$ tts space list
  NAME    DEVICES   ROLE      DEFAULT
  work    3         member    *
  home    1         founder
```

Then invite into it exactly as before — invites are per-space:

```
$ tts invite --space home
```

The two spaces never learn about each other. Devices in `work` cannot see or
address devices in `home`, and each space carries its own mute, quiet hours,
and volume on every device that belongs to both.

To detach a device entirely rather than adding:

```
$ tts space leave work
  Left 'work'. Roster removed locally; 2 remaining members notified.
```

This works offline — the local roster is dropped immediately, and the
notification reaches the others when they next connect.
