# 100. Leaving and rejoining beats rotating, and the recovery is the evidence

**Status:** Accepted.

Four devices were recovered from #166 on 4 September without rotating
anything. Recording how, because the first plan was rotation and it was the
heavier tool for the job.

**What the space looked like afterwards:** one space, `main`, four members,
**no tombstones anywhere**. The Mac spoke in 5.7s, the phone in 12.5s.

### Rotate was the wrong first instinct

`rotate` re-founds the space with a new id, which does sidestep every stale
tombstone — that is why it was reached for. What it also does is leave every
*other* device holding a dead copy of the old space beside the new one, to be
tidied up afterwards on each. And it needs a hand on the founding device,
which in this case nobody had.

**Leaving and rejoining the same space is cleaner**, and the reason is a
property of `do_join` rather than a lucky accident:

- `leave` replaces a device's only space with a fresh solo one, taking the old
  space *and its tombstones* with it;
- `do_join` then sees a space holding only this device — `current_is_unshared`
  — and **replaces** it rather than inserting alongside.

So each device ends with exactly one space and nothing to clean up. That was
predicted here as leftovers to forget later, which was wrong: the prediction
was carried over from the rotate plan and not re-derived when the plan changed.

**The rule this suggests:** rotate is for a device that must be *excluded* —
its own doc string says "for a device that was stolen rather than sold".
Recovering a space whose members are all still trusted is a different problem,
and leaving and rejoining is the tool for it.

### What was actually observed, which is less than was first claimed

Both tombstones cleared, and it is worth being exact about which code did it.
The laptop was rebuilt and relaunched fourteen minutes *before* decision 99
merged, so it was running 98 and not 99 throughout the recovery. Both
clearings therefore came from **98's merge cleanup**, on the first sync after
each device rejoined — the phone's within seconds, the Mac's only when
something spoke to it and forced a sync.

That was first written up as the two fixes being observed by different routes.
It was not: 99's invite-path clearing was never exercised, because the machine
doing the inviting did not have it. A pleasing symmetry is exactly the kind of
claim worth checking against which build was actually running.

**What it does demonstrate** is that 98's cleanup repairs a live roster rather
than only preventing a new one — the same self-healing the Mac saw on load —
and that a device carrying a spent tombstone is fixed by any sync at all.

### Not needed, and worth knowing

`clispeak devices` showed all four correctly *before* the last tombstone
cleared, because `is_current` compares dates and the rejoin was newer. The
contradiction was invisible from the tool and visible only in the file, which
is the same gap #166 asks to close.
