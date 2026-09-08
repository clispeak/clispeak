# 87. An error can have lines, because the values inside it are escaped

**Status:** Accepted.

Decision 38 escapes peer text where it is printed. The CLI applied that to the
whole of `Response::Error.message`, and a newline is a control character — so
an error deliberately written across four lines arrived as one line with
literal `\n` in the middle of it:

```console
$ clispeak --to Twin "hello"
error: more than one device is called 'Twin' in the same space\n  17a756fe…  in main\n…
```

It has looked like this since the message was added and needs two devices
sharing a name in one space to trigger, which is why nobody saw it (#135). It
matters more than the one instance suggests: `CLAUDE.md` says a rejected
message "shows the offending span and a rewrite that can be sent verbatim",
which is a multi-line error by construction.

**The escaping was never the bug. Applying it to the finished message was.**
`Error.message` is our own prose with values interpolated into it, and only
some of those values come from somewhere else.

**The decision: escape the values where they are interpolated, and print the
message with its lines.** `plain` moves into `clispeak-text`, which both the
CLI and the node already depend on, and gains `plain_lines` — the same
function with `\n` passed through, used for `Error.message` and nothing else.

Three things had to move together, or the escaping doubles up or disappears:

- **`clispeak-text` owns both**, so there is one implementation and one set of
  tests for it. The CLI keeps applying `plain` to every peer-supplied *field*
  it prints — names, labels, `detail`, history entries — which is unchanged.
- **The node escapes what it interpolates.** Two values in an error message
  come from another device: the suggestion list in "no device named X. Known:
  …", which is roster names, and a remote `JoinRefused` reason, which is
  free-form text a peer wrote. Both now go through `plain` at the point they
  are interpolated.
- **A control character is refused where a name is chosen**, rather than
  escaped later: in `name_objection`, in `Spaces::set_label`, and in
  `pick_space_label`, which is the one path where a peer's string becomes one
  of this device's own labels without passing through `set_label`.

**This does not reopen decision 38's "at the print, not at ingest".** Nothing
is sanitised on the way into a roster: a peer's name is stored exactly as sent,
still travels unchanged, and `--json` still disagrees with nothing. The node
writing an error message *is* a print, and it is now the innermost one.

**What the payoff looks like.** `resolve`'s message reads as four lines again,
and `revoke`'s sibling — written as one line, deliberately, rather than ship a
third instance of a known display bug — is back across lines with one id per
line, because the next thing anyone does with one is paste it back.

**Costs.** One function that must not be used on a peer's string, which is
said in its own doc comment and is the reason the refusals above exist: a
missed escape is then a missing belt rather than a missing pair of braces. The
app is unaffected either way — it puts the message in `textContent`, where a
newline collapses to a space, so it reads as one sentence rather than showing
a literal `\n`.
