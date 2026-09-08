# 78. Revoke names one device, or refuses

**Status:** Accepted.

**Chosen:** `voicecast revoke` accepts a name or an endpoint id, refuses a
name that matches two devices, and lists the ids so one can be named. The
choosing is a pure function, so it is tested without standing up a node.

**#39 was already half fixed, and the remaining half was the dangerous one.**
`resolve` refuses an ambiguous name for speaking and for the playback
controls, and has for a while. `revoke` still went through `Roster::by_name`,
which returns the first current member that matches — so the one command whose
entire purpose is removing a device chose between candidates arbitrarily and
reported success.

**The case is ordinary, not exotic.** Re-pair a phone after a rebuild and the
old entry sits beside the new one, both answering to the same label. Revoking
"Phone" could remove the device you had just paired and tell you it worked.
Patrick was about an hour from doing exactly that when this was found.

**Accepting an id matters as much as refusing the ambiguity.** The obvious
escape — "rename one of them" — does not work, because the other half of a
name clash is usually the *dead* device and a dead device cannot be renamed.
Refusing alone would have replaced a silent wrong answer with no answer.

**A short id, because that is what `voicecast devices` prints.** A prefix that
still matches two devices says so rather than choosing, which is the same rule
one level down.

**The message is one line, and that is a workaround.** The CLI escapes control
characters in anything a node sends, newlines included, so a message written
across several lines arrives carrying a literal `\n`. The sibling error in
`resolve` has done this since it was written and is left alone deliberately —
it is the real example, and moving it without fixing the cause would hide the
evidence. Filed as #135.

**Costs.** A selector that is a name *or* an id is one more thing to explain,
and a device named after a key prefix would be ambiguous with nothing warning
about it — names win, so the name is what happens. And `revoke` still answers
with `Response::Renamed`, so its confirmation reads "renamed to removed
<id>", which is nonsense; left alone rather than adding a `Response` variant,
because an unknown enum variant fails a whole decode and that is a wire
decision.
