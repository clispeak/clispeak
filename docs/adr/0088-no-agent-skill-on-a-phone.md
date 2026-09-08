# 88. No agent skill on a phone

**Status:** Accepted.

The Settings tab offered "Install the skill" on Android and iOS, and pressing
it would have worked. `BaseDirs` answers with a home directory inside the
app's own sandbox, so the default path looked plausible, the write succeeded,
and the badge went green over a file that nothing on that device will ever
open — there is no agent on a phone, and no filesystem an agent could reach
if there were (#134).

**The decision: `skill_status` returns `None` on mobile, which hides the
section, and `install_skill` refuses there as well.** Both halves, because a
hidden control is not a closed door: `skill-destination` is recorded in the
config directory and travels with the rest of a device's state, so a path
written by a desktop install can arrive on a phone and give the command
something plausible to write to.

The refusal says what the skill is for rather than that the button is
unavailable, which is the difference between an error an agent can act on and
one it will retry.

**Costs.** Two `cfg!(mobile)` checks in the Tauri shell, which is where
platform divergence is allowed to live. Neither is reachable from a Linux
test run, so this is verified by reading and by the phone not offering it —
say "build-verified" until someone opens the Settings tab on a real device.
