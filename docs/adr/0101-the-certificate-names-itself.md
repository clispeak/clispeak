# 101. The certificate names itself

**Status:** Accepted.

The first real run of the split signing job failed on `no identity found`,
having imported the certificate successfully one step earlier — the log says
`1 identity imported.` The `.p12` was good, the private key was present, the
keychain was right. `APPLE_SIGNING_IDENTITY` simply did not match what was in
it.

**That is a failure nobody can diagnose from the log**, and the reason is the
protection working as designed: the identity was stored as a secret, so every
line containing it is masked. `***: no identity found` reads identically
whether the name was mistyped, wrapped in quotes, padded with a newline, or
belonged to a different certificate.

**The decision: stop asking for it.** The signing job imports the certificate
and then asks the keychain which identity arrived —

```
security find-identity -v -p codesigning build.keychain
```

— requires **exactly one** `Developer ID Application`, and signs with its
SHA-1 hash rather than a display name. Exactly one for the reason the Android
job counts its APKs before reporting on one: a `.p12` holding two identities
would otherwise be signed with whichever sorted first, and a check that passes
about something other than what ships is worse than no check.

**Three things go away with the input.** A paste that cannot be verified, a
class of failure that cannot be read, and a secret that was never secret — a
certificate's common name is public in every artefact it signs. Making it a
secret bought nothing and cost the ability to debug.

**The general rule, which this project keeps rediscovering:** where a value is
derivable from something already present, deriving it is not merely more
convenient, it removes the possibility of the two disagreeing. This is the
same shape as decision 86 — a device is the authority on its own name — and
the same shape as reading a signature back off an artefact instead of trusting
the variable that was supposed to produce it.

**Costs.** A `.p12` carrying more than one Developer ID now fails rather than
guessing, which is the intended trade. And this is still unverified: the run
that produced the error is the only evidence, and the fix has not itself been
through a signing job yet.
