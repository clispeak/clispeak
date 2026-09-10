/**
 * The Speak tab: where a message goes, and what came back.
 *
 * Three things here are only checkable in a browser. The picker is rebuilt
 * from a five-second poll, so a selection has to survive one — the same
 * failure as #74, in a control where losing it sends the message to the wrong
 * device rather than merely scrolling. The selector the node receives is
 * built from the picker rather than typed, so nothing else can tell whether
 * `to` and `priority` actually left the page. And a send that reached nobody
 * comes back as a thrown error, which used to be the shape that produced no
 * message at all (#46).
 */
const report = [];
const $ = (id) => document.getElementById(id);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const lastCall = (cmd) => [...window.__calls].reverse().find((c) => c.cmd === cmd);

(async () => {
  await sleep(500);
  $("tab-speak").click();
  await sleep(300);

  const to = $("send-to");
  const values = [...to.options].map((o) => o.value);
  // One space in the stub, so nothing is qualified and the peer is addressed
  // by the bare name a person reads on the Spaces tab.
  report.push([
    "this device, everyone and the peer are all offered",
    values.join("|") === "|all|Phone",
    values.join("|"),
  ]);
  report.push([
    "this device is the one selected to begin with",
    to.value === "",
  ]);
  report.push([
    "the picker says what the choice means",
    $("send-to-note").textContent.length > 0,
  ]);

  // Choose the peer, as a person does, and let a poll run underneath it.
  to.value = "Phone";
  to.dispatchEvent(new Event("change"));
  const note = $("send-to-note").textContent;
  report.push(["choosing a device changes the note", note.includes("Phone"), note]);

  $("send-text").value = "Hello from the app";
  $("send-priority").value = "high";
  await sleep(5600);
  report.push([
    "a chosen device survives a poll",
    to.value === "Phone",
    `after the poll: ${to.value || "(this device)"}`,
  ]);
  report.push([
    "typed text survives a poll",
    $("send-text").value === "Hello from the app",
  ]);

  // Send it, and read what actually reached the command.
  window.__spoken = [
    { device: "Phone", heard: true, outcome: "queued" },
    { device: "Desk", heard: false, outcome: "could not be reached" },
  ];
  $("send").click();
  await sleep(400);

  const sent = lastCall("speak");
  report.push([
    "the selector reaches the node",
    sent?.args?.to === "Phone",
    JSON.stringify(sent?.args ?? null),
  ]);
  report.push(["the priority reaches the node", sent?.args?.priority === "high"]);

  const rows = [...document.querySelectorAll("#send-result li")].map((li) =>
    li.textContent.replace(/\s+/g, " ").trim(),
  );
  report.push([
    "every device gets a row of its own",
    rows.length === 2 && rows[0].includes("Phone") && rows[1].includes("Desk"),
    rows.join(" / "),
  ]);
  report.push([
    "the one that missed is drawn as a miss",
    !document.querySelectorAll("#send-result li")[1].querySelector("span:last-child")
      .className.includes("emerald"),
  ]);
  // The whole reason the box is not cleared: sending the same line to a
  // second device is what this tab is for.
  report.push(["the message stays for a second send", $("send-text").value === "Hello from the app"]);

  // Nothing spoke it anywhere. The command throws, and the panel has to keep
  // the reason after the toast has gone.
  window.__speakFails = "Phone: muted; Desk: quiet hours are on";
  $("send").click();
  await sleep(400);
  const failed = $("send-result").textContent;
  report.push([
    "a send that reached nobody says why, in the panel",
    failed.includes("muted") && failed.includes("quiet hours"),
    failed.replace(/\s+/g, " ").trim(),
  ]);

  const pre = document.createElement("pre");
  pre.id = "report";
  pre.textContent = report
    .map(([what, ok, note]) => `${ok ? "PASS" : "FAIL"}  ${what}${note ? `  (${note})` : ""}`)
    .join("\n");
  document.body.appendChild(pre);
})();
