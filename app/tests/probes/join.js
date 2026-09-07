/**
 * The join flow commits nothing until Join, and sends what it previewed.
 *
 * Two properties, both named in #80 and neither tested. They matter for
 * different reasons.
 *
 * **Nothing joins early.** Reading a code is meant to be a look, not a
 * decision: `preview_invite` exists so an expired or truncated ticket is
 * refused before anything is committed to. If Continue joined, the dialog's
 * Back button would be an offer it cannot honour — a ticket is single use, so
 * the space would already have been entered and the code already spent.
 *
 * **What was previewed is what is sent.** The confirm step shows the space
 * name, the inviting device and the expiry, and a person reads those and then
 * presses Join. If the ticket could change underneath that, the screen would
 * be describing one invite while the button acted on another. The code
 * captures the ticket at preview time in a variable rather than re-reading
 * the field, and this is the test of that: the field is edited after the
 * preview, and the join must ignore the edit.
 *
 * The second check fails against the obvious wrong implementation — reading
 * `join-input` again inside the Join handler — which was confirmed by writing
 * it that way and watching this go red, not assumed.
 */
const report = [];
const $ = (id) => document.getElementById(id);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const called = (cmd) => window.__calls.filter((c) => c.cmd === cmd);

(async () => {
  await sleep(50);

  const previewed = "clispeak://join/THE-ONE-THAT-WAS-PREVIEWED";
  const edited = "clispeak://join/TYPED-AFTER-THE-PREVIEW";

  $("join-open").click();
  await sleep(30);
  report.push(["the dialog opens", !$("join-modal").hidden]);

  $("join-input").value = previewed;
  $("join-read").click();
  await sleep(60);

  report.push(["Continue previews the invite", called("preview_invite").length === 1]);
  report.push([
    "Continue does not join",
    called("join_space").length === 0,
    "a ticket is single use, so joining here would spend it before anyone agreed",
  ]);
  report.push(["it moves to the confirm step", !$("join-step-confirm").hidden]);
  report.push(["and says which space", $("join-space").textContent === "home", $("join-space").textContent]);

  // The field changes after the preview. Everything on screen still describes
  // the invite that was read.
  $("join-input").value = edited;

  $("join-go").click();
  await sleep(60);

  const joins = called("join_space");
  report.push(["Join joins, once", joins.length === 1, `${joins.length} call(s)`]);
  report.push([
    "it sends the ticket that was previewed",
    joins.length === 1 && joins[0].args && joins[0].args.ticket === previewed,
    joins.length === 1 ? String(joins[0].args && joins[0].args.ticket) : "no call",
  ]);
  report.push([
    "and not whatever the field says now",
    joins.length === 1 && joins[0].args && joins[0].args.ticket !== edited,
  ]);

  const pre = document.createElement("pre");
  pre.id = "report";
  pre.textContent = report
    .map(([what, ok, note]) => `${ok ? "PASS" : "FAIL"}  ${what}${note ? `  (${note})` : ""}`)
    .join("\n");
  document.body.appendChild(pre);
})();
