# 94. A `#[cfg]` is not a conditional, it is one spelling of one

**Status:** Accepted.

`cargo run -p xtask -- portability` reported **`3 crates clean`** while this
sat in `clispeak-core/src/identity.rs`, undeclared and uncounted:

```rust
if cfg!(target_os = "android") { "Android phone" }
else if cfg!(target_os = "ios") { "iPhone" }
else { "this device" }
```

The gate's predicate is careful — word boundaries, `not(...)`, `all(...)`,
`cfg_attr` — and it was never reached, because a cheap early return had
already said no:

```rust
if !line.contains("cfg(") && !line.contains("cfg_attr(") { return false; }
```

`cfg!(target_os = "android")` contains neither. The `!` sits between the name
and the paren (#161).

**Third time, same shape.** #88: it checked two crates of three and printed
"3 crates clean" while `cfg(unix)` sat in `clispeak-core`. #103: `check`
reported `all gates passed` on a tree holding a live `<<<<<<< HEAD`, because
nothing opened a `.md` file. Now this. Every one answered honestly about
something *adjacent* to the question asked — which is the sentence
`CLAUDE.md` uses about the failures it catalogues, and the gate meant to catch
them keeps failing the same way.

**The decision, in three parts.**

**The predicate sees `cfg!`.** One more spelling in a list, and the careful
part below starts doing its job.

**The conditional moves out rather than being excused.** Declaring an
exception was available and would have been honest — a plausible label for a
device whose hostname says nothing genuinely differs per platform. But the
rule says platform divergence belongs in `clispeak-engine` or the Tauri shell,
and this one could go: `device_name()` is called from exactly two places, the
daemon and the app, both platform layers already. So `device_name_or(fallback)`
takes the answer from whoever knows the platform, `device_name()` keeps the
one honest fallback this crate has — "this device" — and the `target_os` lives
in `app/src-tauri`, spelled `#[cfg]`, where it is allowed and visible.

**The gate gets its first tests.** It has been wrong three times and had none.
They are mostly cases that *must* be caught, since every failure so far was a
false negative, with a short list that must not be — a gate that cries wolf
gets switched off. The `cfg!` that hid for months is one of the rows.

**Costs.** A public function gained a sibling, and the daemon keeps calling
`device_name()` because a headless Linux box has nothing better to offer than
its hostname. And the gate is still a line-based scan of text rather than a
parse: it now knows three spellings of `cfg` and there may be a fourth. What
changed is that it is tested, so the fourth can be added to a list that proves
it stays.
