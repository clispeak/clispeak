// Thin UI over the node running in this app's Rust side. No framework and no
// bundler: the interesting behaviour lives in voicecast-core, shared with the
// CLI, and this file only moves values between it and the DOM.
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);

/** How often to re-read node state. Cheap, and keeps devices current. */
const REFRESH_MS = 5000;

/**
 * Show a transient message.
 *
 * `tone` is "info" or "error"; errors persist until the next action rather
 * than timing out, so a failure cannot vanish before it is read.
 */
function say(text, tone = "info") {
  const el = $("result");
  el.textContent = text;
  el.className =
    tone === "error"
      ? "min-h-5 text-center text-sm text-red-600 dark:text-red-400"
      : "min-h-5 text-center text-sm text-neutral-500 dark:text-neutral-400";
}

/** Call a Tauri command, surfacing failures rather than swallowing them. */
async function call(cmd, args) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    say(String(e), "error");
    throw e;
  }
}

/** Run an async action with the button disabled, so it cannot double-fire. */
async function withButton(button, label, action) {
  const original = button.textContent;
  button.disabled = true;
  button.textContent = label;
  try {
    await action();
  } catch {
    // `call` has already reported it.
  } finally {
    button.disabled = false;
    button.textContent = original;
  }
}

/** A rough, readable age. Precision past "minutes" helps nobody here. */
function describeAge(secs) {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.round(secs / 60)}m`;
  if (secs < 86400) return `${Math.round(secs / 3600)}h`;
  return `${Math.round(secs / 86400)}d`;
}

/** One row in the device list. */
function deviceRow(device) {
  const li = document.createElement("li");
  li.className = "flex items-center gap-3 px-4 py-3";

  // A dot rather than words: this is glanceable status, and the tooltip
  // carries the detail for anyone who wants it. Three states, because "not
  // seen yet" is genuinely different from "seen, but a while ago".
  const secs = device.last_seen_secs;
  const live = secs != null && secs < 180;
  const dot = document.createElement("span");
  dot.className =
    "size-2 shrink-0 rounded-full " +
    (live
      ? "bg-emerald-500"
      : secs == null
        ? "bg-neutral-300 dark:bg-neutral-700"
        : "bg-neutral-400 dark:bg-neutral-600");
  dot.title =
    secs == null ? "not seen yet" : live ? "active" : `last seen ${describeAge(secs)} ago`;

  const left = document.createElement("div");
  left.className = "min-w-0 flex-1";
  const name = document.createElement("p");
  name.className = "truncate text-sm font-medium";
  name.textContent = device.name;
  const id = document.createElement("p");
  id.className = "truncate font-mono text-xs text-neutral-500 dark:text-neutral-400";
  id.textContent = device.endpoint_id.slice(0, 16) + "…";
  left.append(name, id);

  li.append(dot, left);

  if (device.is_self) {
    const tag = document.createElement("span");
    tag.className =
      "shrink-0 rounded-full bg-neutral-200 px-2 py-0.5 text-xs text-neutral-600 " +
      "dark:bg-neutral-800 dark:text-neutral-400";
    tag.textContent = "this device";
    li.append(tag);
  } else {
    // Removing another device is destructive and easy to hit by accident on a
    // phone, so it asks first.
    const remove = document.createElement("button");
    remove.className =
      "shrink-0 rounded-lg px-2 py-1 text-xs text-neutral-500 transition " +
      "hover:bg-red-50 hover:text-red-600 dark:text-neutral-400 " +
      "dark:hover:bg-red-500/10 dark:hover:text-red-400";
    remove.textContent = "Remove";
    remove.onclick = () =>
      withButton(remove, "…", async () => {
        if (!confirm(`Remove ${device.name} from this space?`)) return;
        say(await call("revoke_device", { name: device.name }));
        await refresh();
      });
    li.append(remove);
  }
  return li;
}

async function refresh() {
  // The node starts asynchronously, so early polls fail while it comes up.
  // That is a transient state, not an error worth painting red.
  let status;
  try {
    status = await invoke("node_status");
  } catch {
    $("ident").textContent = "starting…";
    return;
  }

  $("name").textContent = status.name;
  $("ident").textContent = status.device_id.slice(0, 20) + "…";
  if (document.activeElement !== $("name-input")) {
    $("name-input").value = status.name;
  }

  const pill = $("engine-pill");
  pill.textContent = status.engine;
  pill.className =
    "shrink-0 rounded-full px-2.5 py-1 text-xs font-medium " +
    (status.fallback
      ? "bg-amber-100 text-amber-800 dark:bg-amber-500/15 dark:text-amber-300"
      : "bg-emerald-100 text-emerald-800 dark:bg-emerald-500/15 dark:text-emerald-300");

  // Only shown where it means something: desktop always reports true.
  try {
    $("battery").hidden = await invoke("battery_ok");
  } catch {
    $("battery").hidden = true;
  }

  await refreshVoice();
  await refreshPolicy();
  await refreshSpaces();
  await refreshHistory();

  const devices = await invoke("list_devices").catch(() => null);
  if (!devices) return;
  const list = $("devices");
  list.replaceChildren(
    ...(devices.length
      ? devices.map(deviceRow)
      : [
          Object.assign(document.createElement("li"), {
            className: "px-4 py-3 text-sm text-neutral-500 dark:text-neutral-400",
            textContent: "No devices yet — add one below.",
          }),
        ]),
  );
}

/** A clock time for today, or a date for anything older. */
function whenSaid(unixSeconds) {
  const at = new Date(unixSeconds * 1000);
  const today = new Date();
  const sameDay =
    at.getFullYear() === today.getFullYear() &&
    at.getMonth() === today.getMonth() &&
    at.getDate() === today.getDate();
  return sameDay
    ? at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : at.toLocaleDateString([], { month: "short", day: "numeric" }) +
        " " +
        at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** How a status should read to a person. */
const STATUS_WORDS = {
  spoken: "spoken",
  queued: "queued",
  speaking: "speaking",
  muted: "muted",
  quiet_hours: "quiet hours",
  no_engine: "no engine",
  unreachable: "unreachable",
  rejected: "rejected",
  cancelled: "cancelled",
  dropped: "dropped",
};

/**
 * One message in the history.
 *
 * The text is clamped to two lines and expands on a tap. A message can be any
 * length, and a list where one entry fills the screen is not a list.
 */
function historyRow(entry) {
  const li = document.createElement("li");
  li.className = "px-4 py-3";

  const head = document.createElement("div");
  head.className = "flex items-center gap-2";

  const who = document.createElement("span");
  who.className = "truncate text-xs font-medium text-neutral-600 dark:text-neutral-300";
  who.textContent = entry.from;

  const when = document.createElement("span");
  when.className = "shrink-0 text-xs text-neutral-400 dark:text-neutral-500";
  when.textContent = whenSaid(entry.at);

  const state = document.createElement("span");
  const word = STATUS_WORDS[entry.status] ?? entry.status;
  state.className =
    "shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium " +
    (entry.unheard
      ? "bg-amber-100 text-amber-800 dark:bg-amber-500/15 dark:text-amber-300"
      : "bg-neutral-100 text-neutral-600 dark:bg-neutral-800 dark:text-neutral-400");
  state.textContent = word;

  const spacer = document.createElement("span");
  spacer.className = "flex-1";

  const play = document.createElement("button");
  play.className =
    "shrink-0 rounded-lg border border-neutral-300 px-2 py-1 text-xs transition " +
    "hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800";
  play.textContent = "Play";
  play.title = "Play this message, even while muted";
  play.onclick = (e) => {
    e.stopPropagation();
    withButton(play, "…", async () => {
      await call("replay", { msgId: entry.msg_id });
      say("playing");
      // The status changes to spoken once it plays, so pick that up.
      setTimeout(refreshHistory, 1500);
    });
  };

  head.append(who, when, state, spacer, play);

  const body = document.createElement("p");
  // Two lines until tapped. Long messages are common and the list has to stay
  // scannable.
  body.className =
    "mt-1 line-clamp-2 cursor-pointer text-sm text-neutral-800 dark:text-neutral-200";
  body.textContent = entry.text;
  body.onclick = () => body.classList.toggle("line-clamp-2");

  li.append(head, body);
  return li;
}

/**
 * Show what this device was asked to say.
 *
 * The reason this exists is the muted case: a message refused while the
 * device was silent is otherwise gone, and this is the only place it can be
 * read or played back.
 */
async function refreshHistory() {
  let entries;
  try {
    entries = await invoke("history", { limit: 50 });
  } catch {
    return;
  }
  if ($("history-unheard").checked) {
    entries = entries.filter((e) => e.unheard);
  }
  const list = $("history");
  list.replaceChildren(
    ...(entries.length
      ? entries.map(historyRow)
      : [
          Object.assign(document.createElement("li"), {
            className: "px-4 py-3 text-sm text-neutral-500 dark:text-neutral-400",
            textContent: $("history-unheard").checked
              ? "Nothing unheard."
              : "Nothing yet.",
          }),
        ]),
  );
}

/** One row in the spaces list. */
function spaceRow(space, several) {
  const li = document.createElement("li");
  li.className = "flex items-center gap-3 px-4 py-3";

  const text = document.createElement("div");
  text.className = "min-w-0 flex-1";
  const name = document.createElement("p");
  name.className = "truncate text-sm font-medium";
  name.textContent = space.label;
  const detail = document.createElement("p");
  detail.className = "text-xs text-neutral-500 dark:text-neutral-400";
  detail.textContent =
    `${space.devices} device${space.devices === 1 ? "" : "s"}` +
    (space.is_default ? " · default" : "");
  text.append(name, detail);
  li.append(text);

  // Only offered where it would do something. A single space is always the
  // default and cannot be dropped, so both controls would be dead.
  if (!space.is_default) {
    const useIt = document.createElement("button");
    useIt.className =
      "shrink-0 rounded-lg border border-neutral-300 px-3 py-1 text-xs transition " +
      "hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800";
    useIt.textContent = "Use";
    useIt.onclick = () =>
      withButton(useIt, "…", async () => {
        await call("default_space", { label: space.label });
        say(`bare names now mean ${space.label}`);
        await refresh();
      });
    li.append(useIt);
  }
  if (several) {
    const drop = document.createElement("button");
    drop.className =
      "shrink-0 rounded-lg px-2 py-1 text-xs text-red-600 transition " +
      "hover:bg-red-50 dark:text-red-400 dark:hover:bg-red-500/10";
    drop.textContent = "Drop";
    drop.onclick = () =>
      withButton(drop, "…", async () => {
        if (!confirm(`Drop the space "${space.label}"?`)) return;
        await call("drop_space", { label: space.label });
        say(`dropped ${space.label}`);
        await refresh();
      });
    li.append(drop);
  }
  return li;
}

/**
 * Show the spaces this device belongs to.
 *
 * Always shown, even for the single space most devices have: this is where a
 * second one is created, and hiding the section until there are two made that
 * impossible to reach.
 */
async function refreshSpaces() {
  let spaces;
  try {
    spaces = await invoke("list_spaces");
  } catch {
    return;
  }
  $("spaces-section").hidden = false;
  const several = spaces.length > 1;
  $("spaces").replaceChildren(...spaces.map((s) => spaceRow(s, several)));
}

/** Minutes past midnight for an `HH:MM` string, or null if it is not one. */
function minutesOf(text) {
  const m = /^(\d{1,2}):(\d{2})$/.exec(text ?? "");
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  return h > 23 || min > 59 ? null : h * 60 + min;
}

/**
 * Whether `minute` falls inside a window that may cross midnight.
 *
 * Mirrors `QuietHours::contains` in the node, including treating a window
 * whose ends are equal as empty. The two answers have to agree, or the
 * interface says one thing while the device does another.
 */
function insideWindow(from, to, minute) {
  if (from === to) return false;
  return from < to ? minute >= from && minute < to : minute >= from || minute < to;
}

/**
 * Show whether this device will speak, and why not when it will not.
 *
 * The banner is the point of the section. Both controls can be set to
 * something reasonable and the device still be silent right now, and a person
 * wondering why nothing is coming out deserves to be told rather than left to
 * work it out from a clock and two time fields.
 */
async function refreshPolicy() {
  let policy;
  try {
    policy = await invoke("policy");
  } catch {
    return;
  }

  if (document.activeElement !== $("mute")) $("mute").checked = policy.muted;

  const hasWindow = policy.from != null && policy.to != null;
  if (document.activeElement !== $("quiet-on")) $("quiet-on").checked = hasWindow;
  $("quiet-controls").hidden = !$("quiet-on").checked;
  if (hasWindow) {
    if (document.activeElement !== $("quiet-from")) $("quiet-from").value = policy.from;
    if (document.activeElement !== $("quiet-to")) $("quiet-to").value = policy.to;
  }
  if (document.activeElement !== $("quiet-high")) {
    $("quiet-high").checked = policy.high_breaks_through;
  }

  const now = new Date();
  const minute = now.getHours() * 60 + now.getMinutes();
  const from = minutesOf(policy.from);
  const to = minutesOf(policy.to);
  const quietNow = from != null && to != null && insideWindow(from, to, minute);

  const banner = $("silent-banner");
  if (policy.muted) {
    banner.textContent = "This device is muted. Nothing will be spoken.";
    banner.hidden = false;
  } else if (quietNow && policy.high_breaks_through) {
    banner.textContent = "Quiet hours — only urgent messages will be spoken.";
    banner.hidden = false;
  } else if (quietNow) {
    banner.textContent = `Quiet hours until ${policy.to}. Nothing will be spoken.`;
    banner.hidden = false;
  } else {
    banner.hidden = true;
  }
}

/** Send the quiet window as the controls currently read. */
async function saveQuiet() {
  const on = $("quiet-on").checked;
  await call("set_quiet", {
    from: on ? $("quiet-from").value : null,
    to: on ? $("quiet-to").value : null,
    highBreaksThrough: $("quiet-high").checked,
  });
  await refreshPolicy();
}

/**
 * Show what this device's engine can be set to.
 *
 * Hidden entirely when there is nothing to choose — an engine with one voice
 * and no rate control should not present an empty panel.
 */
async function refreshVoice() {
  let config;
  try {
    config = await invoke("voice_config");
  } catch {
    $("voice-section").hidden = true;
    return;
  }

  const choices = config.available ?? [];
  $("voice-section").hidden = choices.length === 0;
  $("voice-picker-wrap").hidden = choices.length < 2;

  const picker = $("voice-picker");
  // Left alone while open, or the list would close under the user's finger.
  if (document.activeElement !== picker) {
    picker.replaceChildren(
      ...choices.map((v) =>
        Object.assign(document.createElement("option"), {
          value: v.id,
          textContent: v.name,
          selected: v.id === config.current,
        }),
      ),
    );
  }

  if (document.activeElement !== $("rate")) {
    $("rate").value = config.rate;
  }
  $("rate-value").textContent = `${Number(config.rate).toFixed(2)}×`;
}

$("mute").onchange = async () => {
  await call("set_mute", { muted: $("mute").checked });
  say($("mute").checked ? "muted" : "unmuted");
  await refreshPolicy();
};

$("quiet-on").onchange = async () => {
  $("quiet-controls").hidden = !$("quiet-on").checked;
  await saveQuiet();
  say($("quiet-on").checked ? "quiet hours set" : "quiet hours off");
};

// `change` rather than `input`: a time field fires while it is being typed
// into, and saving a half-entered "0:" would clear the window.
$("quiet-from").onchange = saveQuiet;
$("quiet-to").onchange = saveQuiet;
$("quiet-high").onchange = saveQuiet;

$("voice-picker").onchange = async () => {
  await call("set_voice", { id: $("voice-picker").value });
  say("voice changed");
  await refreshVoice();
};

// Fires continuously while dragging, so the label tracks the thumb but the
// engine is only told when the finger lifts.
$("rate").oninput = () => {
  $("rate-value").textContent = `${Number($("rate").value).toFixed(2)}×`;
};

$("rate").onchange = async () => {
  await call("set_rate", { rate: Number($("rate").value) });
  await refreshVoice();
};

$("preview").onclick = () =>
  withButton($("preview"), "…", async () => {
    await call("speak", { text: "This is how this device sounds." });
  });

$("speak-form").onsubmit = async (e) => {
  e.preventDefault();
  const text = $("say-input").value.trim();
  if (!text) return;
  await withButton($("say"), "…", async () => {
    await call("speak", { text });
    $("say-input").value = "";
    say("spoken");
  });
};

$("invite").onclick = () =>
  withButton($("invite"), "…", async () => {
    const { url, qr, expires_in } = await call("make_invite");
    // Trusted: the SVG is produced by our own Rust side, not user input.
    $("qr").innerHTML = qr;
    $("qr").hidden = !qr;
    $("ticket").textContent = url;
    $("expiry").textContent = `Expires in ${Math.max(1, Math.round(expires_in / 60))} min. Single use.`;
    $("invite-out").hidden = false;
    say("");
  });

$("copy").onclick = async () => {
  try {
    await navigator.clipboard.writeText($("ticket").textContent);
    say("copied");
  } catch {
    say("could not copy — select the code instead", "error");
  }
};

$("join-form").onsubmit = async (e) => {
  e.preventDefault();
  await withButton($("join"), "…", async () => {
    const members = await call("join_space", { ticket: $("join-input").value });
    $("join-input").value = "";
    say(`joined — ${members} devices`);
    await refresh();
  });
};

$("rename-form").onsubmit = async (e) => {
  e.preventDefault();
  const name = $("name-input").value.trim();
  if (!name) {
    say("a device needs a name", "error");
    return;
  }
  await withButton($("rename"), "…", async () => {
    await call("rename_device", { name });
    say(`renamed to ${name}`);
    await refresh();
  });
};

$("leave").onclick = () =>
  withButton($("leave"), "…", async () => {
    if (!confirm("Leave this space? Other devices will stop reaching this one.")) return;
    say(await call("leave_space"));
    await refresh();
  });

$("history-unheard").onchange = refreshHistory;

$("history-clear").onclick = () =>
  withButton($("history-clear"), "…", async () => {
    if (!confirm("Forget every message in the history?")) return;
    await call("clear_history");
    await refreshHistory();
  });

$("new-space-form").onsubmit = async (e) => {
  e.preventDefault();
  const label = $("new-space").value.trim();
  if (!label) return;
  await call("new_space", { label });
  $("new-space").value = "";
  say(`created ${label}`);
  await refresh();
};

$("rotate").onclick = () =>
  withButton($("rotate"), "…", async () => {
    if (
      !confirm(
        "Replace this space? Every other device is locked out immediately " +
          "and has to be invited again.",
      )
    )
      return;
    const left = await call("rotate_space");
    say(
      left.length
        ? `space replaced — re-invite ${left.join(", ")}`
        : "space replaced",
    );
    await refresh();
  });

$("battery-fix").onclick = () =>
  withButton($("battery-fix"), "…", async () => {
    await call("request_battery_exemption");
    say("choose Allow, then come back");
  });

/**
 * Join automatically when the app was opened by scanning an invite.
 *
 * Polled rather than pushed: the scan can land before this script exists, so
 * Kotlin parks the value and this collects it.
 */
async function collectScannedInvite() {
  let ticket;
  try {
    ticket = await invoke("pending_invite");
  } catch {
    return;
  }
  if (!ticket) return;
  try {
    const members = await invoke("join_space", { ticket });
    say(`joined from a scan — ${members} devices`);
    await refresh();
  } catch (e) {
    say(String(e), "error");
  }
}

refresh();
collectScannedInvite();
setInterval(() => {
  refresh();
  collectScannedInvite();
}, REFRESH_MS);
