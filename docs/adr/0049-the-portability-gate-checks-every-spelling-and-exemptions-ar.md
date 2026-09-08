# 49. The portability gate checks every spelling, and exemptions are written down

**Status:** Accepted.

`cargo xtask portability` looked for `cfg(target_os)` and `cfg(target_family)`
and printed "3 crates clean". It had never looked for `cfg(unix)`, which was
sitting in `voicecast-core` the whole time, nor `cfg(windows)`,
`cfg(target_arch)`, `cfg(target_env)` or `cfg(target_pointer_width)`. A
`cfg(unix)` with no `cfg(windows)` arm compiles on four targets and fails on
the fifth, which is the exact shape of every row in CLAUDE.md's table of
divergence that has bitten this project. Issue #88.

**Matched as a predicate, not as a prefix.** The first version looked for the
literal text `cfg(unix`, which sees neither `cfg(not(unix))` nor
`cfg(all(unix, feature = "x"))` — the same claim, wrapped. It now looks for
the predicate inside any `cfg` or `cfg_attr`, with a word boundary so a line
mentioning a `windows` field is not a finding.

**Exemptions are declared where they are read.** Some divergence is
unavoidable: setting a file mode has no portable spelling, so `store.rs` needs
`cfg(unix)` whatever the rule says. The choice was between an allowlist in
`xtask`, which drifts from the code it names, and a marker comment above the
line. The marker wins because it sits where the reader already is, and it must
carry a reason — a bare exemption is the thing that gets pasted without
thought. The reason may run to several lines; the first version of the check
only read one, which is exactly the length a real reason turned out not to be.

**The count is printed.** "3 crates clean against 7 conditional forms (6
declared exceptions)" rather than "3 crates clean", because a gate that says
"clean" while meaning "clean apart from six" is how this check came to
overstate itself in the first place. The same reasoning as the JNI class count
in decision 33: a gate that passed and a gate that never ran should not print
the same sentence.
