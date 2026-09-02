// Thin UI over the node running in this app's Rust side. No framework and no
// bundler: the interesting behaviour lives in voicecast-core, shared with the
// CLI, and this file only moves values between it and the DOM.
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);

/** How often to re-read node state. Cheap, and keeps devices current. */
const REFRESH_MS = 5000;

/**
 * How often to re-read what is being spoken.
 *
 * Faster than the rest, because these are controls for something happening
 * right now: a stop button that takes five seconds to notice the message
 * ended is a button people press twice.
 */
const PLAYING_MS = 1000;

/** Which screen is showing. */
let screen = "home";

/**
 * Show one screen and mark its tab.
 *
 * Three screens rather than one long column: the app is mostly a receiver, and
 * what it is saying and what it has said are what you open it for. Who can
 * reach it is checked and changed often enough to deserve its own tab, and
 * what is set once and left alone belongs behind the last one.
 */
function showScreen(name) {
  screen = name;
  $("screen-home").hidden = name !== "home";
  $("screen-spaces").hidden = name !== "spaces";
  $("screen-settings").hidden = name !== "settings";
  for (const tab of [$("tab-home"), $("tab-spaces"), $("tab-settings")]) {
    const active = tab.dataset.screen === name;
    tab.className =
      "flex flex-1 flex-col items-center gap-0.5 py-2.5 text-[11px] font-medium transition " +
      (active
        ? "text-accent-600 dark:text-accent-400"
        : "text-neutral-500 hover:text-neutral-700 dark:text-neutral-400 dark:hover:text-neutral-200");
    tab.setAttribute("aria-current", active ? "page" : "false");
  }
}

/**
 * Show a transient message.
 *
 * `tone` is "info" or "error"; errors persist until the next action rather
 * than timing out, so a failure cannot vanish before it is read.
 */
/** How long an ordinary message stays up before fading. */
const SAY_MS = 3500;

let sayTimer = null;

