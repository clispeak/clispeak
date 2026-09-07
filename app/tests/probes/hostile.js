/**
 * A device name is somebody else's text, rendered on this machine.
 *
 * Names arrive over the network from other people's devices and are shown in
 * the device list, the history, the space list and the target picker. Nothing
 * about a name is validated for *rendering* — the node refuses names that
 * would break a `--to` selector, which is a different question and stops
 * nothing here.
 *
 * So the property is: whatever a name contains, it becomes text. Not "it is
 * escaped", which is a claim about a mechanism — the check below counts
 * elements, because the only thing that matters is whether markup in a name
 * became markup on the page.
 *
 * `main.js` uses `textContent` in sixty-eight places and `innerHTML` in one,
 * for a QR code its own Rust side produced. That is a good position to be in
 * and exactly the kind of thing that decays quietly: one `innerHTML` added
 * later for convenience, in a place a name reaches, and nothing says so. This
 * is what says so.
 */
const report = [];
const $ = (id) => document.getElementById(id);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  // Every shape that has ever mattered: a tag that fires without being
  // clicked, a closed tag, and an entity that would decode into one.
  window.__peerName = '<img src=x onerror="window.__pwned=true">Phone</b>&lt;script&gt;';
  window.__pwned = false;

  // Wait for a poll to bring the name in, rather than reloading.
  await sleep(6000);

  const spaces = $("spaces");
  report.push(["the device list rendered", spaces.textContent.length > 0]);
  report.push([
    "the name is on the page as text",
    spaces.textContent.includes("Phone"),
    "if this fails the probe is testing nothing",
  ]);
  report.push([
    "no image element came from it",
    spaces.querySelectorAll("img").length === 0,
    `${spaces.querySelectorAll("img").length} found`,
  ]);
  report.push([
    "no script element came from it",
    spaces.querySelectorAll("script").length === 0,
  ]);
  report.push([
    "nothing in the name executed",
    window.__pwned === false,
    "an onerror handler ran",
  ]);
  // The angle brackets have to still be *there* — a name that silently loses
  // characters is a different bug, and one that would make the checks above
  // pass for the wrong reason.
  report.push([
    "the markup survives as characters",
    spaces.textContent.includes("<img"),
    "the name was stripped rather than escaped, so this proves less than it looks",
  ]);

  const pre = document.createElement("pre");
  pre.id = "report";
  pre.textContent = report
    .map(([what, ok, note]) => `${ok ? "PASS" : "FAIL"}  ${what}${note ? `  (${note})` : ""}`)
    .join("\n");
  document.body.appendChild(pre);
})();
