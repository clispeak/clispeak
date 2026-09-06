# The website

A plan, for Patrick to approve before anyone writes it. The implementation is
the Mac agent's; this document is the brief and the acceptance criteria.

**What it is for.** Getting a person from *"what is this"* to *"it is
installed and paired with my phone"* without opening a terminal. That is the
whole job. Everything below either serves it or is out of scope.

**What it is not.** Not a documentation site, not a blog, not a product
marketing site. The README is the documentation and stays that way — two
places describing the same software is how one of them starts lying.

## The decisions already made

These are settled and are inputs, not questions.

| | |
|---|---|
| Domain | `clispeak.com`. `clispeak.org` is a 301 at the registrar, not a second site |
| Host | GitHub Pages, from **this** repository |
| Publish | a GitHub Actions workflow, from a `site/` directory |
| Downloads | `https://github.com/clispeak/clispeak/releases/latest/download/<stable name>` (#189) |
| Platforms | Linux, Windows, Android, macOS. **iOS says "coming soon" and offers nothing** (decision 104) |

**Published by Actions, not by pointing Pages at a folder.** Pages will happily
serve `/docs`, and that is precisely the wrong answer here: `docs/` is
decisions, signing procedures and build plans written for us. A workflow
publishing `site/` means what is public is a deliberate list rather than
whatever happens to be in a directory.

## Sequencing: the site cannot go live yet

**There are no releases and no tags.** `gh release list` is empty. Every
`releases/latest/download/…` URL therefore 404s, so a site published today is
a page with four dead buttons — and it fails in the way this project keeps
being failed, because the *page* is fine and only the destinations are
missing.

So the order is fixed:

1. The Android keystore exists and the four secrets are set (#31), or the
   Android link cannot be real — a tagged release now **refuses** to publish
   an unsigned APK (#191)
2. A version is tagged and the draft release is published
3. Every one of the four URLs is fetched and returns a file
4. Then, and only then, DNS points at the site

Building the page can start immediately. Publishing it cannot.

## Structure

**One page.** A second page is a decision to be made later, with a reason.

1. **What it is** — one sentence, then one more. *"Speak text aloud on your
   own devices. Peer to peer, no server, no account."* Then the thing that
   makes it different: an agent drives it.
2. **Download** — four cards. Guess the visitor's platform and highlight that
   one; **always show all four**, because guessing wrong and hiding the right
   answer is worse than not guessing.
3. **After you download** — per platform, the frightening step, in one
   sentence each. This section is not optional; see below.
4. **Pair two devices** — one install does nothing on its own, and this is
   where a person gives up.
5. **For agents** — the reason the project exists, not a footnote.
6. **What works today** — the honest table.
7. Footer: source, licence, issues.

### The section that matters most is 3

Every platform has a moment where the operating system tells the visitor this
software is dangerous, and every one of those moments looks like the download
was a mistake:

| | what happens | what the page must say |
|---|---|---|
| Windows | SmartScreen: *"Windows protected your PC"* | we have no Windows signing certificate; More info → Run anyway |
| macOS | Gatekeeper, on a signed and notarised app, still asks | it is signed and notarised (#29) — this is the normal first-open prompt |
| Android | *"install unknown apps"* must be allowed | how, and that it is per-app |
| Android | **arm64 only** (decision 106) | needs a 64-bit arm phone; will not install on an emulator |
| Linux | Flatpak needs flatpak | the one-line install, and which distributions have it already |

The Android row is the one with history: *"you can't install the app on your
device"* has already happened once here, and the message named nothing.

### And section 5 is the one nobody else would write

`clispeak` is a command an agent calls. The page should show that in about six
lines — install, pair, `clispeak --to Phone "the build finished"` — and say
what makes it usable by an agent rather than by a person: distinct exit codes,
errors that carry a fix, and a receiver that reports a reason instead of
swallowing a failure. Link the skill.

## Design

**Hand-written HTML with Tailwind v4.** The same tool `app/src` already
builds with, so the site and the app look like one product for free, and there
is no framework to keep patched. A static site generator is more machinery
than this needs, and it is machinery that rots.

Match the app's tokens rather than inventing a palette:

- grounds `neutral-50` / `neutral-950`, text `neutral-900` / `neutral-100`
- **one accent**, the app's `--color-accent-*` (oklch, hue 250), used for the
  primary action only, so "the button that does the thing" is never ambiguous
- `font-mono` for anything that is a command or an identity string
- dark mode follows the system
- `tracking-tight` headings, `rounded-xl` cards

**It must be readable with JavaScript disabled.** The only script is the
platform guess, which is an enhancement: without it, four cards, none
highlighted, all working.

## Explicitly out of scope

No analytics. No cookies. No third-party fonts — a system font stack, so
nothing is fetched from anyone. No email capture, no sign-up, no chat widget.

This is not asceticism. The product's whole claim is that it is peer to peer
with no server and no account, and a landing page that phones home about the
people reading that sentence is an argument against the software it is
selling.

## Work, in an order that makes sense

1. `site/index.html`, `site/input.css`, and a Tailwind build — page renders
   locally, with real content and placeholder links
2. The four download cards and the platform guess
3. Sections 3, 4 and 5 — the honest ones, which are writing rather than code
4. `.github/workflows/site.yml` — build the CSS, publish `site/` to Pages
5. A link check that runs after the workflow and fetches all four URLs
6. Favicon and Open Graph image, from #188
7. DNS, last, after step 3 of the sequencing section above

Steps 1 to 5 can be done now. 6 needs the icon. 7 needs a release.

## Acceptance

- [ ] Renders at 360px wide with no horizontal scroll
- [ ] Light and dark both deliberate — the light theme went unlooked-at for the
      whole build of the app, which is what that check is here to prevent
- [ ] Readable with JavaScript off
- [ ] Every download link fetched and returns a file, by a script, not by eye
- [ ] Nothing is loaded from a third party. Checked in the network tab
- [ ] Every claim about a platform matches `README.md`'s table. If the two
      disagree the site is wrong, because the README is where the person who
      changed the code was looking
