/**
 * Patrick set the skill path to a directory under his Desktop, which is why
 * macOS asked the app for Desktop access — and then there was no way back to
 * the default short of retyping it. This is the button that undoes it.
 *
 * The checks that matter are the two that a "just refill the field" version
 * would have failed: the reset must survive the next poll, and the button
 * must disappear once there is nothing left to reset.
 */
const report = [];
const $ = (id) => document.getElementById(id);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  await sleep(600);
  $("tab-settings").click();
  await sleep(400);

  const custom = "/Users/someone/Desktop/skills/voicecast/SKILL.md";
  const fallback = "/Users/someone/.claude/skills/voicecast/SKILL.md";

  report.push(["the section is shown", !$("skill-section").hidden]);
  report.push(["the chosen path is in the field", $("skill-path").value === custom, $("skill-path").value]);
  report.push(["the reset is offered", $("skill-default").hidden === false]);

  $("skill-default").click();
  await sleep(500);

  report.push(["the field now holds the default", $("skill-path").value === fallback, $("skill-path").value]);
  report.push([
    "the reset is no longer offered",
    $("skill-default").hidden === true,
    `hidden=${$("skill-default").hidden}`,
  ]);
  // Says where the old copy is, because it was left there on purpose.
  const said = ($("result-status").textContent || "") + ($("result-alert").textContent || "");
  report.push(["it says where the old copy is", said.includes(custom), said.trim().slice(0, 90)]);

  // The reason the field alone was not enough: a poll must not put it back.
  await sleep(5200);
  report.push([
    "a poll does not restore the old path",
    $("skill-path").value === fallback,
    $("skill-path").value,
  ]);

  const pre = document.createElement("pre");
  pre.id = "report";
  pre.textContent = report
    .map(([what, ok, note]) => `${ok ? "PASS" : "FAIL"}  ${what}${note ? `  (${note})` : ""}`)
    .join("\n");
  document.body.appendChild(pre);
})();
