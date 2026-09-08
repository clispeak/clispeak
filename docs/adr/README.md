# Decisions

Why things are the way they are. One file each, newest last.

These are architecture decision records in the usual sense: a choice that was
made, why, and what it costs. **A record explains the alternative that was
rejected**, which is the part with nowhere else to live — a comment beside the
code can say what the code does, and only a record can say what was tried
first and why it was not kept.

## The rules

**One file per decision**, named `NNNN-a-short-slug.md`, numbered from one and
never reused. They were a single `decisions.md` until 8 September 2026, which
made every parallel branch collide on the next free number and left no way to
tell a live decision from a dead one without reading forward.

**Append, never edit.** A record says what was believed when it was written,
so correcting one destroys the only thing it is for. A decision that stops
being true is superseded by a *later* record, and its `Status` line is updated
to point at the one that replaced it. That status line is the only part of an
existing file that ever changes.

**Write one only when a rejected alternative matters.** If the whole reasoning
is about the code that exists, it belongs in a comment beside that code and
nowhere else. Two copies of one explanation drift apart independently, which
is the failure this directory is supposed to prevent.

**Link it from the code it explains.** A record nothing points at is one
nobody can find at the moment they need it.

**The title is part of the decision, and nothing checks it.** The gate reads
the numbering; whether a heading still describes what actually landed is only
detectable by reading. That has already gone wrong once — 56 was titled "the
Android shell is compiled on every push" after the step it named had been
taken back out in the same change. If a change ends somewhere other than where
it started, the title moves too.

## The records

