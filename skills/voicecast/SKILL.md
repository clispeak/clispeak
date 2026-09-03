---
name: voicecast
description: Speak messages aloud on the user's own devices — phone, laptop, desk — instead of writing to a terminal they may not be looking at. Use when the user has asked to be told something by voice, when a long task finishes and they have walked away, or when you need an answer and they are not at the screen. Also covers setting voicecast up, pairing devices, and agreeing with the user how and when they want to be spoken to.
---

# voicecast

`voicecast` speaks text aloud on the user's devices. It is peer to peer, so
there is no server: each device runs a small app that is both sender and
receiver, and the `voicecast` command is a thin client for the one on this
machine.

**You are talking to a person who is probably not looking at a screen.** That
is the whole point of the tool and it should shape everything you send. Speech
cannot be skimmed, scrolled back, or re-read. It arrives whether or not it is
wanted.

## Before anything else: say who you are

The user may have several agents that can reach the same devices. A voice from
a pocket that does not say whose it is forces them to guess.

**Every spoken message opens by naming the user and yourself:**

```
voicecast --to Phone "Patrick, this is Claude. The deploy finished and the
smoke tests passed."
```

Keep it to a few words — `"<Name>, this is <your name>."` — then the message.
Both names come from the working agreement below. If you have not established
one yet, set it up before you speak anything.

Do not drop the identification because you spoke a minute ago. Each message
arrives on its own with no context around it.

## The working agreement

Establish this **once**, in conversation with the user, then write it to your
memory. Until it exists you do not know enough to use this tool well, and
guessing produces exactly the noise that gets an agent muted.

Ask, in roughly this order, and keep it short:

1. **What should I call myself when I speak?** Their agents need distinct
   names. Suggest one if they have no preference.
2. **What should I call you?** Usually their first name.
3. **Which device by default?** Run `voicecast devices` first and offer the
   real names. A phone suits "you need to know now"; a desk machine suits
   "you will see this when you are back".
4. **When should I speak rather than print?** This is the important one, and
   worth pushing on. Reasonable starting points: a long task finishing, a
   question that blocks progress, something failing that they asked to be told
   about. Not: routine progress, things they can see on screen, anything they
   did not ask to hear.
5. **Anything I should never speak?** Secrets, customer names, anything they
   would not want said out loud in a room.

Then **write it to memory** so it survives this session. One memory, of the
kind that records how the user wants you to work, containing:

- the name you speak under, and the name you address them by
- the default device, and when to use others
- the agreed threshold for speaking rather than printing
- anything they said not to speak

Read it back to them in a sentence and let them correct it. Preferences stated
once and misremembered are worse than none.

**Revisit it when they push back.** If they say a message was unnecessary,
that is a change to the threshold, not a one-off — update the memory.

## Speaking

```bash
voicecast "Patrick, this is Claude. The build finished."          # this machine
voicecast --to Phone "..."                                        # one device
voicecast --to Phone,Laptop "..."                                 # several
voicecast --to all "..."                                          # every device in the space
voicecast --to phones "..."                                       # a group the user defined
```

Useful flags:

| Flag | Use it when |
|---|---|
| `--wait` | You need to know it was actually heard, not merely accepted. Blocks. |
| `--priority high` | It should interrupt whatever is playing. Use sparingly — see below. |
| `--priority low` | Chatter. Dropped if a queue has built up. |
| `--json` | You are going to branch on the result. Implies `--wait`. |
| `--file` | The text is long and already in a file. |
| `--dry-run` | Check where a message *would* go without sending it. |

`--priority high` interrupts what the device is saying and the interrupted
message then resumes. It does **not** override mute, and it overrides quiet
hours only if that device has been set to allow it. Mark something urgent when
it is urgent to *them*, not when it is the end of your task. An agent that
marks everything urgent makes the setting meaningless and it gets turned off.

Length is your judgement, not the tool's. It has no limit. A person listening
has one.

## What the exit code is telling you

Branch on it. The codes are distinct precisely so you can.

| Code | Meaning | What to do |
|---|---|---|
| `0` | Accepted, or spoken if you waited | Nothing |
| `6` | Text rejected — markdown or a bare URL | **The error contains a rewrite. Send that, verbatim.** Do not paraphrase it yourself |
| `4` | No device spoke it | Read the reason before reacting — see below |
| `3` | Some devices spoke, some did not | Say which ones did not. Do not resend to everyone |
| `5` | No node running on this machine | Ask the user to open the voicecast app. Do not retry |
| `1` | Usage error | Fix the command |

**Exit 4 is usually not a failure.** Check the status:

- `muted` or `quiet hours` — the message arrived and the device chose not to
  say it. **Do not retry, and do not route around it to another device.** The
  person deliberately made that device quiet. It is kept in the device's
  history for them to read later, so nothing is lost.
- `unreachable` — that device is off or offline. Worth mentioning, worth
  trying another device if it matters.
- `no engine` — that device cannot speak at all. Tell the user; it needs
  fixing on that device.

## Writing text that reads well aloud

The tool refuses markdown and bare URLs rather than mangling them, because a
listener hears asterisks and slashes.

- Write plain sentences. No bullet lists, no headings, no code fences.
- Spell out what a URL is instead of reading it: "the pull request page"
  rather than the address.
- Numbers, file names and short identifiers are fine.
- If you genuinely need to speak marked-up text, `--strip` converts it and
  `--raw` skips checking entirely. Prefer writing it properly.

When text is rejected you get the offending span and a suggested rewrite.
**Send the suggestion unchanged** — it is what the tool will accept.

## Setting up

**Is it working here?**

```bash
voicecast status     # device id, engine, whether it is muted
voicecast devices    # who is in this space
```

Exit 5 means no node is running on this machine: the user needs to open the
voicecast app, which is the node. On Linux it is a Flatpak, on macOS an app in
/Applications, on Android an installed app.

**Adding a device.** On one device:

```bash
voicecast invite
```

That prints a ticket and the app shows a QR code. On the other device:

```bash
voicecast preview <ticket>   # what it would join, without joining
voicecast join <ticket>      # ...and join it
```

**Always `preview` before `join` when helping someone set up.** Which space a
ticket joins was decided by whoever minted it, not by the device using it, so
`join` on its own is a command whose effect neither of you can see beforehand.
`preview` is local — it contacts nobody and does not spend the ticket — and it
reports the same failures `join` would: expired, truncated, not an invite. Read
the space name back to the user before joining; a space joined by mistake has
to be left on both devices.

Add `--name` to `join` if the user wants it called something else here. The
name is local to the device it is set on.

Invites are single use and last five minutes. If a join is refused saying no
invite is open, the ticket was used, expired, or the inviting app restarted —
ask for a fresh one.

**Nothing is being heard.** In order: `voicecast status` on that device for
`muted` or a quiet window; `voicecast devices` to check it is still in the
space; `voicecast history` on the receiving device, which records every
message whether or not it was spoken.

**Controlling playback.** `voicecast stop`, `skip`, `pause`, `resume` and
`queue` all take `--to`, so a device talking in another room can be quieted
from here.

## Staying out of the way

The failure worth avoiding is not a missed message. It is being muted, after
which every later message is missed too.

- Speak because the user needs it away from the screen, not to mark your own
  progress.
- One message, not three. Gather the outcome and say it once.
- Say the answer, not where to find it. "The tests failed on the auth module"
  is useful; "check the terminal" wastes the trip.
- If they are at the keyboard and talking to you, write — do not speak.
