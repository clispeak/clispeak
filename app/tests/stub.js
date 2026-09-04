/**
 * Stands in for the Rust side, so the interface can be driven without a node.
 *
 * Every command the app calls has to answer with the shape the real one
 * returns — the first version of this returned `null` for two of them, which
 * threw inside `refresh()` and stopped the list ever being built. A stub that
 * is wrong in that direction fails loudly, which is the right way round.
 *
 * `advance()` moves the clock the answers are computed from, so a probe can
 * make presence cross the "active" boundary and see whether a poll notices.
 */
window.__errors = [];
window.addEventListener("error", (e) => window.__errors.push(String(e.message || e.error)));
window.addEventListener("unhandledrejection", (e) =>
  window.__errors.push("rejected: " + String((e.reason && (e.reason.stack || e.reason.message)) || e.reason)),
);
// Reported whether or not a probe finishes, since a probe that dies silently
// looks the same as one that found nothing.
setTimeout(() => {
  const pre = document.createElement("pre");
  pre.id = "errors";
  pre.textContent = window.__errors.join("\n---\n") || "(no errors)";
  document.body.appendChild(pre);
}, 2000);

let offset = 0;
window.__advance = (secs) => {
  offset += secs;
};

/**
 * Playback state, so a probe can pause and resume and see what the interface
 * does about it.
 *
 * Mirrors the queue after #109: a paused message is still reported as what
 * this device is playing, because it is what Resume will continue. `held`
 * false with `paused` true is the other real case — a pause arriving from the
 * CLI with nothing being spoken.
 */
window.__playback = { held: true, paused: false };

let reset = false;
const now = () => Math.floor(Date.now() / 1000);
const device = (name, id, seen, self) => ({
  name,
  endpoint_id: id,
  is_self: !!self,
  last_seen_secs: seen == null ? null : seen + offset,
  space: null,
});

window.__TAURI__ = {
  core: {
    invoke: async (cmd) => {
      // Once reset, `skill_status` reports the default, the way the real
      // command does once the record is gone.
      if (cmd === "skill_status" && reset) {
        return {
          state: "absent",
          path: "/Users/someone/.claude/skills/clispeak/SKILL.md",
          default_path: "/Users/someone/.claude/skills/clispeak/SKILL.md",
          sandboxed: false,
        };
      }
      switch (cmd) {
        case "node_status":
          return {
            name: "Mac",
            device_id: "abcdef0123456789abcdef",
            engine: "Lessac",
            fallback: false,
            reason: null,
            starting: false,
            failed: null,
          };
        case "history":
          return [
            {
              msg_id: "m1",
              // Long enough to be clamped, which is what makes the expand
              // toggle meaningful.
              text: "A deliberately long message ".repeat(20),
              from: "Phone",
              at: now() - 60,
              status: "spoken",
              priority: "normal",
              unheard: false,
            },
            {
              msg_id: "m2",
              text: "Second message",
              from: "Laptop",
              at: now() - 600,
              status: "queued",
              priority: "normal",
              unheard: true,
            },
          ];
        case "list_spaces":
          return [{ label: "home", devices: 2, is_default: true, founded_here: true }];
        case "list_devices":
          // One self row, which never changes, and one peer that starts just
          // inside the three-minute "active" window.
          return [device("Mac", "aaaa1111", 5, true), device("Phone", "bbbb2222", 170)];
        case "now_playing": {
          const p = window.__playback;
          const active = p.held;
          return {
            msg_id: active ? "m1" : null,
            paused: p.paused,
            from: active ? "Phone" : null,
            text: active ? "A message being spoken" : null,
            waiting: 0,
          };
        }
        case "pause_speech":
          window.__playback.paused = true;
          return null;
        case "resume_speech":
          window.__playback.paused = false;
          return null;
        case "stop_speech":
        case "skip_speech":
          window.__playback.held = false;
          window.__playback.paused = false;
          return null;
        case "voice_config":
          return { available: [], current: null, rate: 1.0 };
        case "policy":
          return {
            muted: false,
            quiet_from: null,
            quiet_to: null,
            high_breaks_through: true,
            spaces: [],
          };
        case "battery_ok":
          return true;
        // A skill installed somewhere other than the default, which is the
        // only state in which the reset is offered at all.
        case "skill_status":
          return {
            state: "current",
            path: "/Users/someone/Desktop/skills/clispeak/SKILL.md",
            default_path: "/Users/someone/.claude/skills/clispeak/SKILL.md",
            sandboxed: false,
          };
        case "reset_skill_path":
          reset = true;
          return "/Users/someone/Desktop/skills/clispeak/SKILL.md";
        default:
          return null;
      }
    },
  },
};
