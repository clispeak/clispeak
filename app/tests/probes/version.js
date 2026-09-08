/**
 * Issue #235: the interface never said what version it was.
 *
 * That cost a day and a half — a laptop ran a build from eighty-four minutes
 * before the first release for that long, and the only way anyone found out
 * was by reading a package manager's install timestamp. On a phone there is
 * no command line at all, so this screen is the only place the question can
 * be answered.
 *
 * The checks are about what somebody filing a bug report needs: a number that
 * is there, is the node's, and can be selected. The last one matters because
 * the placeholder is a single character and a screen showing `…` forever
 * looks exactly like a screen showing a version, from a distance.
 */
const report = [];
const $ = (id) => document.getElementById(id);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  await sleep(400);

  const el = $("app-version");
  report.push(["the About row exists", !!el]);
  report.push([
    "it shows the version the node reported",
    el && el.textContent.trim() === "0.9.2",
    el ? `"${el.textContent.trim()}"` : "missing",
  ]);
  report.push([
    "the placeholder is gone",
    el && !el.textContent.includes("…"),
    "a screen still showing the placeholder looks like one showing a version",
  ]);
  report.push([
    "it can be selected for pasting into a report",
    el && el.className.includes("select-all"),
  ]);
  report.push([
    "it is on the settings screen",
    el && el.closest("#screen-settings") !== null,
  ]);

  const pre = document.createElement("pre");
  pre.id = "report";
  pre.textContent = report
    .map(([what, ok, note]) => `${ok ? "PASS" : "FAIL"}  ${what}${note ? `  (${note})` : ""}`)
    .join("\n");
  document.body.appendChild(pre);
})();
