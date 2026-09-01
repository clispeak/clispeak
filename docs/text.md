# Text handling

How text gets from the CLI to a speech engine. Three stages, in order:

```
  validate  ->  protect  ->  split
  (reject)      (mask)       (chunk)
```

Validation happens **in the CLI, before anything is sent**. It costs
microseconds and fails without touching the network.

---

## 1. Validation

The tool is strict by default: text that will not read well aloud is
**rejected with an explanation**, not silently rewritten.

**Why strict rather than lenient.** The caller is an agent, and an agent can
act on an error message — read it, fix the text, retry. That makes validation
a working feedback channel rather than an obstacle. It also produces better
output than any amount of stripping: text an agent *wrote to be spoken* beats
markdown we cleaned up afterward.

### The line: error only when there is an obvious correction

| Input | Behavior | Why |
|---|---|---|
| `**bold**`, `` `code` ``, `# heading`, `[a](b)`, fences, tables, list markers | **error** | The agent can obviously write plain prose instead. |
| Bare URLs — `https://…`, `www.…` | **error** | Reading a URL aloud is unbearable; "the deploy dashboard" is the obvious correction. |
| Emoji | **stripped silently** | There is no correct way to write 🎉 for speech. An error would be a puzzle with no answer. |
| Repeated whitespace | **collapsed silently** | No meaningful alternative to suggest. |

That distinction is the whole rule: **error when the agent could have written
something better, handle it quietly when it could not.**

### The error format

The error message *is* the mechanism, so it carries the offending spans, a
concrete replacement the agent can resend verbatim, and the escape hatch:

```
$ voicecast --to pixel "Updated **3 files**. See \`src/main.rs\` for details."

error: text contains markdown that will not read well aloud

  Updated **3 files**. See `src/main.rs` for details.
          ^^^^^^^^^^^^     ^^^^^^^^^^^^^
          bold emphasis    inline code

  Write text as it should be spoken:
    "Updated 3 files. See main dot rs for details."

  Or pass --strip to convert automatically.

exit: 6
```

Exit code **6** is distinct from `1` (usage) so an agent can tell "my text was
wrong" from "the device is offline" — different failures needing different
responses.

### Escape hatches

| Flag | Behavior |
|---|---|
| *(default)* | Strict. Reject markdown and URLs. |
| `--strip` | Convert markdown to speakable text, then speak. |
| `--raw` | Speak exactly what was given, asterisks and all. |

Strict applies to stdin too, which is why the walkthrough's
`cat CHANGELOG.md | voicecast` becomes `cat CHANGELOG.md | voicecast --strip`. A changelog
*is* markdown; the flag makes the conversion explicit rather than silent.

Messages composed in the GUI are human-authored and not validated.

---

## 2. Protection

Validation removes the markdown problem, so chunking is left with the harder
and quieter one: **sentence boundaries are not where periods are.**

```
Deploy to 10.0.0.1 finished.        ->  "Deploy to 10." / "0." / "0." / "1 finished."
See src/main.rs:42 for the fix.     ->  shredded
Dr. Chen approved v1.2.3 (e.g.).    ->  five fragments
```

Rather than an ever-growing regex, spans that must never be split are
**masked with placeholders** before splitting and restored after:

- File paths and identifiers — `src/main.rs:42`
- Decimals, versions, IP addresses — `3.14`, `v1.2.3`, `10.0.0.1`
- Known abbreviations — `Dr.`, `e.g.`, `i.e.`, `etc.`, `Inc.`, `No.`, `Fig.`
- Ellipses

The point of doing it this way is that it is **testable**. Every case above is
a unit test with an obvious expected output, rather than a regex nobody wants
to touch.

## 3. Splitting

Masked text is split with a UAX #29 segmenter (`unicode-segmentation`), then
unmasked.

**Fallback cascade** when no sentence boundary exists:

1. Sentence boundary
2. Clause boundary — comma, semicolon, dash
3. Hard cap around 200 characters, **never mid-word**

The opposite case needs no special handling: `voicecast "build finished"` has no
terminal punctuation and is simply one chunk.

## Who chunks

**The sender.** It has to anyway in order to stream before reading all of
stdin, and chunking once means all devices get identical boundaries rather
than five slightly different interpretations.

**The receiver may sub-split** a chunk that exceeds its engine's limit, but
never re-joins. Sender decides meaning; receiver handles engine mechanics.

## Why chunks matter beyond latency

Chunk boundaries are also **resume points**. When `--priority high` interrupts
a message, playback resumes at the start of the chunk it was cut off in — so
you hear a clean sentence restart rather than a fragment. Chunking quality is
audible.
