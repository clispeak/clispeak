/**
 * Issue #75: the four dialogs claimed `aria-modal` and kept none of it.
 *
 * The two Tab checks assert the handler's own effect rather than "focus is
 * still inside the dialog". A synthetic Tab does not perform the browser's
 * focus move, so the weaker check passed with no trap at all — which is the
 * kind of vacuous test this file exists to avoid.
 */
const report = [];
const $ = (id) => document.getElementById(id);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const press = (key, shift) =>
  document.dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey: !!shift, bubbles: true }));

(async () => {
  await sleep(600);
  $("tab-spaces").click();
  await sleep(300);

  const remove = [...document.querySelectorAll("#spaces button")].find(
    (b) => b.textContent === "Remove",
  );
  remove.focus();
  const opener = document.activeElement;
  remove.click();
  await sleep(400);

  const box = $("ask");
  report.push(["the dialog opens", !box.hidden]);
  report.push(["focus moves into it", box.contains(document.activeElement)]);
  report.push(["the tab bar is inert", document.querySelector("nav").inert === true]);
  report.push(["the screen behind is inert", $("screen-spaces").inert === true]);

  const stops = [...box.querySelectorAll("button, input")].filter((el) => !el.hidden);
  stops.at(-1).focus();
  press("Tab");
  await sleep(50);
  report.push(["Tab at the last stop wraps to the first", document.activeElement === stops[0]]);

  stops[0].focus();
  press("Tab", true);
  await sleep(50);
  report.push(["Shift-Tab at the first wraps to the last", document.activeElement === stops.at(-1)]);

  press("Escape");
  await sleep(200);
  report.push(["Escape closes it", box.hidden]);
  report.push(["inert is lifted", document.querySelector("nav").inert === false]);
  // The opener is a button `withButton` disabled for the duration, so this
  // only passes because the restore retries once the action has unwound.
  report.push([
    "focus returns to the opener",
    document.activeElement === opener,
    `${document.activeElement.tagName}/${(document.activeElement.textContent || "").trim().slice(0, 12)}`,
  ]);

  const pre = document.createElement("pre");
  pre.id = "report";
  pre.textContent = report
    .map(([what, ok, note]) => `${ok ? "PASS" : "FAIL"}  ${what}${note ? `  (${note})` : ""}`)
    .join("\n");
  document.body.appendChild(pre);
})();