function say(text, tone = "info") {
  const el = $("result");
  if (sayTimer) {
    clearTimeout(sayTimer);
    sayTimer = null;
  }
  if (!text) {
    el.hidden = true;
    return;
  }
  el.textContent = text;
  // Rebuilt in full each time, since the tone changes the whole palette.
  el.className =
    "max-w-sm rounded-full border px-4 py-2 text-center text-sm shadow-lg " +
    (tone === "error"
      ? "border-red-300 bg-red-50 text-red-700 " +
        "dark:border-red-500/40 dark:bg-red-950 dark:text-red-300"
      : "border-neutral-300 bg-white text-neutral-700 " +
        "dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-200");
  el.hidden = false;
  // Errors stay until something replaces them; a confirmation that has been
  // read is only clutter.
  if (tone !== "error") {
    sayTimer = setTimeout(() => {
      el.hidden = true;
      sayTimer = null;
    }, SAY_MS);
  }
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

/**
 * Ask before doing something destructive. Resolves true if confirmed.
 *
 * Not `window.confirm`. WKWebView shows a script dialog only if the host
 * implements a `WKUIDelegate` for it, and wry implements none — so on macOS
 * `confirm()` displayed nothing and returned false, and every action behind
 * one silently did nothing while reporting success. An in-page dialog behaves
 * the same on all five targets, which a native one never did.
 *
 * Escape and the backdrop cancel, because the safe answer should be the
 * easiest one to give.
 */
function ask(question, { input = null } = {}) {
  const box = $("ask");
  const field = $("ask-input");
  $("ask-text").textContent = question;
  field.hidden = input === null;
  field.value = input ?? "";
  box.hidden = false;
  if (input === null) $("ask-yes").focus();
  else {
    field.focus();
    field.select();
  }

  return new Promise((resolve) => {
    const done = (answer) => {
      box.hidden = true;
      $("ask-yes").onclick = null;
      $("ask-no").onclick = null;
      field.onkeydown = null;
      box.onclick = null;
      document.removeEventListener("keydown", onKey);
      resolve(answer);
    };
    const onKey = (e) => {
      if (e.key === "Escape") done(input === null ? false : null);
    };
    // With a field, the answer is what was typed; without one, it is yes or no.
    const yes = () => done(input === null ? true : field.value.trim() || null);
    $("ask-yes").onclick = yes;
    $("ask-no").onclick = () => done(input === null ? false : null);
    // Enter accepts, so a rename does not need the mouse.
    field.onkeydown = (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        yes();
      }
    };
    // Only the backdrop itself, so a click inside the panel does not cancel.
    box.onclick = (e) => {
      if (e.target === box) done(false);
    };
    document.addEventListener("keydown", onKey);
  });
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
function deviceRow(device, space) {
  const li = document.createElement("div");
  li.className =
    "flex items-center gap-3 border-t border-neutral-200 px-3 py-2.5 " +
    "dark:border-neutral-800";

  // A dot for the glance, with the same thing in words underneath. Three
  // states, because "not seen yet" is genuinely different from "seen, but a
  // while ago".
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
  // Said in words as well as shown as a dot. A tooltip needs a pointer, and
  // half the devices running this are phones.
  const when = document.createElement("p");
  when.className = "truncate text-xs text-neutral-500 dark:text-neutral-400";
  when.textContent = device.is_self
    ? "this device"
    : secs == null
      ? "not seen yet"
      : live
        ? "active now"
        : `last seen ${describeAge(secs)} ago`;
  left.append(name, when);

  li.append(dot, left);

  // Removing names its space now, so it is offered on every card rather than
  // only the default one.
  if (!device.is_self) {
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
        if (!(await ask(`Remove ${device.name} from this space?`))) return;
        say(await call("revoke_device", { name: device.name, space }));
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
  await refreshSkill();
  await refreshHistory();

}

/**
 * Show what this device is saying, with the controls to stop it.
 *
 * The reason this exists on the receiving device: a message already playing
 * could only be stopped from another machine or a terminal, which is no use
 * when the phone in your hand is the thing talking.
 */
async function refreshPlaying() {
  let playing;
  try {
    playing = await invoke("now_playing");
  } catch {
    return;
  }

  const active = playing.msg_id != null;
  $("now-playing").hidden = !active;
  if (!active) return;

  $("np-state").textContent = playing.paused ? "paused" : "speaking";
  $("np-state").className =
    "shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium text-white " +
    (playing.paused ? "bg-neutral-500" : "bg-accent-600");
  $("np-from").textContent = playing.from ? `from ${playing.from}` : "";
  $("np-text").textContent = playing.text ?? "";
  $("np-waiting").textContent = playing.waiting
    ? `${playing.waiting} waiting`
    : "";
  $("np-pause").textContent = playing.paused ? "Resume" : "Pause";
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

/**
 * Show whether the agent skill is installed, and where.
 *
 * Hidden where it means nothing: an agent runs on a computer, not on the
 * phone that receives the speech.
 */
async function refreshSkill() {
  let skill;
  try {
    skill = await invoke("skill_status");
  } catch {
    return;
  }
  if (!skill) {
    $("skill-section").hidden = true;
    return;
  }
  $("skill-section").hidden = false;

  const badge = $("skill-state");
  const words = {
    current: ["installed", "bg-emerald-100 text-emerald-800 dark:bg-emerald-500/15 dark:text-emerald-300"],
    stale: ["out of date", "bg-amber-100 text-amber-800 dark:bg-amber-500/15 dark:text-amber-300"],
    absent: ["not installed", "bg-neutral-100 text-neutral-600 dark:bg-neutral-800 dark:text-neutral-400"],
  };
  const [word, colour] = words[skill.state] ?? words.absent;
  badge.textContent = word;
  badge.className = "shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium " + colour;
  $("skill-note").textContent =
    skill.state === "stale" ? "this app has a newer version" : "";

  if (document.activeElement !== $("skill-path")) {
    $("skill-path").value = skill.path;
  }
  // Said plainly rather than discovered by a failed install: inside a
  // sandbox the app genuinely cannot write outside the default location.
  $("skill-hint").textContent = skill.sandboxed
    ? "This app is sandboxed. For anywhere else, run: voicecast skill --install --path <dir>/SKILL.md"
    : "Any path. Agents that keep skills elsewhere can be pointed at their own directory.";
  $("skill-install").textContent =
    skill.state === "absent" ? "Install the skill" : "Reinstall";
}

/** One row in the spaces list. */
/** A small bordered button, the shape used throughout a space card. */
function cardButton(label, { danger = false } = {}) {
  const b = document.createElement("button");
  b.className =
    "shrink-0 rounded-lg px-2.5 py-1 text-xs transition " +
    "focus-visible:outline-2 focus-visible:outline-offset-2 " +
    (danger
      ? "text-red-600 hover:bg-red-50 dark:text-red-400 dark:hover:bg-red-500/10"
      : "border border-neutral-300 hover:bg-neutral-100 " +
        "dark:border-neutral-700 dark:hover:bg-neutral-800");
  b.textContent = label;
  return b;
}

/**
 * One space, with the devices in it.
 *
 * The whole point of the layout: a device belongs to exactly one space, so it
 * is drawn inside that space rather than in a list beside it.
 *
 * Only the default space can be fully managed. Inviting, removing a device and
 * replacing a space all act on `spaces.current()` in the node, so offering them
 * on another card would act on the wrong space — see issue #14. Rather than
 * silently doing the wrong thing, the other cards say what to do instead.
 */
function spaceCard(space, devices, several) {
  const card = document.createElement("div");
  card.className =
    "overflow-hidden rounded-xl border border-neutral-200 bg-white " +
    "dark:border-neutral-800 dark:bg-neutral-900";

  // A tinted header, so the space reads as holding what follows rather than
  // being the first row of it. Tinted on both themes: on a dark ground a
  // lighter fill separates where a shadow cannot.
  const head = document.createElement("div");
  head.className =
    "flex items-center gap-2 border-b border-neutral-200 bg-neutral-50 px-3 py-2.5 " +
    "dark:border-neutral-800 dark:bg-neutral-800/60";

  const title = document.createElement("p");
  title.className = "min-w-0 flex-1 truncate text-sm font-semibold";
  title.textContent = space.label;
  head.append(title);

  if (space.is_default) {
    const badge = document.createElement("span");
    badge.className =
      "shrink-0 rounded-full bg-accent-500/15 px-2 py-0.5 text-xs font-medium " +
      "text-accent-600 dark:text-accent-400";
    badge.textContent = "default";
    head.append(badge);
  } else {
    const use = cardButton("Make default");
    use.onclick = () =>
      withButton(use, "…", async () => {
        await call("default_space", { label: space.label });
        say(`bare names now mean ${space.label}`);
        await refresh();
      });
    head.append(use);
  }
  card.append(head);

  // Devices. Only the default space's members can be told apart from the rest,
  // because `list_devices` reports a space only when there is more than one.
  const mine = devices.filter((d) =>
    d.space == null ? space.is_default : d.space === space.label,
  );
  if (mine.length) {
    for (const device of mine) card.append(deviceRow(device, space.label));
  } else {
    const none = document.createElement("p");
    none.className = "px-3 py-3 text-sm text-neutral-500 dark:text-neutral-400";
    none.textContent = "No devices yet.";
    card.append(none);
  }

  const foot = document.createElement("div");
  foot.className =
    "flex flex-wrap gap-2 border-t border-neutral-200 px-3 py-2 " +
    "dark:border-neutral-800";

  const rename = cardButton("Rename");
  rename.onclick = () =>
    withButton(rename, "…", async () => {
      const to = await ask(`New name for "${space.label}"`, {
        input: space.label,
      });
      if (!to || to === space.label) return;
      await call("rename_space", { label: space.label, to });
      say(`renamed to ${to}`);
      await refresh();
    });
  foot.append(rename);

  // Inviting names its space, so the button can sit on the card it belongs
  // to rather than acting on whichever space happens to be default.
  const add = cardButton(`Add a device`);
  add.onclick = () =>
    withButton(add, "…", async () => {
      await showInvite(space.label);
    });
  foot.append(add);

  // Leaving, replacing and inviting all name their space now, so every card
  // gets the same controls rather than only the default one.
  const leave = cardButton(`Leave ${space.label}`, { danger: true });
  leave.onclick = () =>
    withButton(leave, "…", async () => {
      if (
        !(await ask(
          `Leave ${space.label}? The other devices are told, and stop ` +
            "reaching this one.",
        ))
      )
        return;
      say(await call("leave_space", { space: space.label }));
      await refresh();
    });

  const rotate = cardButton(`Replace ${space.label}`, { danger: true });
  rotate.onclick = () =>
    withButton(rotate, "…", async () => {
      if (
        !(await ask(
          `Replace ${space.label}? Every other device is locked out ` +
            "immediately and has to be invited again.",
        ))
      )
        return;
      const left = await call("rotate_space", { space: space.label });
      say(
        left.length
          ? `space replaced — re-invite ${left.join(", ")}`
          : "space replaced",
      );
      await refresh();
    });
  foot.append(leave, rotate);

  // Only where the difference matters. Forgetting is the escape hatch for a
  // space whose other devices cannot be reached to be told — it removes the
  // space here and sends nothing, so they go on counting this device a
  // member. Leaving is what you want otherwise.
  if (several) {
    const forget = cardButton("Forget on this device", { danger: true });
    forget.onclick = () =>
      withButton(forget, "…", async () => {
        if (
          !(await ask(
            `Forget ${space.label} on this device? Nothing is sent: the other ` +
              "devices will still count this one as a member.",
          ))
        )
          return;
        await call("drop_space", { label: space.label });
        say(`forgot ${space.label}`);
        await refresh();
      });
    foot.append(forget);
  }
  card.append(foot);
  return card;
}

async function refreshSpaces() {
  const [spaces, devices] = await Promise.all([
    invoke("list_spaces").catch(() => null),
    invoke("list_devices").catch(() => []),
  ]);
  if (!spaces) return;
  const several = spaces.length > 1;
  $("spaces").replaceChildren(
    ...spaces.map((space) => spaceCard(space, devices, several)),
  );
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

/**
 * Show an invite for one space.
 *
 * The space travels on the ticket, so what the button said when it was
 * pressed is what the joining device gets — even if the default changes
 * before the code is scanned.
 */
async function showInvite(space) {
  const { url, qr, expires_in } = await call("make_invite", { space });
  // Trusted: the SVG is produced by our own Rust side, not user input.
  $("qr").innerHTML = qr;
  $("qr").hidden = !qr;
  $("ticket").textContent = url;
  $("invite-for").textContent = space ? `Joins ${space}.` : "";
  $("expiry").textContent = `Expires in ${Math.max(1, Math.round(expires_in / 60))} min. Single use.`;
  $("invite-out").hidden = false;
  // The panel sits below the space cards, so a button pressed on a card can
  // open it several hundred pixels out of view — which looks exactly like
  // nothing happening.
  $("invite-out").scrollIntoView({ behavior: "smooth", block: "center" });
  say("");
}

// The unqualified button invites into the default space. Each space card has
// its own, which names the space it belongs to.
$("invite").onclick = () =>
  withButton($("invite"), "…", async () => {
    await showInvite();
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

$("skill-install").onclick = () =>
  withButton($("skill-install"), "…", async () => {
    const path = await call("install_skill", { path: $("skill-path").value });
    say(`skill installed at ${path}`);
    await refreshSkill();
  });

$("tab-home").onclick = () => showScreen("home");
$("tab-spaces").onclick = () => showScreen("spaces");
$("tab-settings").onclick = () => showScreen("settings");

$("np-pause").onclick = async () => {
  // Read the label rather than tracking state here: the poll owns what is
  // true, and a second source would drift from it.
  const resuming = $("np-pause").textContent === "Resume";
  await call(resuming ? "resume_speech" : "pause_speech");
  await refreshPlaying();
};

$("np-skip").onclick = async () => {
  await call("skip_speech");
  await refreshPlaying();
  await refreshHistory();
};

$("np-stop").onclick = async () => {
  await call("stop_speech");
  await refreshPlaying();
  await refreshHistory();
};

$("history-unheard").onchange = refreshHistory;

$("history-clear").onclick = () =>
  withButton($("history-clear"), "…", async () => {
    if (!(await ask("Forget every message in the history?"))) return;
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

showScreen("home");
refresh();
refreshPlaying();
collectScannedInvite();
setInterval(() => {
  refresh();
  collectScannedInvite();
}, REFRESH_MS);
setInterval(refreshPlaying, PLAYING_MS);
