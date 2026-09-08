# 67. The self-signed certificate does not fix the keychain prompt

**Status:** Accepted.

**Chosen:** `docs/signing.md` stops recommending a self-signed certificate as
the free fix for #29, and says a Developer ID certificate is the answer for
both halves. The self-signed section stays, leading with the negative result.

This file said a self-signed Code Signing certificate ends the rebuild prompt
"at no cost", and decision 56 said the same. **It does not.** Measured on
macOS 26.5, on the machine the document exists for:

- `codesign` used the certificate — `Authority=voicecast dev`,
  `--verify --deep` reporting `satisfies its Designated Requirement`.
- The designated requirement changed from `cdhash H"24a1051…"` to
  `identifier "com.voicecast.app" and certificate leaf = H"a494cdd8…"` —
  exactly the stable shape the reasoning turned on.
- Three rebuilds, each with a genuinely different `CDHash`, each answered with
  *Always Allow*, each followed by another prompt. The process parked on
  `SecKeychainFindGenericPassword` → `ClientSession::decrypt`, the same stack
  as the original diagnosis.

**The reasoning was right and incomplete.** A stable designated requirement is
necessary and is not sufficient. The untested explanation is that a keychain
ACL needs a *trusted* anchor to name, and macOS does not trust a self-signed
root — the same fact that makes `find-identity -v` report zero, which this
file had already noticed and correctly dismissed as harmless *for signing*. It
was not harmless here. Untested, because the next step is a Developer ID
certificate, which has no such problem.

**Two wrong instructions were found by one person following the document
once**, and both were mine. The first told the reader that
`find-identity -v -p codesigning` printing zero meant the certificate type was
wrong; it means macOS does not trust a self-signed root, and the setup had in
fact succeeded. The second replaced it with an example of `find-identity`
output that I composed rather than ran — the real command prints a count and
no name at all, because it only lists valid identities. The corrected section
now carries captured output.

**What that says about the rest of this file.** Everything in it that was
*measured* held up — the `cdhash` versus certificate-leaf requirement, the
`codesign -d -r-` reading, quarantine propagating from a downloaded disk image
to the app dragged out of it. Everything *inferred* was wrong: the `-v` check,
its replacement's example output, and the central claim that the free
certificate fixes anything. The file was written by someone reasoning
carefully about a platform they could not run.

**Section 2's Developer ID example is marked unverified rather than left to
read as a transcript.** Nobody has run `security find-identity -v -p
codesigning` against a real Developer ID certificate; the line it promises is
an expectation. Reasoning says `-v` is right there — Apple's root is trusted,
so the certificate is valid as well as usable — and reasoning of exactly that
quality is what produced both errors above. The box comes out when someone
pastes in the literal output.

**Cost of the change.** Nobody gets a free fix for #29 any more, because there
is not one. An afternoon was spent finding that out, which is cheaper than
each future reader spending one.
