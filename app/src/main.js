// Thin UI over the node running in this app's Rust side. No framework and no
// bundler: the interesting behaviour lives in voicecast-core, shared with the
// CLI, and this file only moves values between it and the DOM.
import { openModal } from "./modal.js";

const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);

/** How often to re-read node state. Cheap, and keeps devices current. */
const REFRESH_MS = 5000;

/**
 * How deep to read history when filtering for unheard messages.
 *
 * Above what the node retains (200 at the time of writing), so the filter
 * sees everything there is rather than the newest screenful. Coupled to that
 * retention only in the direction that fails safe: too large simply returns
 * everything, too small hides entries.
 */
const HISTORY_DEPTH = 1000;

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
  const status = $("result-status");
  const alert = $("result-alert");
  if (sayTimer) {
    clearTimeout(sayTimer);
    sayTimer = null;
  }
  // Both, always: a confirmation must not leave the previous error on screen,
  // and an error must not sit beside a stale confirmation.
  status.replaceChildren();
  alert.replaceChildren();
  if (!text) return;

  const bubble = Object.assign(document.createElement("p"), {
    textContent: text,
    // Rebuilt in full each time, since the tone changes the whole palette.
    className:
      "max-w-sm rounded-full border px-4 py-2 text-center text-sm shadow-lg " +
      (tone === "error"
        ? "border-red-300 bg-red-50 text-red-700 " +
          "dark:border-red-500/40 dark:bg-red-950 dark:text-red-300"
        : "border-neutral-300 bg-white text-neutral-700 " +
          "dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-200"),
  });
  // Appended into a region that was already there, rather than unhiding the
  // region itself. That difference is the whole of why these were silent.
  (tone === "error" ? alert : status).replaceChildren(bubble);

  // Errors stay until something replaces them; a confirmation that has been
  // read is only clutter.
  if (tone !== "error") {
    sayTimer = setTimeout(() => {
      status.replaceChildren();
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
  const cancelled = input === null ? false : null;
  const box = $("ask");
  const field = $("ask-input");
  $("ask-text").textContent = question;
  field.hidden = input === null;
  field.value = input ?? "";

  return new Promise((resolve) => {
    // Answered rather than left hanging. A second dialog was reachable
    // through the Tab escape this rewrite closes, and it left the first
    // promise unresolved for ever — with the button that opened it stuck
    // reading "…" until the app was restarted.
    const close = openModal(box, {
      focus: input === null ? $("ask-yes") : field,
      onClose: () => resolve(answer),
    });
    if (!close) {
      resolve(cancelled);
      return;
    }

    // Read by `onClose`, so every route out — Escape, the backdrop, either
    // button — resolves exactly once and through one path.
    let answer = cancelled;
    const done = (given) => {
      answer = given;
      close();
    };
    // With a field, the answer is what was typed; without one, it is yes or no.
    const yes = () => done(input === null ? true : field.value.trim() || null);
    $("ask-yes").onclick = yes;
    $("ask-no").onclick = () => done(cancelled);
    // Enter accepts, so a rename does not need the mouse.
    field.onkeydown = (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        yes();
      }
    };
  });
}

/** Run an async action with the button disabled, so it cannot double-fire. */
async function withButton(button, label, action) {
  const original = button.textContent;
  // Disabling the focused element moves focus to `<body>`, and this happens
  // *before* `action` runs — so a dialog opened inside `action` captured
  // `<body>` as its opener and restored focus to nothing when it closed. A
  // keyboard user lost their place in the page every time they confirmed
  // anything (#113). The blur happens here, so the repair belongs here.
  const hadFocus = document.activeElement === button;
  button.disabled = true;
  button.textContent = label;
  try {
    await action();
  } catch {
    // `call` has already reported it.
  } finally {
    button.disabled = false;
    button.textContent = original;
    // Only if focus is still nowhere: anything that has deliberately taken it
    // in the meantime — a dialog still open, a field the action focused —
    // outranks putting it back.
    if (hadFocus && button.isConnected && document.activeElement === document.body) {
      button.focus();
    }
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
/**
 * Put the devices of one space into its card, keeping the rows that survive.
 *
 * Shared by the first build and every poll after it, so there is one answer to
 * what a card's device list looks like rather than two that drift.
 */
function fillDevices(rows, devices, label) {
  const shown = syncRows(rows, devices, {
    key: (d) => d.endpoint_id,
    build: (d) => deviceRow(d, label),
    update: updateDeviceRow,
  });
  if (shown === 0) {
    rows.replaceChildren(
      Object.assign(document.createElement("p"), {
        className: "px-3 py-3 text-sm text-neutral-500 dark:text-neutral-400",
        textContent: "No devices yet.",
      }),
    );
  }
}

/**
 * Bring an existing space card up to date.
 *
 * The label is the key, so it cannot have changed. What can: the default
 * badge, and the whole device list underneath.
 */
function updateSpaceCard(card, space, devices, several) {
  bindCardActions(card, space, several);
  const head = card.firstElementChild;
  const badge = head?.querySelector('[data-part="default"]');
  if (space.is_default && !badge) head?.append(defaultBadge());
  if (!space.is_default && badge) badge.remove();

  const rows = card.querySelector('[data-part="devices"]');
  if (rows) fillDevices(rows, devicesIn(space, devices), space.label);
}

/** Which reported devices belong to a space. */
function devicesIn(space, devices) {
  return devices.filter((d) =>
    d.space == null ? space.is_default : d.space === space.label,
  );
}

/** The badge on the default space. */
function defaultBadge() {
  const badge = document.createElement("span");
  badge.dataset.part = "default";
  badge.className =
    "shrink-0 rounded-full bg-accent-500/15 px-2 py-0.5 text-xs font-medium " +
    "text-accent-600 dark:text-accent-400";
  badge.textContent = "default";
  return badge;
}

function isLive(device) {
  return device.last_seen_secs != null && device.last_seen_secs < 180;
}

/** The presence dot's colour. Three states, split out so a poll can repaint it. */
function dotClass(device) {
  return (
    "size-2 shrink-0 rounded-full " +
    (isLive(device)
      ? "bg-emerald-500"
      : device.last_seen_secs == null
        ? "bg-neutral-300 dark:bg-neutral-700"
        : "bg-neutral-400 dark:bg-neutral-600")
  );
}

/** The dot's tooltip. */
function dotTitle(device) {
  if (device.last_seen_secs == null) return "not seen yet";
  return isLive(device) ? "active" : `last seen ${describeAge(device.last_seen_secs)} ago`;
}

/** Presence in words, since a tooltip needs a pointer and half of these are phones. */
function seenText(device) {
  if (device.is_self) return "this device";
  if (device.last_seen_secs == null) return "not seen yet";
  return isLive(device) ? "active now" : `last seen ${describeAge(device.last_seen_secs)} ago`;
}

/**
 * Bring an existing device row up to date.
 *
 * Presence is the only thing that moves, and it moves on every poll — which
 * is why rebuilding the row instead was losing a focused Remove button every
 * five seconds.
 */
function updateDeviceRow(row, device) {
  const dot = row.querySelector('[data-part="dot"]');
  if (dot) {
    const className = dotClass(device);
    if (dot.className !== className) dot.className = className;
    dot.title = dotTitle(device);
  }
  const seen = row.querySelector('[data-part="seen"]');
  const text = seenText(device);
  if (seen && seen.textContent !== text) seen.textContent = text;
}

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
  dot.dataset.part = "dot";
  dot.className = dotClass(device);
  dot.title = dotTitle(device);

  const left = document.createElement("div");
  left.className = "min-w-0 flex-1";
  const name = document.createElement("p");
  name.className = "truncate text-sm font-medium";
  name.textContent = device.name;
  // Said in words as well as shown as a dot. A tooltip needs a pointer, and
  // half the devices running this are phones.
  const when = document.createElement("p");
  when.dataset.part = "seen";
  when.className = "truncate text-xs text-neutral-500 dark:text-neutral-400";
  when.textContent = seenText(device);
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
    // The command itself being unreachable is a different thing from the
    // node having failed, and is genuinely transient: it means the app is
    // still registering its state.
    $("ident").textContent = "starting…";
    return;
  }

  // Asked first, because none of the rest of this means anything when there
  // is no node. `starting` is transient and says so; `failed` is not, and
  // used to be indistinguishable from it — the window said "starting…" for
  // as long as it was open.
  $("node-banner").hidden = !status.failed;
  $("node-reason").textContent = status.failed ?? "";
  if (status.failed) {
    $("ident").textContent = "not running";
    $("name").textContent = "voicecast";
    return;
  }
  if (status.starting) {
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

  // A device that cannot speak says why, on the screen someone is looking at
  // when they notice it has gone quiet. The reason has always existed; it
  // reached only whoever sent a message and was never heard.
  $("engine-banner").hidden = !status.reason;
  $("engine-reason").textContent = status.reason ?? "";

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
  // Shown while paused even with nothing held, because this panel is the only
  // way back from a pause. Keying it on `active` alone meant that pausing hid
  // the Resume button at the exact moment it was the only control that
  // mattered, and every later message queued, raised a toast, and made no
  // sound (#109). A pause can also arrive from the CLI with an empty queue,
  // and then the only other evidence is that the device has gone quiet.
  const shown = active || playing.paused;
  $("now-playing").hidden = !shown;
  if (!shown) return;

  // Nothing to skip to when nothing is held; Stop stays, because it ends the
  // pause as well as the queue.
  $("np-skip").hidden = !active;

  $("np-state").textContent = playing.paused ? "paused" : "speaking";
  $("np-state").className =
    "shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium text-white " +
    (playing.paused ? "bg-neutral-500" : "bg-accent-600");
  $("np-from").textContent = playing.from ? `from ${playing.from}` : "";
  $("np-text").textContent =
    playing.text ?? (playing.paused ? "Speech is held on this device." : "");
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
  who.dataset.part = "who";
  who.textContent = entry.from;

  const when = document.createElement("span");
  // `neutral-400` on white is about 2.5:1, and this is 12px text — under the
  // 4.5:1 AA floor for anything that is not large. It read as decoration
  // because it was too faint to read, which is not the same as unimportant:
  // it is how you tell a message from an hour ago from one from Tuesday.
  when.className = "shrink-0 text-xs text-neutral-500 dark:text-neutral-400";
  when.dataset.part = "when";
  when.textContent = whenSaid(entry.at);

  const state = document.createElement("span");
  state.dataset.part = "state";
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
  body.dataset.part = "body";
  body.textContent = entry.text;
  body.onclick = () => body.classList.toggle("line-clamp-2");

  li.append(head, body);
  return li;
}

/**
 * Reconcile a keyed list against fresh data, keeping the nodes that survive.
 *
 * Both lists used to `replaceChildren` on every five-second poll, which threw
 * away three things nobody had finished with: an expanded message collapsed
 * mid-read, a focused Remove or Play button dropped focus to `<body>` so a
 * keyboard user lost their place entirely, and a Play button showing "…" was
 * reset while its request was still in flight. Issue #74.
 *
 * A row that is still wanted is patched, not rebuilt, because every one of
 * those three states lives on the node itself — a class, the focus ring, a
 * disabled attribute — and survives exactly as long as the node does.
 *
 * Nodes are moved only when the order genuinely changed. A move is a remove
 * and an insert as far as the DOM is concerned, which blurs a focused element,
 * so reordering everything to prepend one new row would have defeated the
 * point. Prepending touches only the new node.
 */
function syncRows(list, items, { key, build, update }) {
  const kept = new Map();
  for (const node of list.children) {
    if (node.dataset.key !== undefined) kept.set(node.dataset.key, node);
  }

  const wanted = items.map((item) => {
    const id = String(key(item));
    const existing = kept.get(id);
    if (existing) {
      kept.delete(id);
      update(existing, item);
      return existing;
    }
    const fresh = build(item);
    fresh.dataset.key = id;
    return fresh;
  });

  // Whatever is left is genuinely gone — a revoked device, a forgotten space.
  for (const node of kept.values()) node.remove();

  wanted.forEach((node, i) => {
    const at = list.children[i];
    if (at !== node) list.insertBefore(node, at ?? null);
  });
  // Placeholder rows carry no key, so they survive the loop above and have to
  // be cleared here — otherwise "Nothing yet." sits under the first message.
  while (list.children.length > wanted.length) list.lastElementChild.remove();

  return wanted.length;
}

/** The row shown when a list has nothing in it. Keyless, so `syncRows` prunes it. */
function emptyRow(text, className) {
  return Object.assign(document.createElement("li"), { className, textContent: text });
}

/**
 * Show what this device was asked to say.
 *
 * The reason this exists is the muted case: a message refused while the
 * device was silent is otherwise gone, and this is the only place it can be
 * read or played back.
 */
async function refreshHistory() {
  const unheardOnly = $("history-unheard").checked;
  let entries;
  try {
    // More than the screen shows, when filtering. The filter runs here, so
    // asking for 50 and then keeping the unheard ones hid any unheard message
    // that had fallen outside the newest 50 — from the one view whose whole
    // purpose is finding it. `HISTORY_DEPTH` is above what the node retains,
    // so this asks for everything it has; the node caps the answer at what it
    // kept, and nothing older exists to be found.
    entries = await invoke("history", { limit: unheardOnly ? HISTORY_DEPTH : 50 });
  } catch {
    return;
  }
  if (unheardOnly) entries = entries.filter((e) => e.unheard);

  const list = $("history");
  const shown = syncRows(list, entries, {
    key: (e) => e.msg_id,
    build: historyRow,
    update: updateHistoryRow,
  });
  if (shown === 0) {
    list.replaceChildren(
      emptyRow(
        unheardOnly ? "Nothing unheard." : "Nothing yet.",
        "px-4 py-3 text-sm text-neutral-500 dark:text-neutral-400",
      ),
    );
  }
}

/**
 * Bring an existing history row up to date.
 *
 * Only what can actually change: the relative time, which changes on every
 * poll, and the status, which changes when a queued message is spoken or a
 * muted one is played back. The text of a message never changes, and the Play
 * button is deliberately left alone — rewriting it is what reset a request
 * that was still in flight.
 */
function updateHistoryRow(li, entry) {
  const part = (name) => li.querySelector(`[data-part="${name}"]`);
  const when = part("when");
  const fresh = whenSaid(entry.at);
  // Guarded because assigning identical text still invalidates a selection
  // inside it, and this runs every five seconds.
  if (when && when.textContent !== fresh) when.textContent = fresh;

  const state = part("state");
  const word = STATUS_WORDS[entry.status] ?? entry.status;
  if (state && state.textContent !== word) {
    state.textContent = word;
    state.className =
      "shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium " +
      (entry.unheard
        ? "bg-amber-100 text-amber-800 dark:bg-amber-500/15 dark:text-amber-300"
        : "bg-neutral-100 text-neutral-600 dark:bg-neutral-800 dark:text-neutral-400");
  }
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
  // Offered only when there is something to undo. Comparing against the
  // field rather than the reported path so the button appears as soon as
  // someone types a different one, not a poll later.
  $("skill-default").hidden =
    $("skill-path").value.trim() === skill.default_path;
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

  // A badge on the default and nothing on the others. Changing the default
  // lives in the Manage dialog with everything else that acts on a space —
  // a second place to do it is the duplication this screen was rebuilt to
  // remove.
  if (space.is_default) head.append(defaultBadge());
  card.append(head);

  // Devices. Only the default space's members can be told apart from the rest,
  // because `list_devices` reports a space only when there is more than one.
  const mine = devicesIn(space, devices);
  // A container of its own, rather than rows appended straight onto the card:
  // a poll reconciles this list in place, and that needs a parent whose
  // children are only device rows.
  const rows = document.createElement("div");
  rows.dataset.part = "devices";
  card.append(rows);
  fillDevices(rows, mine, space.label);

  const foot = document.createElement("div");
  foot.className =
    "flex items-center gap-2 border-t border-neutral-200 px-3 py-2 " +
    "dark:border-neutral-800";

  // One button rather than five. Every action on a space needs to say which
  // space it acts on, and five buttons each repeating the name wrapped to
  // three rows on a phone — so the name moves to the top of a dialog and the
  // buttons inside it need no qualifier at all.
  const manage = cardButton("Manage");
  manage.dataset.part = "manage";
  foot.append(manage);

  // Kept on the card, not buried in the dialog: adding a device is the one
  // thing here that is done often, and the whole point of the layout is that
  // the button sits on the space it invites into.
  const add = cardButton("Add a device");
  add.dataset.part = "add";
  foot.append(add);

  card.append(foot);
  // Bound here as well as on every poll, so there is one place that decides
  // what these buttons do with the *current* facts.
  bindCardActions(card, space, several);
  return card;
}

/**
 * Point a card's buttons at the space as it is now.
 *
 * Rebound on every poll rather than captured once. A kept card's handlers
 * closed over the `space` and the `several` of the moment it was built, so a
 * card that survived a poll would open Manage with a stale `is_default` and,
 * once a second space existed, without the actions that only appear when
 * there is more than one. Keeping the node is only correct if what the node
 * does is kept current with it.
 */
function bindCardActions(card, space, several) {
  const manage = card.querySelector('[data-part="manage"]');
  if (manage) {
    manage.onclick = () =>
      withButton(manage, "…", async () => {
        showManage(space, several);
      });
  }
  const add = card.querySelector('[data-part="add"]');
  if (add) {
    add.onclick = () =>
      withButton(add, "…", async () => {
        await showInvite(space.label);
      });
  }
}

async function refreshSpaces() {
  const [spaces, devices] = await Promise.all([
    invoke("list_spaces").catch(() => null),
    invoke("list_devices").catch(() => []),
  ]);
  if (!spaces) return;
  // Kept so an unqualified invite can still name the space it joins. The
  // request stays unqualified — the node picks the default when it mints the
  // ticket — but "Joins home" is what a person needs to read before handing
  // the code to another device.
  defaultSpace = spaces.find((s) => s.is_default)?.label ?? null;
  const several = spaces.length > 1;
  syncRows($("spaces"), spaces, {
    key: (space) => space.label,
    build: (space) => spaceCard(space, devices, several),
    update: (card, space) => updateSpaceCard(card, space, devices, several),
  });
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
 * Which policy the Quiet controls are editing: `null` for this device, or a
 * space label.
 *
 * Held here rather than read off the selector each time, because the selector
 * is rebuilt whenever the spaces change and a rebuild would otherwise silently
 * move the controls to a different policy under the reader's hands.
 */
/**
 * What the time fields show for a policy with no window of its own.
 *
 * A neutral starting point, deliberately not carried over from whichever
 * scope was looked at last.
 */
const DEFAULT_QUIET = { from: "22:00", to: "07:00" };

let policyScope = null;

/** Whether right now falls inside a policy's quiet window. */
function inQuietHours(policy) {
  const now = new Date();
  const minute = now.getHours() * 60 + now.getMinutes();
  const from = minutesOf(policy.from);
  const to = minutesOf(policy.to);
  return from != null && to != null && insideWindow(from, to, minute);
}

/**
 * Show whether this device will speak, and why not when it will not.
 *
 * The banner is the point of the section. Both controls can be set to
 * something reasonable and the device still be silent right now, and a person
 * wondering why nothing is coming out deserves to be told rather than left to
 * work it out from a clock and two time fields.
 *
 * With more than one space there are several answers to "will it speak", so
 * the controls edit one policy at a time and say which. A mute switch that
 * silently applies to something other than what is named beside it is the
 * failure this whole section exists to avoid.
 */
async function refreshPolicy() {
  let policy;
  let spaces;
  try {
    [policy, spaces] = await Promise.all([
      invoke("policy"),
      invoke("list_spaces").catch(() => []),
    ]);
  } catch (e) {
    // Returning quietly here left the last drawn state on screen, which for
    // a first read is "not muted, no quiet hours" — a claim about settings
    // nobody could read. The node now says why it could not answer, so say
    // it (#73).
    say(`Could not read this device's policy: ${e}`, "error");
    return;
  }

  // A scope naming a space this device has since left falls back to the
  // device, rather than editing a policy nothing can reach.
  const labels = spaces.map((s) => s.label);
  if (policyScope != null && !labels.includes(policyScope)) policyScope = null;

  $("scope-row").hidden = labels.length < 2;
  const picker = $("policy-scope");
  // Left alone while open, or the list would close under the user's finger.
  if (document.activeElement !== picker) {
    picker.replaceChildren(
      Object.assign(document.createElement("option"), {
        value: "",
        textContent: "This device",
        selected: policyScope == null,
      }),
      ...labels.map((label) =>
        Object.assign(document.createElement("option"), {
          value: label,
          textContent: label,
          selected: label === policyScope,
        }),
      ),
    );
  }

  // What the controls show: the device's own policy, or the selected space's
  // override. A space with no override reads as "nothing extra", which is
  // exactly what it is.
  const forSpace = policyScope != null;
  const shown = forSpace
    ? (policy.spaces.find((s) => s.label === policyScope) ?? {
        muted: false,
        from: null,
        to: null,
        high_breaks_through: false,
      })
    : policy;

  $("mute-label").textContent = forSpace ? `Mute ${policyScope}` : "Mute";
  $("mute-note").textContent = forSpace
    ? `Nothing sent in ${policyScope} is spoken here. Other spaces are unaffected.`
    : "Silent until you turn this off. Nothing breaks through.";
  $("quiet-note").textContent = forSpace
    ? `Silent every day between these times, for ${policyScope} only.`
    : "Silent every day between these times.";

  // Said outright rather than left to be inferred from a checkbox that turns
  // out to change nothing: a space adds silence and never removes it.
  const note = $("scope-note");
  if (!forSpace) {
    note.hidden = true;
  } else if (policy.muted) {
    note.textContent =
      `This device is muted, so ${policyScope} is silent whatever you set here. ` +
      "Unmute the device to hear any of it.";
    note.hidden = false;
  } else {
    note.textContent =
      "Adds to this device's settings. A space can be quieter than the device, " +
      "never louder.";
    note.hidden = false;
  }

  if (document.activeElement !== $("mute")) $("mute").checked = shown.muted;

  const hasWindow = shown.from != null && shown.to != null;
  if (document.activeElement !== $("quiet-on")) $("quiet-on").checked = hasWindow;
  $("quiet-controls").hidden = !$("quiet-on").checked;
  // Reset when there is no window, rather than leaving whatever was there.
  // With one policy the fields could only ever hold their own values; with a
  // scope selector they hold the *previous* scope's, and switching from a
  // space with a window to one without left the old times in place — so
  // ticking the box would have saved a window the reader never chose for it.
  if (document.activeElement !== $("quiet-from")) {
    $("quiet-from").value = hasWindow ? shown.from : DEFAULT_QUIET.from;
  }
  if (document.activeElement !== $("quiet-to")) {
    $("quiet-to").value = hasWindow ? shown.to : DEFAULT_QUIET.to;
  }
  if (document.activeElement !== $("quiet-high")) {
    $("quiet-high").checked = shown.high_breaks_through;
  }

  // The banner answers "will this device speak", which is a question about
  // the device however the controls happen to be scoped. Spaces silenced on
  // their own are named, because the only other evidence is messages that
  // never arrive.
  const banner = $("silent-banner");
  const hushed = policy.spaces
    .filter((s) => s.muted || inQuietHours(s))
    .map((s) => s.label);
  if (policy.muted) {
    banner.textContent = "This device is muted. Nothing will be spoken.";
    banner.hidden = false;
  } else if (inQuietHours(policy) && policy.high_breaks_through) {
    banner.textContent = "Quiet hours — only urgent messages will be spoken.";
    banner.hidden = false;
  } else if (inQuietHours(policy)) {
    banner.textContent = `Quiet hours until ${policy.to}. Nothing will be spoken.`;
    banner.hidden = false;
  } else if (hushed.length) {
    banner.textContent = `Silent in ${hushed.join(", ")}. Everything else will be spoken.`;
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
    space: policyScope,
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

$("policy-scope").onchange = async () => {
  // An empty value is the device. Stored as null so every reader asks the
  // same question — "is a space selected" — instead of comparing with "".
  policyScope = $("policy-scope").value || null;
  await refreshPolicy();
};

$("mute").onchange = async () => {
  const muted = $("mute").checked;
  await call("set_mute", { muted, space: policyScope });
  const what = policyScope ?? "this device";
  say(muted ? `${what} muted` : `${what} unmuted`);
  await refreshPolicy();
};

$("quiet-on").onchange = async () => {
  $("quiet-controls").hidden = !$("quiet-on").checked;
  await saveQuiet();
  const forWhat = policyScope ? ` for ${policyScope}` : "";
  say($("quiet-on").checked ? `quiet hours set${forWhat}` : `quiet hours off${forWhat}`);
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

/** Undoes what opening the manage dialog bound. */
let closeManage = () => {};

/** A full-width button for the manage dialog's stacked list. */
function manageButton(label, note, { danger = false } = {}) {
  const b = document.createElement("button");
  b.type = "button";
  b.className =
    "w-full rounded-lg border px-3 py-2 text-left text-sm transition " +
    "focus-visible:outline-2 focus-visible:outline-offset-2 " +
    (danger
      ? "border-red-200 text-red-600 hover:bg-red-50 " +
        "dark:border-red-500/30 dark:text-red-400 dark:hover:bg-red-500/10"
      : "border-neutral-300 hover:bg-neutral-100 " +
        "dark:border-neutral-600 dark:hover:bg-neutral-700");
  b.append(
    Object.assign(document.createElement("span"), {
      className: "font-medium",
      textContent: label,
    }),
  );
  // The consequence, next to the button that causes it. These actions differ
  // from each other in what they send and what the other devices are left
  // believing, which a one-word label cannot carry.
  if (note) {
    b.append(
      Object.assign(document.createElement("span"), {
        className:
          "mt-0.5 block text-xs font-normal text-neutral-500 dark:text-neutral-400",
        textContent: note,
      }),
    );
  }
  return b;
}

/**
 * Everything that acts on one space, in a dialog titled with that space.
 *
 * The name at the top is the whole point. These actions used to sit in the
 * card footer, where each had to spell out its target — "Leave work",
 * "Replace work" — to be unambiguous, and five of them wrapped into three
 * rows on a phone. Named once here, the buttons say what they do and nothing
 * about the name is left to be inferred.
 */
function showManage(space, several) {
  const box = $("manage-modal");
  $("manage-title").textContent = space.label;
  $("manage-sub").textContent = space.is_default
    ? "Bare device names resolve here."
    : `Reach devices in it as ${space.label}/<device>.`;

  // Each action closes the dialog before doing anything: `ask` is another
  // dialog at the same layer, and a confirmation stacked on the panel that
  // raised it is not a shape this markup supports.
  const run = (fn) => async () => {
    closeManage();
    await fn();
  };

  const actions = [];

  if (!space.is_default) {
    const def = manageButton(
      "Make default",
      "Device names with no space in front of them resolve here.",
    );
    def.onclick = run(async () => {
      await call("default_space", { label: space.label });
      say(`bare names now mean ${space.label}`);
      await refresh();
    });
    actions.push(def);
  }

  const add = manageButton(
    "Add a device",
    "Shows a code that joins this space, and no other.",
  );
  add.onclick = run(() => showInvite(space.label));
  actions.push(add);

  const rename = manageButton("Rename", "Only on this device. No one is told.");
  rename.onclick = run(async () => {
    const to = await ask(`New name for "${space.label}"`, { input: space.label });
    if (!to || to === space.label) return;
    await call("rename_space", { label: space.label, to });
    say(`renamed to ${to}`);
    await refresh();
  });
  actions.push(rename);

  const danger = [];

  const leave = manageButton(
    "Leave",
    "The other devices are told, and stop reaching this one.",
    { danger: true },
  );
  leave.onclick = run(async () => {
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
  danger.push(leave);

  const rotate = manageButton(
    "Replace",
    "Everyone else is locked out at once and has to be invited again.",
    { danger: true },
  );
  rotate.onclick = run(async () => {
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
  danger.push(rotate);

  // Only where the difference matters. Forgetting is the escape hatch for a
  // space whose other devices cannot be reached to be told — it removes the
  // space here and sends nothing, so they go on counting this device a
  // member. Leaving is what you want otherwise.
  if (several) {
    const forget = manageButton(
      "Forget on this device",
      "Sends nothing. The others still count this device a member.",
      { danger: true },
    );
    forget.onclick = run(async () => {
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
    danger.push(forget);
  }

  $("manage-actions").replaceChildren(...actions);
  $("manage-danger").replaceChildren(...danger);

  closeManage =
    openModal(box, {
      focus: actions[0],
      onClose: () => {
        $("manage-actions").replaceChildren();
        $("manage-danger").replaceChildren();
        $("manage-close").onclick = null;
        closeManage = () => {};
      },
    }) ?? (() => {});
  $("manage-close").onclick = closeManage;
}

/** Undoes what opening the invite dialog bound, so it can be opened again. */
let closeInvite = () => {};

/** This device's default space, for naming an invite that did not name one. */
let defaultSpace = null;

/**
 * Show an invite for one space, in a dialog.
 *
 * The space travels on the ticket, so what the button said when it was
 * pressed is what the joining device gets — even if the default changes
 * before the code is scanned. It is named at the top of the dialog for the
 * same reason: an invite that joined the wrong space cannot be taken back
 * once somebody has scanned it.
 *
 * The ticket is fetched before the dialog opens, so it never appears holding
 * the previous space's code. A failure leaves it shut, with `call` having
 * already said why.
 */
async function showInvite(space) {
  const { url, qr, expires_in } = await call("make_invite", { space });
  // Trusted: the SVG is produced by our own Rust side, not user input.
  $("qr").innerHTML = qr;
  $("qr").hidden = !qr;
  $("ticket").textContent = url;
  // The space is the fact this dialog exists to make unmissable, so it is
  // said in the heading rather than left to be inferred from the code.
  const joins = space ?? defaultSpace;
  $("invite-for").replaceChildren(
    document.createTextNode("Joins "),
    Object.assign(document.createElement("span"), {
      className: "font-semibold text-neutral-900 dark:text-neutral-100",
      textContent: joins ?? "the default space",
    }),
  );
  $("expiry").textContent = `Expires in ${Math.max(1, Math.round(expires_in / 60))} min. Single use.`;

  const box = $("invite-modal");
  // Focus lands on the way out rather than on the code: there is nothing here
  // to fill in, and a keyboard user's next move is to leave.
  closeInvite =
    openModal(box, {
      focus: $("invite-done"),
      onClose: () => {
        // The code is spent once it is used, and leaving it in the DOM means
        // the next open can flash the old one before the new one arrives.
        $("qr").replaceChildren();
        $("ticket").textContent = "";
        $("invite-close").onclick = null;
        $("invite-done").onclick = null;
        closeInvite = () => {};
      },
    }) ?? (() => {});
  $("invite-close").onclick = closeInvite;
  $("invite-done").onclick = closeInvite;
  say("");
}

$("copy").onclick = async () => {
  try {
    await navigator.clipboard.writeText($("ticket").textContent);
    say("copied");
  } catch {
    say("could not copy — select the code instead", "error");
  }
};

/** Undoes what opening the join dialog bound. */
let closeJoin = () => {};

/**
 * Join a space, in two steps: read the invite, then act on it.
 *
 * The destination is written into the ticket by whoever minted it, so this
 * device cannot choose it — which is why joining is not offered on a space
 * card, and why it is worth a step of its own to say what the code will do.
 * `preview_invite` is local: no device is contacted and the single-use token
 * is not spent, so reading a code costs nothing and can be undone by going
 * back.
 *
 * `prefill` is for a scanned code, which arrives already known. It still
 * stops at the confirmation — a scan is easy to do by accident, and a space
 * joined by mistake has to be left again on both devices.
 */
function showJoin(prefill = "") {
  const box = $("join-modal");
  const code = $("join-step-code");
  const confirm = $("join-step-confirm");

  const step = (which) => {
    code.hidden = which !== "code";
    confirm.hidden = which !== "confirm";
  };

  $("join-input").value = prefill;
  step("code");

  // Held between the steps: the confirmation names a space, and joining has
  // to use the same code that was read, not whatever the field says by then.
  let read = "";

  const onRead = () =>
    withButton($("join-read"), "…", async () => {
      const ticket = $("join-input").value.trim();
      if (!ticket) {
        say("paste an invite first", "error");
        return;
      }
      // Errors here are the ones the join would have raised — expired,
      // truncated, not an invite — surfaced before anything is committed to.
      const preview = await call("preview_invite", { ticket });
      read = ticket;
      $("join-space").textContent = preview.label ?? "their default space";
      const mins = Math.max(1, Math.round(preview.expires_in / 60));
      $("join-meta").textContent =
        `From device ${preview.from} · expires in ${mins} min · single use`;
      // Prefilled with the inviter's name, because agreeing is the common
      // case and a name that matches across devices is one less thing to
      // reconcile. A ticket that carried no name gets an empty field rather
      // than an invented one.
      $("join-name").value = preview.label ?? "";
      step("confirm");
      $("join-go").focus();
    });

  const onGo = () =>
    withButton($("join-go"), "…", async () => {
      const label = $("join-name").value.trim();
      const joined = await call("join_space", {
        ticket: read,
        label: label || null,
      });
      closeJoin();
      say(`joined '${joined.space}' — ${joined.members} devices`);
      await refresh();
    });

  closeJoin =
    openModal(box, {
      focus: $("join-input"),
      onClose: () => {
        // A spent code left in the field is one that fails confusingly on
        // the next open.
        $("join-input").value = "";
        $("join-name").value = "";
        read = "";
        for (const id of ["join-close", "join-cancel", "join-read", "join-back", "join-go"]) {
          $(id).onclick = null;
        }
        closeJoin = () => {};
      },
    }) ?? (() => {});
  $("join-close").onclick = closeJoin;
  $("join-cancel").onclick = closeJoin;
  $("join-read").onclick = onRead;
  $("join-back").onclick = () => {
    step("code");
    $("join-input").focus();
  };
  $("join-go").onclick = onGo;
}

$("join-open").onclick = () => showJoin();

/**
 * Which theme this device shows: "system", "light" or "dark".
 *
 * Kept in `localStorage` rather than in the node, because it is a property of
 * this screen and not of this device's identity — nothing about it belongs on
 * the wire, and a node-side setting would have to cross five platforms to say
 * what one webview should paint. The trade is that clearing site data forgets
 * it, which falls back to following the system.
 *
 * `<html data-theme>` is set by an inline script in the document head as well
 * as here. That is not duplication: this runs after first paint, and doing it
 * only here flashes the wrong theme on every launch.
 */
function applyTheme(choice) {
  document.documentElement.dataset.theme = choice;
  for (const b of document.querySelectorAll("[data-theme-choice]")) {
    const on = b.dataset.themeChoice === choice;
    b.className =
      "flex-1 rounded-md px-3 py-1.5 text-sm font-medium transition " +
      (on
        ? "bg-white text-neutral-900 shadow-sm dark:bg-neutral-950 dark:text-neutral-100"
        : "text-neutral-500 hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-100");
    b.setAttribute("aria-pressed", String(on));
  }
}

{
  // Reads back what the head script already applied, so the buttons agree
  // with the page rather than with a default.
  let saved = "system";
  try {
    saved = localStorage.getItem("voicecast-theme") ?? "system";
  } catch {
    // Storage can be unavailable outright — a webview with site data off, or
    // a private context. Following the system is the right answer then, and
    // it is what the markup already says.
  }
  applyTheme(["light", "dark"].includes(saved) ? saved : "system");

  for (const b of document.querySelectorAll("[data-theme-choice]")) {
    b.onclick = () => {
      const choice = b.dataset.themeChoice;
      applyTheme(choice);
      try {
        // "system" is stored rather than removed, so a later default change
        // cannot silently move a device that had chosen to follow the OS.
        localStorage.setItem("voicecast-theme", choice);
      } catch {
        say("this device will not remember that until it is restarted", "error");
      }
    };
  }
}

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

$("skill-default").onclick = () =>
  withButton($("skill-default"), "…", async () => {
    const previous = await call("reset_skill_path");
    await refreshSkill();
    // The old copy is deliberately left where it was — it is the user's file
    // and this button did not offer to delete it. Saying where it is beats
    // leaving a skill somewhere that will never be updated again.
    say(
      previous
        ? `back to the default. The copy at ${previous} is still there and will not be kept in step.`
        : "back to the default",
    );
  });

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
  // Shown rather than acted on. A scan is easy to do by accident, and the
  // space it joins is decided by whoever made the code — the same reason a
  // pasted invite gets a confirmation, and more so here, where nothing was
  // typed at all.
  showScreen("spaces");
  showJoin(ticket);
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
