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
 * `main.js` uses `textContent` in seventy-six places and `innerHTML` in one,
 * for a QR code its own Rust side produced. That is a good position to be in
 * and exactly the kind of thing that decays quietly: one `innerHTML` added
 * later for convenience, in a place a name reaches, and nothing says so. This
 * is what says so.
 *
 * The first number said sixty-eight and was seventy-two before the Speak tab
 * was written — stale by four, in the comment whose subject is things going
 * stale. Nothing counts it, so it is a note rather than a claim: the checks
 * below are what hold, and they hold wherever they are pointed. Which is why
 * pointing them at each new surface is the part that matters.
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

  // The Speak tab shows the same name twice more: in the target picker, and
  // in the row saying what that device did with the message. Both are new
  // surfaces for the same text, which is the way this decays — the property
  // holds everywhere it was checked and nowhere it was added since.
  $("tab-speak").click();
  await sleep(300);
  const picker = $("send-to");
  report.push([
    "the picker offers the name as text",
    [...picker.options].some((o) => o.textContent.includes("<img")),
    [...picker.options].map((o) => o.textContent).join(" | "),
  ]);
  report.push([
    "no element came out of the picker",
    picker.querySelectorAll("img, script").length === 0,
  ]);

  window.__spoken = [{ device: window.__peerName, heard: false, outcome: "muted" }];
  $("send-text").value = "anything";
  $("send").click();
  await sleep(400);
  const result = $("send-result");
  report.push(["the result row rendered", result.textContent.includes("<img")]);
  report.push([
    "no element came out of the result row",
    result.querySelectorAll("img, script").length === 0,
  ]);
  report.push(["still nothing executed", window.__pwned === false]);

  const pre = document.createElement("pre");
  pre.id = "report";
  pre.textContent = report
    .map(([what, ok, note]) => `${ok ? "PASS" : "FAIL"}  ${what}${note ? `  (${note})` : ""}`)
    .join("\n");
  document.body.appendChild(pre);
})();
