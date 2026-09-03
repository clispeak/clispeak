# CLI surface

> Status: draft. The `voicecast` binary is a thin client to the local node — it
> writes to an IPC socket and exits. See `architecture.md`.

## Design principles

**Fast by default.** An agent may fire many of these. The default path accepts
the message locally and returns in single-digit milliseconds. Confirmation is
opt-in, not the default.

**Machine-readable on request.** The primary caller is an agent, so `--json`
and meaningful exit codes are first-class, not an afterthought.

**The sender expresses intent; the receiver enforces policy.** A sender can say
"this is urgent." It cannot override a device's mute, quiet hours, or volume.
Whatever the receiver decided is reported back honestly.

---

## Speaking

```
voicecast [OPTIONS] [TEXT...]
voicecast say [OPTIONS] [TEXT...]
```

```
$ voicecast "build finished"
$ voicecast --to pixel "needs your input"
$ voicecast --to pixel,laptop "deploy is live"
$ voicecast --to all "coffee"
$ echo "hello" | voicecast
$ cat CHANGELOG.md | voicecast --strip --to laptop
$ voicecast -f notes.md --to desk
```

Text comes from arguments, or stdin when arguments are absent. Both are the
same code path — long-form and short-form are not distinct modes.

**Text is validated before sending.** Markdown and bare URLs are rejected with
an explanation the agent can act on, rather than silently rewritten. `--strip`
converts instead; `--raw` skips validation. See `text.md`. When stripping would
not change anything — the text is not markup, it merely resembles it — the
error says so rather than offering `--strip` as advice that cannot work.

**Flags go before the text.** Speech legitimately starts with a hyphen, so the
text argument takes everything after it, a real flag included:
`voicecast hello --to Phone` speaks nothing and fails, because "--to" and
"Phone" were read as words. The error names the flag and prints the corrected
command, which can be run as printed.

### The subcommand collision

`voicecast stop` is ambiguous: speak the word "stop", or stop playback? The rule:

**A first argument that exactly matches a subcommand name is treated as a
subcommand.** To speak such a word, be explicit:

```
$ voicecast say stop
$ voicecast -- stop
```

Multi-word text never collides. This only affects single bare words that
happen to be reserved.

## Targeting

| Selector | Meaning |
|---|---|
| `--to desk` | One device by name |
| `--to desk,pixel` | Several |
| `--to all` | Every device in the space |
| `--to here` | This machine only |
| `--to phones` | A group (sender-side config) |
| *(omitted)* | The configured default target |

Groups are purely local config — they never appear in the protocol:

```
$ voicecast group set phones pixel,iphone
$ voicecast group set loud desk,laptop
$ voicecast groups
```

## Options

| Flag | Default | Meaning |
|---|---|---|
| `-t, --to <sel>` | config default | Target selector |
| `-p, --priority <lvl>` | `normal` | `low` \| `normal` \| `high` |
| `-f, --file <path>` | — | Read text from a file |
| `-v, --voice <name>` | receiver's | Request a voice, if the receiver has it |
| `-w, --wait` | off | Block until every target reaches a terminal state |
| `--timeout <secs>` | estimated | With `--wait`. Overrides the estimate below |
| `--json` | off | Machine-readable result on stdout |
| `-q, --quiet` | off | Suppress normal output. Errors still go to stderr — a failure nobody can explain is worse than a quiet success |
| `--strip` | off | Convert markdown to speakable text instead of rejecting |
| `--raw` | off | Speak exactly as given; skip validation entirely |
| `--dry-run` | off | Resolve targets and print them; speak nothing |

## Priority and queue semantics

A device speaks one message at a time. Priority decides what happens when
another arrives.

| Priority | Behavior |
|---|---|
| `low` | Appended. **Dropped** if the queue is already deep, or during quiet hours. For chatter that isn't worth interrupting anything. |
| `normal` | Appended. Spoken in order. The default. |
| `high` | **Interrupts** the current message, speaks, then resumes the interrupted one from the start of the chunk it was cut off in. |

Resuming from the chunk boundary rather than the exact word is a direct
benefit of sentence-level chunking — you hear a clean sentence restart rather
than a fragment.

**`high` does not override receiver policy.** A muted device stays silent and
reports `muted`. Whether high-priority messages may break through quiet hours
is a toggle on the receiver, **off by default**. Otherwise "urgent" becomes
meaningless the first time an agent marks everything urgent.

