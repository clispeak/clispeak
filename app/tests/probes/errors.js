/**
 * Issue #144: an error toast could never be dismissed.
 *
 * Errors are pinned on purpose — a failure must not vanish before it is read.
 * But "until the next action" is not a way out on a phone, where the next
 * action may never come, and the bubble had no control, no click handler, and
 * lived inside a `pointer-events-none` wrapper. One failure held the bottom
 * of the screen for the rest of the session.
 *
 * These check the way out exists without giving up the pinning. Every one of
 * them fails against the interface as it was — run before the fix, not
 * assumed.
 */
const report = [];
const $ = (id) => document.getElementById(id);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const bubble = () => $("result-alert").firstElementChild;

(async () => {
  await sleep(600);

  // Reach the real `say()` the way the app does: a failing command.
  window.__forceError = "the node is not running";
  $("preview").click();
  await sleep(300);

  report.push(["a failure raises an error bubble", !!bubble()]);
  if (!bubble()) {
    finish();
    return;
  }

  report.push([
    "the bubble is a control, not a paragraph",
    bubble().tagName === "BUTTON",
    `is <${bubble().tagName.toLowerCase()}>`,
  ]);

  // The wrapper turns pointer events off for the whole layer, which is right
  // for the layer and fatal for a thing that must be tapped.
  report.push([
    "and can actually be tapped",
    getComputedStyle(bubble()).pointerEvents !== "none",
    `pointer-events: ${getComputedStyle(bubble()).pointerEvents}`,
  ]);

  report.push([
    "it says what pressing it does",
    /dismiss/i.test(bubble().getAttribute("aria-label") ?? ""),
    `aria-label: ${bubble().getAttribute("aria-label")}`,
  ]);

  // The pinning is the part that must survive: an error still does not time
  // out on its own. SAY_MS is 3500, so this is comfortably past it.
  await sleep(4200);
  report.push([
    "an untouched error is still there after the info timeout",
    !!bubble(),
    "pinning is deliberate and must not regress",
  ]);

  bubble()?.click();
  await sleep(150);
  report.push(["pressing it clears the error", !bubble()]);

  finish();
})();

function finish() {
  const pre = document.createElement("pre");
  pre.id = "report";
  pre.textContent = report
    .map(([what, ok, note]) => `${ok ? "PASS" : "FAIL"}  ${what}${note ? `  (${note})` : ""}`)
    .join("\n");
  document.body.appendChild(pre);
}
