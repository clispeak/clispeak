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

/** Clear a message that has been superseded by things working again. */
function clearResult() {
  $("result").className = "sub";
  $("result").textContent = "";
}

async function refresh() {
  // The node starts asynchronously, so early polls can fail while it comes
  // up. Report that as a transient state rather than an error, and clear it
  // once it succeeds — otherwise a startup blip stays on screen forever.
  let status;
  try {
    status = await invoke("node_status");
  } catch (e) {
    $("ident").textContent = "starting…";
    return;
  }
  clearResult();
  $("name").textContent = status.name;
  $("name-input").placeholder = status.name;
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

$("rename").onclick = async () => {
  const name = $("name-input").value.trim();
  if (!name) {
    $("result").className = "sub warn";
    $("result").textContent = "type a name first";
    return;
  }
  await call("rename_device", { name });
  $("result").className = "sub";
  $("result").textContent = `renamed to ${name}`;
  $("name-input").value = "";
  await refresh();
};

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
