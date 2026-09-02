// Thin UI over the node running in this app's Rust side. Deliberately small:
// the interesting behaviour lives in voicecast-core, shared with the CLI.
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);

/** Call a Tauri command, showing failures rather than swallowing them. */
async function call(cmd, args) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    $("result").textContent = String(e);
    $("result").className = "sub warn";
    throw e;
  }
}

async function refresh() {
  const status = await call("node_status");
  $("name").textContent = status.name;
  $("ident").innerHTML =
    `<code>${status.device_id.slice(0, 16)}…</code> · ${status.engine}` +
    (status.fallback ? ' · <span class="warn">fallback voice</span>' : "");

  const devices = await call("list_devices");
  $("devices").innerHTML = devices.length
    ? devices
        .map(
          (d) =>
            `<div class="row"><span>${d.name}${d.is_self ? " (this device)" : ""}</span>` +
            `<span class="id">${d.endpoint_id.slice(0, 12)}…</span></div>`,
        )
        .join("")
    : "<div class='sub'>none yet</div>";
}

$("invite").onclick = async () => {
  const { url, expires_in } = await call("make_invite");
  $("ticket").textContent = url;
  $("result").className = "sub";
  $("result").textContent = `Expires in ${Math.floor(expires_in / 60)}m. Single use.`;
};

$("join").onclick = async () => {
  const members = await call("join_space", { ticket: $("join-input").value.trim() });
  $("result").className = "sub";
  $("result").textContent = `Joined. ${members} devices.`;
  await refresh();
};

$("say").onclick = async () => {
  await call("speak", { text: $("say-input").value });
  $("result").className = "sub";
  $("result").textContent = "spoken";
};

refresh();
setInterval(refresh, 5000);
