# 70. A Developer ID certificate closes #29, and `bundle` says how it signed

**Status:** Accepted.

**Chosen:** `docs/signing.md` records the Developer ID route as measured
rather than expected, and `cargo xtask bundle` prints which identity it signed
with before it builds.

**#29's development half is closed, with evidence.** Three signing schemes,
the same test each time — install a genuinely different binary and see whether
the keychain asks:

| signing | designated requirement | rebuild |
|---|---|---|
| ad-hoc | `cdhash H"24a1051…"` | prompts |
| self-signed | `identifier … and certificate leaf = H"…"` | prompts |
| Developer ID | `identifier … and anchor apple generic and certificate leaf[subject.OU] = …` | silent, twice |

Each `CDHash` was checked different from the installed one before testing, so
no run could pass vacuously. Decision 67 recorded the middle row as a
failure with an untested explanation; the explanation is now confirmed in
macOS's own words. With both certificates in one keychain,
`security find-identity` prints `(CSSMERR_TP_NOT_TRUSTED)` beside the
self-signed one and nothing beside the Developer ID one. A keychain ACL needs
a trusted anchor, and `anchor apple generic` is it.

**`bundle` now says how it signed, because it silently did not.** Halfway
through that test a rebuild came out ad-hoc: `~/.zprofile` is read by login
shells and the shell running the build predated the `export`. The command
printed nothing either way, and the next step would have been to install it,
see a prompt, and conclude that Developer ID does not fix #29 — from a test
that never signed anything.

That is the day's shape once more, and this time inside the test rather than
inside the thing being tested. So the first line of a bundle is now either
`signing  as <identity>` or four lines saying it is ad-hoc, why, and what that
costs. The same reasoning as the release job reading its own signature back:
a build that quietly did the other thing is indistinguishable from success.

**Costs.** Two lines of output on every bundle, and one more place that has to
stay in step with the variable's name. The check is that the variable is set
and non-empty, not that the identity exists — `codesign` fails loudly on a
name it cannot find, so duplicating that here would be a second answer to a
question already answered well.

**Not covered.** Whether CI should hold the certificate at all is #117, and
it is Patrick's decision rather than a technical one. Notarisation is still
unwired, so `spctl` reports `Unnotarized Developer ID` on the signed build —
Gatekeeper recognising the developer and having seen no Apple ticket, which
is exactly true.