## Control

```
$ voicecast stop                  # stop current message, clear queue, all targets
$ voicecast stop --to pixel       # just that device
$ voicecast stop --id m_8fk2p     # one specific message, wherever it is
$ voicecast skip                  # abandon current, continue with the queue
$ voicecast pause
$ voicecast resume
$ voicecast queue                 # what's pending, where
```

Every send returns a message ID, which is what `--id` addresses.

## Feedback

By default `voicecast` exits as soon as the local node accepts the message. This is
the fast path, and it tells you nothing about whether anything was actually
spoken.

```
$ voicecast --to pixel "done"
m_8fk2p
```

With `--wait`, it blocks for terminal state on every target:

```
$ voicecast --to all --wait "deploy finished"
  desk      spoken     3.2s
  laptop    spoken     3.4s
  pixel     spoken     3.1s
  iphone    unreachable  (app not in foreground)
$ echo $?
3
```

### How long `--wait` waits

There is no fixed limit. The receiving device estimates one from the text it
was given, its own speaking rate, and whatever is already queued in front of
it, then adds an allowance for starting up. The bound is at least 30 seconds
and at most an hour.

It is estimated rather than fixed because any constant is wrong at exactly one
length. The previous flat 120 seconds was fine until a 569-word message
arrived: that is around 148 seconds of speech, so the device said all of it
while the caller was told it had not finished — see issue #6.

The estimate assumes a device speaks more slowly than any engine here actually
does, because it is an upper bound on waiting rather than a delay. The wait
ends the moment speaking does, so a generous estimate costs nothing; a mean
one reports a message as unfinished while it is being read aloud.

It is worked out on the **receiving** device, which is the only one that knows
its engine, its rate and its queue. `--timeout` is the caller overriding all of
that, and always wins — including for a remote device, which is sent the value.

If the bound is reached the message is reported as `speaking` rather than
`spoken`, with a detail saying so. That is not a failure: the device is still
talking, and the message lands in its history either way.

With `--json`, for agents:

```json
{
  "id": "m_8fk2p",
  "targets": [
    { "device": "desk",   "endpoint_id": "9f2c…", "status": "spoken",      "took_ms": 3210, "detail": null },
    { "device": "pixel",  "endpoint_id": "41ab…", "status": "spoken",      "took_ms": 3140, "detail": null },
    { "device": "iphone", "endpoint_id": "7c15…", "status": "unreachable", "took_ms": null, "detail": "background" }
  ]
}
```

`endpoint_id` is the full public key, not the elision shown here. It is in
every row because `device` is a label: local, freely chosen, and not unique.
Two devices can carry the same one, and this device's own name beats a peer
that shares it — so a report naming only the label can say `spoken` about a
device you did not mean. Match on `endpoint_id` and the row is unambiguous.

### When a name means more than one device

A bare name that matches devices in **different spaces** is refused; qualify
it as `work/laptop`. A name matching two devices in the **same** space is also
refused, and the error lists their ids, because qualifying cannot separate
them and the fix is to rename one on the device itself.

The case that is *not* refused is this device's own name. It wins outright —
your own machine is what you meant — but the peers it beat are reported:

```
$ voicecast --to laptop "deploying"
  laptop           spoken       (this device's own name was used; 1 other device
                                 also answers to it and was not sent to: a37d705ec0f61d92)
```

The send succeeded and the exit code is `0`. The detail is there because the
alternative is a one-row report that reads as a clean send while a second
machine you might have meant heard nothing. Whether that case should refuse
instead of reporting is issue #39; it would break any caller relying on
today's behaviour, so it is not decided here.

In the table, an id column appears on **every** row as soon as any two rows
share a device name, and on none otherwise:

```
  other            [b59aeb94436b9b53] spoken       3.7s
  twin             [a37d705ec0f61d92] spoken       2.2s
  twin             [ec9de4e234d041e3] spoken       1.9s
```

### Per-target status

`queued` · `speaking` · `spoken` · `muted` · `quiet_hours` · `no_engine` ·
`unreachable` · `rejected` · `cancelled` · `dropped`

`no_engine` matters more than it looks: a Linux receiver whose Piper voice
model hasn't downloaded yet cannot speak. It must say so rather than silently
discarding the message.

