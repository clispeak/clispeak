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
converts instead; `--raw` skips validation. See `text.md`.

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
is a per-device toggle, **off by default**, and set on the receiver. Otherwise
"urgent" becomes meaningless the first time an agent marks everything urgent.

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
    { "device": "desk",   "status": "spoken",      "duration_ms": 3210 },
    { "device": "pixel",  "status": "spoken",      "duration_ms": 3140 },
    { "device": "iphone", "status": "unreachable", "reason": "background" }
  ]
}
```

### Per-target status

`queued` · `speaking` · `spoken` · `muted` · `quiet_hours` · `no_engine` ·
`unreachable` · `rejected` · `cancelled` · `dropped`

`no_engine` matters more than it looks: a Linux receiver whose Piper voice
model hasn't downloaded yet cannot speak. It must say so rather than silently
discarding the message.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Accepted — or, with `--wait`, spoken everywhere |
| `1` | Usage or configuration error |
| `2` | No targets matched the selector |
| `3` | Partial — at least one target succeeded, at least one didn't |
| `4` | Every target failed |
| `5` | Local node unavailable (could not reach or start it) |
| `6` | Text rejected — markdown or a URL that won't read well aloud |

## Devices and membership

```
$ voicecast devices                    # names, platform, status, voice, last seen
$ voicecast invite                     # QR + ticket to add a device
$ voicecast invite --print-only        # ticket only, for pasting over SSH
$ voicecast join <ticket>              # join a space from this device
$ voicecast revoke <name>
$ voicecast rename <new>                 # this device's own label
$ voicecast status                     # this node: identity, connections, queue
$ voicecast init                       # first run: identity + new space
```

## Spaces

A device can belong to several spaces at once, kept fully separate.

```
$ voicecast space list
  NAME    DEVICES   ROLE      DEFAULT
  work    3         member    *
  home    4         founder

$ voicecast space new home           # found a new space from this device
$ voicecast space leave work         # drop the roster + gossip a self-revocation
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
its own app UI:

- Display name
- Voice and engine
- Volume
- Mute (indefinite, manual)
- Quiet hours, and whether `high` may break through (default: no)
- **Per space**: separate mute, quiet hours, volume, and optionally voice
- Voice model download, with fallback state and reason shown when degraded
- Space membership; revoke other devices

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
