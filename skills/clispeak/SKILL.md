---
name: clispeak
hooks:
  UserPromptSubmit:
    - matcher: "*"
      hooks:
        - type: command
          command: clispeak prefs --brief
description: Speak messages aloud on the user's own devices — phone, laptop, desk — instead of writing to a terminal they may not be looking at. Use when the user has asked to be told something by voice, when a long task finishes and they have walked away, or when you need an answer and they are not at the screen. Also covers setting clispeak up, pairing devices, and how they want to be spoken to.
---

# clispeak

`clispeak` speaks text aloud on the user's devices. It is peer to peer, so
there is no server: each device runs a small app that is both sender and
receiver, and the `clispeak` command is a thin client for the one on this
machine.

**You are talking to a person who is probably not looking at a screen.** That
is the whole point of the tool and it should shape everything you send. Speech
cannot be skimmed, scrolled back, or re-read. It arrives whether or not it is
wanted.

## The first thing you do, every time

```bash
clispeak prefs
```

That prints how this person wants to be spoken to. **Read it rather than
recalling it.** The agreement lives in a file on this machine, shared by every
agent that runs here, so it is current even if you have never seen it, even if
this session was compacted an hour ago, and even if the last agent to talk to
them was a different one entirely.

Reading is idempotent and remembering is not: an agent that forgets to read
simply reads again, while an agent that forgets to *write something down* has
lost it permanently. Nothing here asks you to hold the agreement in your head.

**The first time this tool comes up in a conversation, do one more thing:**

- **If an agreement is recorded** — read it back in one sentence and ask
  whether it is still right. One exchange, then get on with what they asked
  for.
- **If nothing is recorded** — run `clispeak prefs setup` and work through the
  questions with them. It prints them in order with the command that records
  each answer, and it knows which ones they have already answered.

```bash
clispeak prefs setup     # the questions, in order
clispeak prefs reset     # forget it all and start again, keeping this skill
```

**Once, not before every message.** The questions belong at the moment
this first comes up, which is already an engagement. They do not belong in
front of every first message — that is a form standing between someone and the
thing they asked for, and it is why this skill used to get skipped.

## Recording what they tell you

When they state a preference — "stop telling me about builds", "always use my
phone", "keep it short" — write it down **in the same turn**, then say what
you recorded in one sentence.

```bash
clispeak prefs add speak-when "the nightly build breaks"
clispeak prefs remove speak-when 3      # by the number `prefs` showed
clispeak prefs set output brief
clispeak prefs set address-me-as Patrick
clispeak prefs set speak-as "Clispeak Lead"
clispeak prefs set speak-to Phone
clispeak prefs set fallback-to all
```

**The read-back is not optional.** Nobody opens this file — you are the only
way they ever learn what is stored. A rule you got wrong and said out loud is
corrected in the same breath; one you got wrong silently is invisible and
permanent.

**Only durable preferences.** "That was annoying" might mean *never again* or
might mean *not right now*, and recording the standing version of a passing
remark makes the tool quietly do less for a reason nobody can see. When it is
ambiguous, ask. One short question is cheaper than a wrong rule.

`clispeak prefs` shows who added each rule and when, so "why does it keep
telling me about that?" has an answer, and "take that off" is the next command.

## Output: where your answer goes

`clispeak prefs` reports one of four modes. It decides how you split a response
between the terminal and the voice.

| Mode | Terminal | Voice |
|---|---|---|
| `terminal` | everything | nothing — speaking is refused |
| `brief` | everything | a short summary, about forty words |
| `full` | everything | the same text |
| `speech` | keep it minimal | everything |

Under `terminal` and `brief`, the `speak_when` rules decide whether a moment is
worth a message at all. Under `speech` they do not — "say everything" is the
instruction, and filtering on top of it would silently drop things.

## Which device, and what to do when it is not there

`clispeak prefs` reports **Speak to** — the device a message goes to when you
do not pass `--to`. It is the most consequential line in the agreement: the
others shape a message, this one decides whether it is heard at all.

It may also report a fallback, for when that device is `unreachable`. **The
tool does not act on it and you must.** Whether an unreachable phone is worth
chasing to every other device is a judgement about *this* message, not a
property of the failure — a rule that broadcast to `all` every time a phone was
off would be loudest exactly when nobody is there. Decide, then say what you
did.

**Only for `unreachable`.** `muted` and quiet hours are decisions they made;
routing around those defeats the setting, and the message is in that device's
history to read later.

## Say who you are

The user may have several agents that can reach the same devices. A voice from
a pocket that does not say whose it is forces them to guess.

`clispeak prefs` reports **Call yourself** — use that name, not one you picked.
They chose it so they can tell one voice from another.

```
clispeak --to Phone "Patrick, this is Claude. The deploy finished and the
smoke tests passed."
```

Where a name has been recorded, **the tool enforces this**: a message that does
not open by naming them is refused with a corrected line containing both
names, which you send exactly as given. Under `speech` it is not required, because constant messages make it a
tic rather than a courtesy.

Do not drop it because you spoke a minute ago. Each message arrives on its own.

## Speaking

```bash
clispeak "Patrick, this is Claude. The build finished."     # this machine
clispeak --to Phone "..."                                   # one device
clispeak --to Phone,Laptop "..."                            # several
clispeak --to all "..."                                     # everything in the space
clispeak --to phones "..."                                  # a group they defined
clispeak --to work/laptop "..."                             # a device in a named space
clispeak --to work/all "..."                                # a whole space
```

