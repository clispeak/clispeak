# 33. Kotlin reached over JNI is kept from R8 by a rule, and the rule is gated

**Status:** Accepted.

**Decision.** `proguard-rules.pro` keeps `Speech`, `Invites` and `Battery`
outright, and `cargo xtask portability` fails if `voicecast-engine` names a
`com/voicecast/…` class that has no keep rule.

**What was wrong.** The release APK died the moment it launched:

```
java.lang.NoSuchMethodError: no static method
"Lcom/voicecast/app/Speech;.setVoice(Ljava/lang/String;)Z"
```

R8 deletes what it cannot see called, and the engine calls its Kotlin by name
— `find_class("com/voicecast/app/Speech")` — where the only caller is a string
inside a `.so`. Nothing in Kotlin or Java refers to those classes at all.

**Why it survived every gate.** `isMinifyEnabled` is true for `release` and
false for `debug`, and every test this project had run on a real phone was a
debug build. The fault existed only in the artefact built for other people,
and it reached one the same afternoon the release workflow first produced a
downloadable APK. This is not a platform difference — it is a difference
between two builds of the same platform — and it hid the same way.

**Why a gate rather than a note.** The failure mode is adding a fourth class
and learning about it from a crash report; nothing in the compiler, the tests
or CI has any opinion. The check extracts the string literals the engine
actually uses, so it cannot drift from the code it protects. It was verified
by deleting the `Speech` rule and watching it name that line — a gate nobody
has seen fail is a gate nobody should trust.

**What is still not covered.** Nothing verifies that the release APK
*launches*. The gate catches a missing keep rule, not the next thing R8 might
break, and not a native library that fails to load. A CI step that installs
the release APK on an emulator and waits for the node to answer would; it is
tracked in #41 and worth doing before anyone downloads this.

**Cost.** Four classes are exempt from shrinking, which is a few kilobytes,
and `-keep … { *; }` is broader than the members actually called — deliberately,
because the narrower form breaks silently the next time a signature changes.
