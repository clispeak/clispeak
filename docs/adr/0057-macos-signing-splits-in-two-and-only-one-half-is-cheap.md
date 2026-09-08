# 57. macOS signing splits in two, and only one half is cheap

**Status:** Accepted.

**Chosen:** a self-signed Code Signing certificate on the development
machine, and nothing in a repository secret until there is a Developer ID
certificate to put there. The release job reads `APPLE_CERTIFICATE`,
`APPLE_CERTIFICATE_PASSWORD` and `APPLE_SIGNING_IDENTITY` when they exist,
builds unsigned when they do not, and reads the signature back either way.

Issue #29 reads as one problem and is two. The keychain prompt on every
rebuild is a *development* problem with a free fix: an ad-hoc signature's
designated requirement is a `cdhash`, so a keychain grant names one specific
build and the next build invalidates it. Any certificate replaces that with an
identifier and an issuer, which every later build satisfies. Gatekeeper
warning a *downloader* is a distribution problem, and a certificate only one
Mac has ever seen does not touch it.

**Why the self-signed certificate stays off CI.** It would change nothing a
downloader sees — the warning is identical — while putting a code-signing
private key inside a job that runs several hundred crates' build scripts, npm
lifecycle scripts and a Gradle plugin, any of which can read the environment.
That is the surface decision 55 narrowed the token for, and a signing key is
worse to lose than a token: revoking a token costs nobody an install.

**Why the job reads the signature back.** An ad-hoc signature is a signature.
`codesign -v` passes on it, the bundle runs, and nothing distinguishes "signed
with the certificate" from "the variable never reached codesign" without
printing the designated requirement. Configuring signing and having it
silently not apply would look exactly like success, which is the shape that
has cost this project the most: the reason exists and something between it and
the reader drops it — a Tailwind class emitting no CSS, `isMinifyEnabled`
deleting the Kotlin the engine calls, a CI run cancelled rather than failed.
So the step prints the requirement on every run, and fails on the one
combination that is a lie: an identity configured and an ad-hoc artefact.

Falsified before trusting it, against the ad-hoc bundle in `/Applications` and
against a certificate-signed app, in all four combinations. The first shape
matched a bare `*)` as "signed" and announced a signed build for an app that
was not there — the check's own version of the failure it exists to catch. It
now requires the words `designated =>` to claim anything.

**Costs.** The certificate is Patrick's to create and lives only on his Mac,
so a second developer hits the rebuild prompt until they make their own — the
recipe is in `docs/signing.md`. The verification step is macOS-only shell in a
workflow, exercised only on tags and `workflow_dispatch`, so a mistake in it
surfaces late. And the release body still says the app is unsigned: when a
Developer ID certificate lands, that sentence, the README paragraph and
`docs/signing.md` all owe an update in the same change.
