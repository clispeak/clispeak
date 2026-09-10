# The website

**The brief the site is held to.** It was written as a plan for approval
before anything existed; clispeak.com has been live since 8 September 2026 and
this is now the standard it is checked against rather than a proposal.

The one thing to keep in view when editing it: everything below either gets a
person from *"what is this"* to *"installed and paired with my phone"*, or it
does not belong on the page.

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

## Sequencing, and what it was for

This section said **"There are no releases and no tags. `gh release list` is
empty"**, and set the order: sign the Android key, tag a release, fetch all
four URLs, and only then point DNS at the site. That order was followed and
every step is done.

It is kept because the reasoning still binds anyone who adds a download: **a
page is not finished when the page is right.** A site published before the
artefacts exist has four dead buttons, and it fails the way this project keeps
being failed — the page is fine and only the destinations are missing. That is
what `site/check-links.sh` exists to catch, and why it fetches the download
URLs rather than checking that the markup contains them.

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
6. **Agreements** — how to tell an assistant when to speak.
7. Footer: source, licence, issues.

A *"What works today"* platform table was here and has been removed. It shipped
carrying the line *"if this disagrees with the README, the README is right"* —
a section admitting in its own copy that it would go out of date, on a page
nobody re-reads. Per-platform status is process information; it belongs in the
repository, where the people who need it look.

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

## Work

All seven steps are done: the page, the download cards and the platform guess,
the honest sections, `.github/workflows/site.yml`, `site/check-links.sh`, the
favicon and Open Graph image, and DNS.

## Acceptance

Checked on 10 September 2026, by running each one rather than by eye.

- [x] **No horizontal scroll.** Measured in a headless browser at three
      widths: the document is narrower than the viewport at each. The section
      nav is wider than a phone and scrolls *inside its own*
      `overflow-x-auto` container, which is the intended shape — the page body
      never scrolls sideways. Note for anyone re-running this: headless
      Chromium floors `innerWidth` at 500px, so `--window-size=360` does not
      actually test 360.
- [x] **Readable with JavaScript off.** 1,546 words and all four download
      links are in the HTML. `guess.js` only ever *adds* emphasis to a card;
      with it off there are four cards, none highlighted, every link working.
- [x] **Every download link fetched and returns a file**, by
      `site/check-links.sh`, not by eye. It also checks the section nav has
      one link per list item, after a nav entry was once added as a bare
      anchor beside an existing one.
- [x] **Nothing is loaded from a third party.** No `src` or `href` on the page
      points anywhere but github.com and clispeak.com.
- [ ] **Light and dark both deliberate.** Not re-verified here. The light
      theme went unlooked-at for the whole build of the app, which is what
      this check exists to prevent, and it needs eyes rather than a script.
- [x] **No platform claims to keep in step.** The per-platform table has been
      removed. This item used to say the site must match `README.md`'s table
      and that the README wins — two places holding one fact, with a rule for
      which lies. Deleting one of them is the better fix.

**The screenshots are out of date.** All three show a three-tab app; there are
four tabs since the Speak tab shipped.
