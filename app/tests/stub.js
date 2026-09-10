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

/**
 * Every command the interface has asked for, in order.
 *
 * A probe cannot otherwise tell "the page did not join" from "the page joined
 * with something else" — both leave the same DOM. The arguments are kept too,
 * because the join flow's whole contract is *which* ticket was sent.
 */
window.__calls = [];

/**
 * A device name for the roster, so a probe can supply a hostile one.
 *
 * Names come from other people's devices and are rendered on this one. A
 * default that looks nothing like an attack is deliberate: the probe sets
 * this, so what is being tested is visible in the probe rather than hidden in
 * the stub.
 */
window.__peerName = "Phone";

/**
 * What a send reports back, per device.
 *
 * Overridable because "some devices spoke and some did not" is the outcome
 * the Speak tab exists to draw, and it is not reachable from a stub that
 * always succeeds. `window.__speakFails` is the other half: nothing being
 * spoken anywhere comes back as an error from the command rather than as an
 * empty list, and the tab has to say so somewhere that does not fade.
 */
window.__spoken = null;
window.__speakFails = null;

/**
 * What the node reports as its version.
 *
 * A probe sets this to the empty string to check the one case that actually
 * bit: a node too old to report one, where the screen must not print a blank
 * or the word "undefined" where a number belongs.
 */
window.__version = "0.9.2";

window.__TAURI__ = {
  core: {
    invoke: async (cmd, args) => {
      window.__calls.push({ cmd, args: args ?? null });
      // Lets a probe make one command fail. The error path in `say()` is
      // otherwise only reachable with a broken node, and it is the path that
      // had no way out of it (#144). Cleared as it fires, so a probe arms a
      // single failure rather than breaking everything after it.
      if (window.__forceError) {
        const message = window.__forceError;
        window.__forceError = null;
        throw message;
      }
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
            version: window.__version,
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
          return [
            device("Mac", "aaaa1111", 5, true),
            device(window.__peerName, "bbbb2222", 170),
          ];
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
        // The two halves of joining. `preview_invite` is what Continue calls
        // and it commits to nothing; `join_space` is the one that does. A
        // probe checks that the second does not happen early, and that when
        // it does happen it carries the ticket that was previewed.
        case "preview_invite":
          return {
            from: "aaaa1111bbbb2222",
            label: "home",
            expires_in: 240,
          };
        case "join_space":
          return { space: "home", members: 2 };
        // A skill installed somewhere other than the default, which is the
        // only state in which the reset is offered at all.
        case "skill_status":
          return {
            state: "current",
            path: "/Users/someone/Desktop/skills/clispeak/SKILL.md",
            default_path: "/Users/someone/.claude/skills/clispeak/SKILL.md",
            sandboxed: false,
          };
        case "speak":
          if (window.__speakFails) throw window.__speakFails;
          return window.__spoken ?? [{ device: "Mac", heard: true, outcome: "spoken" }];
        case "reset_skill_path":
          reset = true;
          return "/Users/someone/Desktop/skills/clispeak/SKILL.md";
        default:
          return null;
      }
    },
  },
};