**A bare name resolves in the default space.** If the same name exists in two
others, the error asks you to qualify it as `space/device` and names the
spaces. If two devices share a name inside *one* space, qualifying cannot
separate them and the error says so — one of them has to be renamed on the
device itself.

| Flag | Use it when |
|---|---|
| `--wait` | You need to know it was heard, not merely accepted. Blocks. |
| `--priority high` | It should interrupt what is playing. Sparingly. |
| `--priority low` | Chatter. Dropped if a queue has built up. |
| `--json` | You are going to branch on the result. Implies `--wait`. |
| `--file` | The text is long and already in a file. |
| `--dry-run` | Check where a message *would* go without sending it. |

`--priority high` interrupts and the interrupted message then resumes. It does
**not** override mute, and overrides quiet hours only where that device allows
it. Mark something urgent when it is urgent to *them*, not when it is the end
of your task. An agent that marks everything urgent makes the setting
meaningless and it gets turned off.

**Put flags before the text.** Speech can start with a hyphen — "- item one",
"-5 degrees" — so everything after the text is read as more text, including a
real flag. The error names the flag and prints the corrected command; run it
as given.

## What the exit code is telling you

Branch on it. The codes are distinct precisely so you can.

| Code | Meaning | What to do |
|---|---|---|
| `0` | Accepted, or spoken if you waited | Nothing |
| `6` | Text refused — markdown, a bare URL, or a missing opener | **The error contains a rewrite. Send it verbatim.** Do not compose your own |
| `4` | Nothing spoke it | Read the reason first — see below |
| `3` | Some devices spoke, some did not | Say which did not. Do not resend to everyone |
| `2` | The selector matched no device | The command was fine, the name was not. `clispeak devices` |
| `5` | No node running, or it wedged | Ask them to open the clispeak app. Do not retry |
| `1` | Usage error | Fix the command; the error often prints the fixed line |

**Exit 4 is usually not a failure.** Read the status:

- `muted`, `quiet hours`, or an agreement set to `terminal` — it arrived and
  the device, or the person, chose silence. **Do not retry and do not route to
  another device.** It is in that device's history for them to read.
- `unreachable` — off or offline. Worth mentioning; worth another device if it
  matters.
- `no engine` — that device cannot speak at all. Tell them; it needs fixing
  there.

## Writing text that reads well aloud

The tool refuses markdown and bare URLs rather than mangling them, because a
listener hears asterisks and slashes.

- Plain sentences. No bullets, headings or code fences.
- Say what a URL is — "the pull request page" — not the address.
- Numbers, file names and short identifiers are fine.
- `--strip` converts marked-up text; `--raw` skips every check including the
  agreement. Prefer writing it properly: `--raw` is a decision, and it shows.

A `rejected` *status* is different from exit 6 — it comes from the receiving
device, and says either that you are not in the space you sent to, or that the
message was over 100,000 characters. Split a long one.

## Pairing a device

This is real setup with steps, unlike the agreement above.

```bash
clispeak status      # device id, engine, whether it is muted
clispeak devices     # who is in this space
```

On one device `clispeak invite` prints a ticket and the app shows a QR code.
On the other:

```bash
clispeak preview <ticket>   # what it would join, without joining
clispeak join <ticket>      # ...and join it
```

**Always `preview` before `join`.** Which space a ticket joins was decided by
whoever minted it, so `join` alone has an effect neither of you can see
beforehand. `preview` contacts nobody, spends nothing, and reports the same
failures. Read the space name back before joining — a space joined by mistake
has to be left on both devices.

`join --name` calls it something else here. Invites are single use and last
five minutes; if one is refused saying no invite is open, it was used, expired,
or the inviting app restarted.

**Nothing being heard?** In order: `clispeak status` for `muted` or a quiet
window; `clispeak devices` to check it is still in the space; `clispeak
history` on the receiving device, which records every message whether or not it
was spoken.

**Playback.** `stop`, `skip`, `pause` and `resume` take `--to`. `queue`,
`status`, `history`, `mute` and `unmute` do not — they are about this device,
and passing `--to` is refused rather than ignored.

## Keeping yourself current

```bash
clispeak skill --check     # does the installed skill match this build?
clispeak skill --install   # write it
```

Run the check before helping with setup. A skill installed months ago
describes a tool that has moved, and it is confident while doing so — offer to
update it first.

**The hook comes with this file.** Its frontmatter declares a
`UserPromptSubmit` hook running `clispeak prefs --brief`, registered when this
skill is invoked and running for the rest of the session. That is what puts
the agreement back in front of you on every turn, including after this
conversation has been compacted and the text you read earlier is gone.

Nothing to install separately and nothing of theirs to edit — which is the
point. A hook written into their agent's settings file would outlive the skill
and go on running a command they had removed.

## What is not yours to run

`revoke`, `rotate`, `leave` and `quit` change who can reach whom, or stop the
node. They are the user's decisions. Explain them, offer the command, and let
them run it.

## Undoing all of this

```bash
clispeak forget                # remove the skill, and the hook it carries
clispeak forget --everything   # ...and the agreement too
```

The agreement is kept by default because other agents on this machine share
it. **Whatever you wrote in your own notes, you must delete yourself** — no
tool can reach inside your memory, and this one will not pretend to. Then tell
them what you removed, including which of your own notes went.

## Staying out of the way

The failure worth avoiding is not a missed message. It is being muted, after
which every later message is missed too.

- Speak because they need it away from the screen, not to mark your progress.
- One message, not three. Gather the outcome and say it once.
- Say the answer, not where to find it. "The tests failed on the auth module"
  is useful; "check the terminal" wastes the trip.
- If they are at the keyboard talking to you, write — do not speak.
