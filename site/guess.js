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
