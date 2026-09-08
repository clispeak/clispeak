// Highlight the card the visitor probably wants.
//
// An enhancement and nothing more. With JavaScript off there are four cards,
// none highlighted, every link working — which is the point: guessing wrong
// and hiding the right answer is worse than not guessing, so this only ever
// *adds* emphasis and never removes a card or a link.
(() => {
  const ua = navigator.userAgent;
  const platform = navigator.platform || "";

  // Order matters. Android reports "Linux" in its user agent, so it has to be
  // tested first or every phone is offered a Flatpak.
  const guess =
    /Android/i.test(ua) ? "android"
    : /iPhone|iPad|iPod/i.test(ua) ? "ios"
    : /Mac/i.test(platform) || /Mac OS X/i.test(ua) ? "macos"
    : /Win/i.test(platform) || /Windows/i.test(ua) ? "windows"
    : /Linux|X11/i.test(platform) || /Linux/i.test(ua) ? "linux"
    : null;

  // iOS is deliberately not a card. A visitor on an iPhone gets told there is
  // nothing to download rather than being highlighted toward a file that does
  // not exist for them.
  if (!guess || guess === "ios") return;

  const card = document.getElementById(`card-${guess}`);
  if (!card) return;

  card.classList.add("ring-2", "ring-accent-500", "border-accent-500");
  const link = card.querySelector("[data-primary]");
  if (link) {
    // The accent is spent here and nowhere else on the page, so "the button
    // that does the thing" is unambiguous.
    link.classList.remove("border-neutral-300", "hover:bg-neutral-100",
                          "dark:border-neutral-700", "dark:hover:bg-neutral-800");
    link.classList.add("border-accent-600", "bg-accent-600", "text-white",
                       "hover:bg-accent-500");
  }

  const note = document.getElementById("guess-note");
  if (note) {
    note.textContent = "Your platform looks highlighted below — all four work.";
    note.hidden = false;
  }
})();

/**
 * Say which version the download links will give you.
 *
 * Fetched, not written into the page. A number typed by hand goes stale the
 * next time we ship and then states the wrong thing with confidence — and the
 * links beside it point at "latest", so a hardcoded version would eventually
 * disagree with the very files it labels.
 *
 * Silent on failure. Someone on a flaky connection, or behind something that
 * blocks the API, should see the page they came for rather than an error
 * about a detail. No version at all is better than a wrong one.
 */
(async () => {
  const el = document.getElementById("latest-version");
  if (!el) return;
  try {
    const r = await fetch("https://api.github.com/repos/clispeak/clispeak/releases/latest", {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!r.ok) return;
    const tag = (await r.json()).tag_name;
    // Trusted only as far as its shape. It is someone else's JSON landing in
    // our page, and `textContent` keeps it text whatever it contains.
    if (!/^v?\d+\.\d+\.\d+/.test(tag || "")) return;
    el.textContent = `Latest release: ${tag}`;
    el.hidden = false;
  } catch {
    // Offline, blocked, or rate limited. The page is still the page.
  }
})();
