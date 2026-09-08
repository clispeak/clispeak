# 61. The app's reply handling is one module, so a test can execute it

**Status:** Accepted.

**Chosen:** every Tauri command's `match` on `Response` moves into a `replies`
module of small pure functions, and one test drives a real `Node` through
those same functions, asserting only that the catch-all did not fire.

The node has changed what it returns three times and the app went on matching
the old shape each time (#46). Each compiled, each was green, each was found
by a person pressing a button — and the worst of them announced a failure
*after speaking the message*, which is the screenshot Patrick sent from his
phone.

**Why not narrow the return types.** That is the fix that would make this a
compile error, and it is right, but it means `Node`'s methods returning typed
values rather than `Response` — a change to the crate the CLI shares over the
wire, and the seam decision 79 will need. Doing it before there is any test
above the unit level means a large refactor whose only verification is
reading. The order is harness first.

**Why the assertion is "not `unexpected response`" rather than "Ok".** A
silent engine legitimately refuses to speak, and a command that surfaces the
refusal is doing its job. The failure being detected is narrower and exact:
`describe` fires only when the node returned a shape no arm names. Asserting
success would have made the test fail for the right reason and the wrong
cause, and would have needed a working speech engine on a CI runner.

**Falsified against two of the three historical drifts**, by reintroducing
them and watching the test fail:

```
speak:       unexpected response: Report { msg_id: "m_445814b0", targets: [ … ] }
leave_space: unexpected response: Left { space: "Three", unreached: 0, refounded: true }
```

The first is Patrick's screenshot, reproduced as a build failure.

**Costs.** The test binds a real iroh endpoint, so it is ~1.5s rather than
instant, and it needs a UDP socket. `set_config_dir` is a `OnceLock`, so this
has to stay *one* test in this process — a second would silently share the
first's directory rather than getting its own, which is why the reason is
written above the test and not only here. `join` is not covered: it dials a
peer, so without a second device it would wait on a network rather than
return a shape. And this catches drift at test time, not compile time; the
compiler still permits the mismatch.

**One thing worth repeating from #46's own text.** `app/src-tauri/src/lib.rs`
has a module gated `#[cfg(all(test, target_os = "macos"))]`. Tests there
compile on every target and run on none that CI tests, which is
indistinguishable from passing. This test is deliberately not in it.