`rejected` is the receiver's decision rather than the sender's, so it carries a
reason: either this device is not in the space the message was sent in, or the
message was longer than the 100,000 characters it will speak in one go. Split
it and send the parts.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Accepted — or, with `--wait`, spoken everywhere |
| `1` | Usage or configuration error |
| `2` | No targets matched the selector |
| `3` | Partial — at least one target succeeded, at least one didn't |
| `4` | Every target failed |
| `5` | Local node unavailable — unreachable, stopped mid-request, or wedged and not answering |
| `6` | Text rejected — markdown or a URL that won't read well aloud |

## Devices and membership

```
$ voicecast devices                    # names, platform, status, voice, last seen
$ voicecast invite                     # QR + ticket to add a device
$ voicecast invite | pbcopy            # the ticket alone, for pasting over SSH
$ voicecast preview <ticket>           # what that invite would join
$ voicecast join <ticket>              # join a space from this device
$ voicecast join <ticket> --name home  # ...calling it something else here
$ voicecast invite --space work        # invite into a named space
$ voicecast revoke <name>
$ voicecast revoke <name> --space work
$ voicecast rename <new>                 # this device's own label
$ voicecast status                     # this node: identity, engine, queue
```

`status` names the engine, and when the engine cannot speak it says why on the
line below rather than leaving the reader to guess from a word:

```
engine:  unavailable  (fallback)
         Piper is not installed in any of: ~/Library/Application Support/voicecast,
         /app/share/voicecast, /usr/share/voicecast
```

The node has always known that sentence — it reaches whoever *sends* a message
as the reason for `no_engine` — but the status line, which is where somebody
looks first, used to report an engine that would never start as "starting…".

When the CLI cannot reach a node at all it says what it found and stops short
of saying why:

```
$ voicecast status
error: nothing is listening for voicecast

The node may not be running, or may not have finished starting.
On macOS it does not bind until the keychain prompt is answered,
which returns after every update — look for a dialog behind the app.

start one with: voicecastd, or open the voicecast app
```

Exit code 5 either way. The macOS lines appear only on macOS: naming a dialog
that cannot exist sends the reader hunting for somewhere that is not there.

There is no `init`. A node creates its identity and founds its own space the
first time it starts, so there is nothing for a separate command to do — and
one that did nothing would be worse than its absence.

`preview` exists because the joining device cannot choose what it joins. The
space is written into the ticket when the invite is minted, so `join` on its
own is a command whose effect you cannot see until it has happened:

```
$ voicecast preview voicecast://join/AE7Q...
joins 'work'
From 3332cac4fca203fa
Expires in 4m 12s. Single use.
Join it with:  voicecast join <the same code>
```

It is local. No device is contacted, the single-use token is not spent, and
the same errors `join` would raise — expired, truncated, not an invite —
arrive here instead, before anything is committed to. The app's Join dialog
is the same two steps.

By default the joined space takes the name the inviter uses for it; `--name`
overrides that. The label is local either way: it is how *this* device writes
`work/laptop`, and nothing about it is sent to anyone.

`invite` needs no `--print-only` either. The ticket goes to stdout on its own;
the expiry note and the instruction for the other device go to stderr. So
piping it somewhere already yields the bare code, which is what that flag was
for.

## Spaces

A device can belong to several spaces at once, kept fully separate.

```
$ voicecast space list
  NAME    DEVICES   ROLE      DEFAULT
  work    3         member    *
  home    4         founder

$ voicecast space new home           # found a new space from this device
$ voicecast leave --space work       # tell the others, then remove it here
                               # warns if you are the last member
$ voicecast space default home       # which space bare target names resolve in
$ voicecast space rename work team   # local label only
$ voicecast space rotate             # new space, re-invite survivors (see below)
                               # also spelled `voicecast rotate`
```

Leaving works offline: the local roster is dropped immediately, and the signed
tombstone reaches remaining members as they reconnect.

### Rotation — the panic button

Revocation is eventually consistent, so a device that has been offline since
the revoke will still honor the revoked member until it syncs. When that
window matters — a stolen phone rather than a sold laptop — rotate instead:

```
$ voicecast space rotate
  Created a replacement for 'home'.
  Re-invite surviving devices:  desk, laptop, ipad
  [ QR ]
```

Rotating also cancels any invite still open on this device. A ticket names a
space by id and lives for five minutes, so one shown just before the panic
button was pressed would otherwise still be scannable — and the space it named
would be gone, which used to admit the scanner to the replacement instead.

