// Asking before counting, and counting download clicks by platform.
//
// The consent *defaults* live inline in the page head, because they have to
// reach Google's data layer before anything else does. This file is the part
// that can wait: showing the question, remembering the answer, and recording
// which download button was pressed. Decision 124 has the reasoning.

(() => {
  const KEY = "clispeak-consent";
  const banner = document.getElementById("consent");
  const reopen = document.getElementById("consent-open");
  // `gtag` is defined inline in the head. If that script was stripped or
  // blocked, every call below becomes a no-op rather than an error.
  const tag = typeof window.gtag === "function" ? window.gtag : () => {};

  const stored = () => {
    try {
      return localStorage.getItem(KEY);
    } catch {
      return null;
    }
  };

  /**
   * Whether this visitor is probably somewhere consent is required.
   *
   * **A guess, and arranged so being wrong is cheap.** Google already knows
   * the region from the IP address and has defaulted analytics to off there;
   * this only decides whether to *ask*. An EU visitor whose timezone is not
   * European is simply never counted. A visitor elsewhere whose timezone is
   * European is asked a question they did not strictly need. Neither error
   * records anybody without a yes.
   *
   * The Atlantic zones are EU and EEA territory with no `Europe/` name: the
   * Canaries, Madeira, the Azores, and Iceland.
   */
  const probablyEurope = () => {
    try {
      const zone = Intl.DateTimeFormat().resolvedOptions().timeZone || "";
      return /^(Europe\/|Atlantic\/(Canary|Madeira|Azores|Reykjavik)$)/.test(zone);
    } catch {
      // Cannot tell, so ask. Asking is the safe way to be wrong.
      return true;
    }
  };

  let opener = null;

  const show = () => {
    if (!banner) return;
    opener = document.activeElement;
    banner.hidden = false;
  };

  const choose = (value) => {
    try {
      localStorage.setItem(KEY, value);
    } catch {
      // Private window or storage blocked: the choice still applies to this
      // page, it just will not be remembered, so the question comes back.
    }
    tag("consent", "update", { analytics_storage: value });
    if (banner) banner.hidden = true;
    // Back to where the visitor was, if they opened this from the footer.
    if (opener && typeof opener.focus === "function") opener.focus();
  };

  if (banner) {
    banner.addEventListener("click", (e) => {
      const button = e.target.closest("[data-consent]");
      if (button) choose(button.dataset.consent);
    });
  }

  // The way back. Withdrawing consent has to be as easy as giving it, so the
  // question is reachable from every page for every visitor, not only the
  // ones the guess above picked out.
  if (reopen) {
    reopen.hidden = false;
    reopen.addEventListener("click", show);
  }

  if (!stored() && probablyEurope()) show();

  /**
   * Record which download was clicked, by platform.
   *
   * The platform is read from the file name in the link rather than from an
   * attribute beside it, so there is one place it is written and nothing to
   * drift: `clispeak-macos.dmg` is macOS because the URL says so.
   *
   * This counts *clicks*, not completed downloads. GitHub's own download
   * counts are the other half, and they include every automated fetch of the
   * file — which is why a click, made by a person on this page, is the number
   * worth having beside them.
   *
   * `auxclick` as well as `click`, because a middle-click opens the link and
   * fires no `click` event at all.
   */
  const recordDownload = (e) => {
    if (e.type === "auxclick" && e.button !== 1) return;
    const link = e.target.closest && e.target.closest('a[href*="/releases/latest/download/"]');
    if (!link) return;
    const file = link.href.split("/").pop();
    const match = /^clispeak-([a-z]+)/.exec(file);
    tag("event", "download_click", {
      platform: match ? match[1] : "unknown",
      file_name: file,
    });
  };
  document.addEventListener("click", recordDownload);
  document.addEventListener("auxclick", recordDownload);
})();
