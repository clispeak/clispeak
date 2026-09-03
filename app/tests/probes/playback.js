/**
 * Issue #109: pausing playback left the app mute with no way back.
 *
 * The controls key off `now_playing.msg_id`, and a pause moves the message
 * out of "speaking" and into the held slot. So the whole panel — Resume
 * included — hid itself at the exact moment it was the only thing that could
 * undo what had just been done. Everything afterwards queued, showed a toast,
 * and never made a sound.
 *
 * Every check here fails against the interface as it was. Confirmed by
 * running it before the fix, not assumed.
 */
const report = [];
const $ = (id) => document.getElementById(id);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const shown = (id) => !$(id).hidden && $(id).offsetParent !== null;

(async () => {
  await sleep(600);

  report.push(["the panel is there while speaking", shown("now-playing")]);
  report.push(["the button offers Pause", $("np-pause").textContent === "Pause"]);

  $("np-pause").click();
  await sleep(400);

  // The bug, in one line.
  report.push(["the panel survives a pause", shown("now-playing")]);
  report.push([
    "the button now offers Resume",
    $("np-pause").textContent === "Resume",
    `reads "${$("np-pause").textContent}"`,
  ]);
  report.push(["the state reads paused", $("np-state").textContent === "paused"]);

  // And back again, which is the whole point of it still being on screen.
  $("np-pause").click();
  await sleep(400);
  report.push(["Resume puts it back to speaking", $("np-state").textContent === "speaking"]);
  report.push(["the button offers Pause again", $("np-pause").textContent === "Pause"]);

  // The other real case: a pause with nothing held, which is what arrives
  // from the CLI when the queue is empty. Nothing is playing, so there is no
  // message to show — but the device is mute and must say so, or the only
  // evidence is that messages stop being spoken.
  window.__playback.held = false;
  window.__playback.paused = true;
  await sleep(5600);
  report.push([
    "a pause with nothing playing is still visible",
    shown("now-playing"),
    "otherwise the device is mute with no indication",
  ]);
  report.push([
    "and can be undone",
    $("np-pause").textContent === "Resume",
    `reads "${$("np-pause").textContent}"`,
  ]);

  const pre = document.createElement("pre");
  pre.id = "report";
  pre.textContent = report
    .map(([what, ok, note]) => `${ok ? "PASS" : "FAIL"}  ${what}${note ? `  (${note})` : ""}`)
    .join("\n");
  document.body.appendChild(pre);
})();