The excluded device is locked out *immediately* rather than eventually,
because it was never in the new space. This is only practical because joining
is cheap — three survivors is two scans.

### Targeting across spaces

Bare names resolve in the **default space**. Qualify with `space/device` to
reach anywhere else:

```
$ voicecast --to pixel "..."            # default space
$ voicecast --to work/laptop "..."      # explicit
$ voicecast --to work/all "..."         # every device in one space
$ voicecast --to work/all,home/all      # both, spelled out
```

A bare name resolves in the default space when it exists there, and otherwise
anywhere it is unique. Ambiguity is an error, never a guess:

```
$ voicecast --to laptop "..."
  error: 'laptop' exists in 2 spaces (work, home)
         qualify it:  work/laptop  or  home/laptop
```

**There is deliberately no selector meaning "every device in every space."**
`--to all` is scoped to one space. Crossing spaces requires naming them,
because the failure it prevents — a work message arriving on the family
tablet — is exactly what separate spaces are for.

Groups expand one level: a group naming another group is left alone rather
than followed. Nesting buys little and a cycle would hang the tool.

Groups may span spaces, since they're local config:

```toml
[groups]
phones = ["home/pixel", "work/iphone"]
```

## History

Every message a device is asked to speak is recorded, **whether or not it was
spoken**. A message refused while the device was muted or in quiet hours is
the one worth keeping: it is the only record that it came at all.

```
$ voicecast history                    # recent messages, newest first
$ voicecast history --unheard          # only the ones never spoken
$ voicecast history -n 100
$ voicecast history --clear
$ voicecast replay <msg-id>            # speak it again, here
```

`replay` plays **through** mute and quiet hours. Those settings exist to stop
a device making noise unasked; pressing play is the ask. A replayed message
keeps its id, so one that was never heard is marked heard once played.

The app shows the same list, with the text in full and a play button per
message.

## Receiver-side settings

Not reachable from the CLI of *another* device — these live on each device, in
its own app UI, and in a handful of local commands:

- Display name
- Voice and engine
- Mute (indefinite, manual)
- Quiet hours, and whether `high` may break through (default: no)
- **Per space**: separate mute and quiet hours
- Voice model download, with fallback state and reason shown when degraded
- Space membership; revoke other devices

Volume is not implemented at any level yet.

### Mute and quiet hours

```
$ voicecast mute                       # this device: silent until unmuted
$ voicecast unmute
$ voicecast quiet 22:00-07:00          # this device, every day
$ voicecast quiet 22:00-07:00 --high   # let urgent messages through
$ voicecast quiet off
$ voicecast quiet                      # show what is set, changing nothing
```

Each takes `--space` to act on one space instead of the whole device:

```
$ voicecast mute --space work          # work goes quiet here; nothing else does
$ voicecast quiet 18:00-09:00 --space work
$ voicecast unmute --space work
```

Note the asymmetry with `revoke`, `leave` and `rotate`, where omitting
`--space` means *the default space*. Here it means **the device**, because the
device-wide switch is the one most people ever touch.

**A space can be quieter than its device, never louder.** Both policies are
consulted and either can refuse, so muting the device silences every space, and
a per-space setting only ever adds silence. This means there is no way to hear
one space while the device is muted — `high` is the mechanism for that, and
`docs/decisions.md` #29 records why the trade went this way.

To have work go quiet in the evening while home still speaks, leave the
device's own quiet hours off and set a window on `work` alone.

**Text spoken locally is governed by the device policy only.** Speech this
device originates is not *in* a space, so muting `work` does not silence an
agent running here; muting the device does.

`voicecast quiet` with nothing set prints the device policy first, then one
block per space that restricts something:

```
muted:   no
quiet:   off

work (on top of the above)
  quiet:   18:00-09:00
```

## Config

`~/.config/voicecast/` (XDG; platform equivalents elsewhere).

```toml
# config.toml
default_space  = "home"
default_target = "here"
default_priority = "normal"

[groups]
phones = ["pixel", "iphone"]
loud   = ["desk", "laptop"]
```

Identity lives in the system keyring, not here. The device roster is managed
by the node, not hand-edited.

## Agent usage

The shape this is all built around:

```bash
voicecast --to phones "Claude needs your input on the migration plan"
```

Fire-and-forget, a few milliseconds, no blocking. When the agent needs to know
it landed:

```bash
voicecast --to phones --wait --json "deploy finished" | jq -r '.targets[].status'
```