| # | Decision |
|---|---|
| 1 | [Peer-to-peer, no hosted server](0001-peer-to-peer-no-hosted-server.md) |
| 2 | [iroh as the transport](0002-iroh-as-the-transport.md) |
| 3 | [Tauri v2 for the receiver app](0003-tauri-v2-for-the-receiver-app.md) |
| 4 | [Every install is both sender and receiver](0004-every-install-is-both-sender-and-receiver.md) |
| 5 | [CLI is a thin client to the local node](0005-cli-is-a-thin-client-to-the-local-node.md) |
| 6 | [Text only, on the wire](0006-text-only-on-the-wire.md) |
| 7 | [Any message length](0007-any-message-length.md) |
| 8 | [Native TTS engines out of the gate](0008-native-tts-engines-out-of-the-gate.md) |
| 9 | [A signed space roster, not pairwise pairing](0009-a-signed-space-roster-not-pairwise-pairing.md) |
| 10 | [Fire-and-forget by default, confirmation opt-in](0010-fire-and-forget-by-default-confirmation-opt-in.md) |
| 11 | [The receiver enforces policy; the sender only expresses intent](0011-the-receiver-enforces-policy-the-sender-only-expresses-inten.md) |
| 12 | [Multiple spaces per device, one keypair](0012-multiple-spaces-per-device-one-keypair.md) |
| 13 | [CBOR, and one QUIC stream per message](0013-cbor-and-one-quic-stream-per-message.md) |
| 14 | [Reject bad text; don't silently rewrite it](0014-reject-bad-text-dont-silently-rewrite-it.md) |
| 15 | [Lazy roster sync, eager revoke, no gossip layer](0015-lazy-roster-sync-eager-revoke-no-gossip-layer.md) |
| 16 | [Space rotation instead of fast revocation](0016-space-rotation-instead-of-fast-revocation.md) |
| 17 | [espeak-ng floor, Piper downloaded, fallback made visible](0017-espeak-ng-floor-piper-downloaded-fallback-made-visible.md) |
| 18 | [Named `voicecast`](0018-named-voicecast.md) |
| 19 | [The Flatpak bundles Piper and ships its own indicator library](0019-the-flatpak-bundles-piper-and-ships-its-own-indicator-librar.md) |
| 20 | [A space is identified by its founder, and a device may hold several](0020-a-space-is-identified-by-its-founder-and-a-device-may-hold-s.md) |
| 21 | [A stale socket is reclaimed only after a connection is refused](0021-a-stale-socket-is-reclaimed-only-after-a-connection-is-refus.md) |
| 22 | [Windows refuses to start rather than starting silent](0022-windows-refuses-to-start-rather-than-starting-silent.md) *(superseded)* |
| 23 | [The agent skill lives in this repo, and a test keeps it honest](0023-the-agent-skill-lives-in-this-repo-and-a-test-keeps-it-hones.md) |
| 24 | [The core owns where state lives; the app only overrides it on mobile](0024-the-core-owns-where-state-lives-the-app-only-overrides-it-on.md) |
| 25 | [The device that speaks decides how long a caller waits](0025-the-device-that-speaks-decides-how-long-a-caller-waits.md) |
| 26 | [The frontend asks for confirmation in the page, not through the webview](0026-the-frontend-asks-for-confirmation-in-the-page-not-through-t.md) |
| 27 | [Every control lives in the space it acts on](0027-every-control-lives-in-the-space-it-acts-on.md) |
| 28 | [An invite names its space, and joining one adds rather than replaces](0028-an-invite-names-its-space-and-joining-one-adds-rather-than-r.md) |
| 29 | [A space may be quieter than its device, never louder](0029-a-space-may-be-quieter-than-its-device-never-louder.md) |
| 30 | [A desktop that cannot speak stays on the network and says why](0030-a-desktop-that-cannot-speak-stays-on-the-network-and-says-wh.md) |
| 31 | [Joining is a read and then a decision, not a single button](0031-joining-is-a-read-and-then-a-decision-not-a-single-button.md) |
| 32 | [A device's name is asked of the system, not read from a Linux file](0032-a-devices-name-is-asked-of-the-system-not-read-from-a-linux.md) |
| 33 | [Kotlin reached over JNI is kept from R8 by a rule, and the rule is gated](0033-kotlin-reached-over-jni-is-kept-from-r8-by-a-rule-and-the-ru.md) |
| 34 | [A report names the device, not just the label it was addressed by](0034-a-report-names-the-device-not-just-the-label-it-was-addresse.md) |
| 35 | [The socket is a name, and the node says so](0035-the-socket-is-a-name-and-the-node-says-so.md) |
| 36 | [A peer's clock is bounded, and a join record names the peer that sent it](0036-a-peers-clock-is-bounded-and-a-join-record-names-the-peer-th.md) |
| 37 | [A peer that names a space is answered about that space, or not at all](0037-a-peer-that-names-a-space-is-answered-about-that-space-or-no.md) |
| 38 | [Peer text is escaped where it is printed, not where it is stored](0038-peer-text-is-escaped-where-it-is-printed-not-where-it-is-sto.md) |
| 39 | [Any member may vouch for any device, and the docs now say so](0039-any-member-may-vouch-for-any-device-and-the-docs-now-say-so.md) |
| 40 | [A receiver bounds what it will accept as a message, and says why it refused](0040-a-receiver-bounds-what-it-will-accept-as-a-message-and-says.md) |
| 41 | [A process is waited on by polling, so it can be killed while we wait](0041-a-process-is-waited-on-by-polling-so-it-can-be-killed-while.md) |
| 42 | [A process that ran and failed is not a process that spoke](0042-a-process-that-ran-and-failed-is-not-a-process-that-spoke.md) |
| 43 | [The keyring is asked once per process](0043-the-keyring-is-asked-once-per-process.md) |
| 44 | [Exit codes and `--json` keep the promise the docs made](0044-exit-codes-and-json-keep-the-promise-the-docs-made.md) |
| 45 | [State is written privately and all at once, through one function](0045-state-is-written-privately-and-all-at-once-through-one-funct.md) |
| 46 | [A node that cannot start is a state the interface can read](0046-a-node-that-cannot-start-is-a-state-the-interface-can-read.md) |
| 47 | [Clicking the Dock icon brings the window back](0047-clicking-the-dock-icon-brings-the-window-back.md) |
| 48 | [A failure that reached the engine says which engine, and what it said](0048-a-failure-that-reached-the-engine-says-which-engine-and-what.md) |
| 49 | [The portability gate checks every spelling, and exemptions are written down](0049-the-portability-gate-checks-every-spelling-and-exemptions-ar.md) |
| 50 | [Policy is asked again at the moment of speaking](0050-policy-is-asked-again-at-the-moment-of-speaking.md) |
| 51 | [The release job builds the variant that ships, and says what it built](0051-the-release-job-builds-the-variant-that-ships-and-says-what.md) |
| 52 | [A node keeps looking for a speech engine it did not find at startup](0052-a-node-keeps-looking-for-a-speech-engine-it-did-not-find-at.md) |
| 53 | [Every gate is one command, and `main` never cancels its own run](0053-every-gate-is-one-command-and-main-never-cancels-its-own-run.md) |
| 54 | [The tray's left click opens the window; the menu is on the right](0054-the-trays-left-click-opens-the-window-the-menu-is-on-the-rig.md) |
| 55 | [The release workflow is hardened before there is anything to steal](0055-the-release-workflow-is-hardened-before-there-is-anything-to.md) |
| 56 | [Two Android bugs, and the shell that nothing compiles until a tag](0056-two-android-bugs-and-the-shell-that-nothing-compiles-until-a.md) |
| 57 | [macOS signing splits in two, and only one half is cheap](0057-macos-signing-splits-in-two-and-only-one-half-is-cheap.md) |
| 58 | [A list is reconciled, not rebuilt](0058-a-list-is-reconciled-not-rebuilt.md) |
| 59 | [`aria-modal` is a claim; `inert` is the mechanism](0059-aria-modal-is-a-claim-inert-is-the-mechanism.md) |
| 60 | [The Android key goes in CI, and the macOS one does not](0060-the-android-key-goes-in-ci-and-the-macos-one-does-not.md) |
| 61 | [The app's reply handling is one module, so a test can execute it](0061-the-apps-reply-handling-is-one-module-so-a-test-can-execute.md) |
| 62 | [`handle_peer` takes a trait, so the protocol can be driven by a test](0062-handle-peer-takes-a-trait-so-the-protocol-can-be-driven-by-a.md) |
| 63 | [The five-target verdict is bought once, at the point that asks for it](0063-the-five-target-verdict-is-bought-once-at-the-point-that-ask.md) |
| 64 | [Two gates that check the tree before the tree is checked](0064-two-gates-that-check-the-tree-before-the-tree-is-checked.md) |
| 65 | [Notarisation is a second thing, and the release says whether it happened](0065-notarisation-is-a-second-thing-and-the-release-says-whether.md) |
| 66 | [Marking a pull request ready needs a gap, and the rollup is a history](0066-marking-a-pull-request-ready-needs-a-gap-and-the-rollup-is-a.md) |
| 67 | [The self-signed certificate does not fix the keychain prompt](0067-the-self-signed-certificate-does-not-fix-the-keychain-prompt.md) |
| 68 | [A pause is a state the interface has to keep showing](0068-a-pause-is-a-state-the-interface-has-to-keep-showing.md) |
| 69 | [Whoever takes focus away is who gives it back](0069-whoever-takes-focus-away-is-who-gives-it-back.md) |
| 70 | [A Developer ID certificate closes #29, and `bundle` says how it signed](0070-a-developer-id-certificate-closes-29-and-bundle-says-how-it.md) |
| 71 | [`--to` is refused where it means nothing](0071-to-is-refused-where-it-means-nothing.md) |
| 72 | [The Piper payload signs the way notarisation requires](0072-the-piper-payload-signs-the-way-notarisation-requires.md) |
| 73 | [The skill path can be put back, and the old copy is not deleted](0073-the-skill-path-can-be-put-back-and-the-old-copy-is-not-delet.md) |
| 74 | [Open source, MIT or Apache-2.0, and two things that block it](0074-open-source-mit-or-apache-2-0-and-two-things-that-block-it.md) |
| 75 | [A control says what it did, not what it meant to do](0075-a-control-says-what-it-did-not-what-it-meant-to-do.md) |
| 76 | [The CLI and the node prove themselves to each other](0076-the-cli-and-the-node-prove-themselves-to-each-other.md) |
| 77 | [The Flatpak is packaging, not containment, and says so](0077-the-flatpak-is-packaging-not-containment-and-says-so.md) |
| 78 | [Revoke names one device, or refuses](0078-revoke-names-one-device-or-refuses.md) |
| 79 | [iOS is named rather than left to fall through](0079-ios-is-named-rather-than-left-to-fall-through.md) |
| 80 | [iOS speaks through AVSpeechSynthesizer, on the main thread, without an unsafe claim](0080-ios-speaks-through-avspeechsynthesizer-on-the-main-thread-wi.md) |
| 81 | [A voice we may actually ship](0081-a-voice-we-may-actually-ship.md) |
| 82 | [The project is called clispeak](0082-the-project-is-called-clispeak.md) |
| 83 | [The rename changed a signature, and every pairing died quietly](0083-the-rename-changed-a-signature-and-every-pairing-died-quietl.md) |
| 84 | [An error you cannot dismiss](0084-an-error-you-cannot-dismiss.md) |
| 85 | [A join that discards everything is not a join](0085-a-join-that-discards-everything-is-not-a-join.md) |
| 86 | [A device is called one thing, in every space, on every start](0086-a-device-is-called-one-thing-in-every-space-on-every-start.md) |
| 87 | [An error can have lines, because the values inside it are escaped](0087-an-error-can-have-lines-because-the-values-inside-it-are-esc.md) |
| 88 | [No agent skill on a phone](0088-no-agent-skill-on-a-phone.md) |
| 89 | [The node bounds its own dial, so the CLI stops blaming it](0089-the-node-bounds-its-own-dial-so-the-cli-stops-blaming-it.md) *(superseded)* |
| 90 | [The history is written off the path of a message being spoken](0090-the-history-is-written-off-the-path-of-a-message-being-spoke.md) |
| 91 | [macOS speaks in its own voice, and stops carrying Piper](0091-macos-speaks-in-its-own-voice-and-stops-carrying-piper.md) |
| 92 | [A test that signs and verifies agrees with itself](0092-a-test-that-signs-and-verifies-agrees-with-itself.md) |
| 93 | [Something answering is not a node answering](0093-something-answering-is-not-a-node-answering.md) |
| 94 | [A `#[cfg]` is not a conditional, it is one spelling of one](0094-a-cfg-is-not-a-conditional-it-is-one-spelling-of-one.md) |
| 95 | [Two budgets, not one budget with a mystery in it](0095-two-budgets-not-one-budget-with-a-mystery-in-it.md) |
| 96 | [Windows speaks in its own voice too](0096-windows-speaks-in-its-own-voice-too.md) |
| 97 | [A dozing phone is not an absent one](0097-a-dozing-phone-is-not-an-absent-one.md) |
| 98 | [A guess must not travel like a decision](0098-a-guess-must-not-travel-like-a-decision.md) |
| 99 | [The commonest route back was the one left out](0099-the-commonest-route-back-was-the-one-left-out.md) |
| 100 | [Leaving and rejoining beats rotating, and the recovery is the evidence](0100-leaving-and-rejoining-beats-rotating-and-the-recovery-is-the.md) |
| 101 | [The certificate names itself](0101-the-certificate-names-itself.md) |
| 102 | [The Mac speaks, and backgrounding costs nothing](0102-the-mac-speaks-and-backgrounding-costs-nothing.md) |
| 103 | [A socket in a room only you can enter](0103-a-socket-in-a-room-only-you-can-enter.md) |
| 104 | [What 1.0 is, and what it deliberately is not](0104-what-1-0-is-and-what-it-deliberately-is-not.md) |
| 105 | [The Windows installer calls our binary rather than deciding anything](0105-the-windows-installer-calls-our-binary-rather-than-deciding.md) |
| 106 | [Android is arm64, and the launch check dies with that choice](0106-android-is-arm64-and-the-launch-check-dies-with-that-choice.md) |
| 107 | [The Flatpak installs a command-line tool it cannot be driven by](0107-the-flatpak-installs-a-command-line-tool-it-cannot-be-driven.md) |
| 108 | [One machine, one address book, or the node refuses to start](0108-one-machine-one-address-book-or-the-node-refuses-to-start.md) |
| 109 | [Windows ships unsigned for 1.0](0109-windows-ships-unsigned-for-1-0.md) |
| 110 | [The socket has to cross the sandbox too, and the refusal comes back out](0110-the-socket-has-to-cross-the-sandbox-too-and-the-refusal-come.md) *(superseded)* |
| 111 | [One directory with two names, and the refusal comes back](0111-one-directory-with-two-names-and-the-refusal-comes-back.md) |
| 112 | [The beta is 0.9.0, because Android cannot count a suffix](0112-the-beta-is-0-9-0-because-android-cannot-count-a-suffix.md) |
| 113 | [Prove the clock is wrong where it can be proved, and say so where it cannot](0113-prove-the-clock-is-wrong-where-it-can-be-proved-and-say-so-w.md) |
| 114 | [Two nodes in one process, so the protocol can be tested at all](0114-two-nodes-in-one-process-so-the-protocol-can-be-tested-at-al.md) |
| 115 | [A rename stamps itself strictly after the label it replaces](0115-a-rename-stamps-itself-strictly-after-the-label-it-replaces.md) |
| 116 | [A space id names the founding, not the second it happened in](0116-a-space-id-names-the-founding-not-the-second-it-happened-in.md) |
| 117 | [The CLI gets a library target, so its copies can be checked](0117-the-cli-gets-a-library-target-so-its-copies-can-be-checked.md) |
| 118 | [The browser probes run in CI, because the reason they did not was wrong](0118-the-browser-probes-run-in-ci-because-the-reason-they-did-not.md) |
| 119 | [`CLAUDE.md` gets audited, and the first audit found three stale claims](0119-claude-md-gets-audited-and-the-first-audit-found-three-stale.md) |
| 120 | [The Windows pipe carries access rules, and CI is what checks them](0120-the-windows-pipe-carries-access-rules-and-ci-is-what-checks.md) |
| 121 | [The working agreement lives in the tool, not in an agent's memory](0121-the-working-agreement-lives-in-the-tool-not-in-an-agents-mem.md) |
| 122 | [The hook rides in the skill, and the questions ride in the binary](0122-the-hook-rides-in-the-skill-and-the-questions-ride-in-the-bi.md) |
| 123 | [The hook's frontmatter carries no `args`, and a test says so](0123-the-hooks-frontmatter-carries-no-args-and-a-test-says-so.md) |
