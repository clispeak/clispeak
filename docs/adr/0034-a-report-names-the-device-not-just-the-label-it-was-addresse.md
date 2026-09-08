# 34. A report names the device, not just the label it was addressed by

**Status:** Accepted.

`TargetResult` carried `device`, and `device` is a label: local, freely
chosen, and not unique. So the report could say `spoken` about a device the
caller did not mean and there was nothing in it to tell.

That is not hypothetical. #38 made every macOS and Windows device answer to
"this device", and the report that proved the bug read:

```
{ "device": "this device", "status": "spoken", "took_ms": 2068 }
```

Correct, useless, and indistinguishable from the report you get when it worked.
`endpoint_id` is now on every row, in full, always in `--json`.

**Why the id is not always in the table.** A short id on every row is noise on
the report that has one row and one device, which is nearly all of them. It
appears when two rows share a device name — and then on *every* row, not only
the colliding ones. A column that appears on some rows is not a column: the
first version indented `spoken` two positions on exactly the rows a reader is
comparing.

**Why the shadowed peers are reported rather than refused.** Three ways a name
can mean more than one device, and they are not alike:

- Different spaces — already refused, with a qualified rewrite that works.
- The same space — refused, but the message said `'twin' exists in 2 spaces
  (space-2, space-2)` and offered `space-2/twin or space-2/twin`: the count was
  wrong, the two options were one option, and it was the command that had just
  failed. An agent following that suggestion loops, and neither device was
  addressable by any selector. Now it names their ids and says qualifying
  cannot help.
- This device's own name — takes the local device *without consulting the
  roster*, which is why the earlier claim that `Roster::by_name` was
  responsible was wrong. This one still resolves the same way, because your own
  machine is what you meant. But it now reports the peers it beat.

Reporting rather than refusing keeps every existing caller working. Refusing is
the larger change, it breaks any agent script that relies on today's resolution,
and it is Patrick's call rather than ours — issue #39.

**Cost.** One more field on the wire, defaulted so an older node still parses.
`Target::Here` gained a payload, which cost the dedup its meaning until it was
fixed: `--to here,laptop` on a machine called `laptop` produced two values that
both meant this device, and comparing them whole would have made it speak
twice. `Here` now collapses on identity and merges the shadow lists. Four tests
hold that down, because it is the kind of regression that is invisible until
someone hears it.
