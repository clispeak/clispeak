/**
 * Issue #74: what a five-second poll used to throw away.
 *
 * Every check here fails if `syncRows` goes back to `replaceChildren` — that
 * was confirmed by doing it, not assumed. The presence checks are the other
 * half: keeping a node is only right if what the node *shows* still updates.
 */
const report = [];
const $ = (id) => document.getElementById(id);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  await sleep(500);
  $("tab-spaces").click();
  await sleep(300);

  // Expand a clamped message, as a reader does.
  const body = document.querySelector('#history [data-part="body"]');
  const row = body.closest("li");
  body.click();
  report.push(["a message can be expanded at all", !body.classList.contains("line-clamp-2")]);

  // Put focus on a button, as a keyboard user does.
  const remove = [...document.querySelectorAll("#spaces button")].find(
    (b) => b.textContent === "Remove",
  );
  remove.focus();
  report.push(["a button can take focus at all", document.activeElement === remove]);

  // Leave a request in flight.
  const play = document.querySelector("#history button");
  play.disabled = true;
  play.textContent = "…";

  // The peer row, not the first: "this device" is a constant by design.
  const seen = [...document.querySelectorAll('#spaces [data-part="seen"]')].find(
    (el) => el.textContent !== "this device",
  );
  const seenBefore = seen.textContent;
  const dot = seen.closest("div").parentElement.querySelector('[data-part="dot"]');
  const dotBefore = dot.className;

  // Push the peer past the three-minute mark, then let the app poll.
  window.__advance(40);
  await sleep(5600);

  const bodyNow = document.querySelector('#history [data-part="body"]');
  const playNow = document.querySelector("#history button");
  report.push(["the same history node survives a poll", bodyNow === body && row.isConnected]);
  report.push(["an expanded message stays expanded", !bodyNow.classList.contains("line-clamp-2")]);
  report.push(["a focused button keeps focus", document.activeElement === remove && remove.isConnected]);
  // Read from the DOM rather than the reference above: a detached node keeps
  // its own text, so checking the reference alone passed even when the row
  // had been thrown away.
  report.push(["an in-flight button keeps its label", playNow === play && playNow.disabled]);
  report.push(["presence text still updates", seen.textContent !== seenBefore, `${seenBefore} → ${seen.textContent}`]);
  report.push(["presence dot still repaints", dot.className !== dotBefore]);

  const pre = document.createElement("pre");
  pre.id = "report";
  pre.textContent = report
    .map(([what, ok, note]) => `${ok ? "PASS" : "FAIL"}  ${what}${note ? `  (${note})` : ""}`)
    .join("\n");
  document.body.appendChild(pre);
})();
