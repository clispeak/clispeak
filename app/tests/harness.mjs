/**
 * Drives the interface in a headless browser and reports what it finds.
 *
 * There is no bundler and no framework here, which is deliberate — but it
 * also meant no way to check the behaviours that only exist once the page is
 * running. Both of the defects this was written for were invisible to review
 * and obvious in a browser: a list rebuilt on a five-second poll threw away an
 * expanded message and a focused button (#74), and four dialogs claimed
 * `aria-modal` while letting Tab walk out behind the backdrop (#75).
 *
 * Manual, not CI. It needs a real Chrome, which the build images do not carry,
 * and pulling one in to run two probes is a poor trade — so this is a command
 * someone runs when touching the interface, and the PR says whether it was
 * run. Saying that plainly is better than a check that quietly never runs.
 *
 *   node app/tests/harness.mjs           # every probe
 *   node app/tests/harness.mjs lists     # one of them
 *
 * How it works: the sources are copied to a scratch directory, a stub for
 * `window.__TAURI__` is injected ahead of `main.js`, the probe is injected
 * after it, and the page is loaded with a virtual time budget so the app's
 * own five-second poll fires in well under a second of real time. The probe
 * writes its findings into a `<pre>` that this script reads back out of the
 * dumped DOM.
 */

import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { cp, mkdtemp, readFile, writeFile, readdir } from "node:fs/promises";
import { existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { extname, join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const src = join(here, "..", "src");

/** Where a real Chrome might be. Skipped, loudly, if none is found. */
const CHROMES = [
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/Applications/Chromium.app/Contents/MacOS/Chromium",
  "/usr/bin/google-chrome",
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
  "/var/lib/flatpak/exports/bin/org.chromium.Chromium",
];

const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css" };

async function main() {
  const only = process.argv[2];
  const chrome = CHROMES.find((p) => existsSync(p));
  if (!chrome) {
    console.error("no Chrome or Chromium found — install one, or add its path to CHROMES");
    console.error("looked in:\n  " + CHROMES.join("\n  "));
    process.exit(2);
  }

  const probes = (await readdir(join(here, "probes")))
    .filter((f) => f.endsWith(".js"))
    .filter((f) => !only || f.startsWith(only));
  if (probes.length === 0) {
    console.error(`no probe matches "${only}"`);
    process.exit(2);
  }

  const dir = await mkdtemp(join(tmpdir(), "voicecast-ui-"));
  for (const file of ["index.html", "main.js", "modal.js"]) {
    await cp(join(src, file), join(dir, file));
  }
  // Generated and not committed, so a fresh checkout has none until Tailwind
  // has run. The probes assert behaviour rather than appearance, so its
  // absence is worth a line rather than a failure — but it is worth the line,
  // because a probe that depended on layout would fail confusingly without it.
  if (existsSync(join(src, "styles.css"))) {
    await cp(join(src, "styles.css"), join(dir, "styles.css"));
  } else {
    console.log("note: src/styles.css has not been built — running unstyled");
  }
  await cp(join(here, "stub.js"), join(dir, "stub.js"));
  for (const probe of probes) await cp(join(here, "probes", probe), join(dir, probe));

  const page = await readFile(join(dir, "index.html"), "utf8");
  const anchor = '<script type="module" src="main.js"></script>';
  if (!page.includes(anchor)) throw new Error("index.html no longer loads main.js as expected");

  const server = createServer(async (req, res) => {
    const name = decodeURIComponent(req.url.split("?")[0]).replace(/^\//, "") || "index.html";
    try {
      const body = await readFile(join(dir, name));
      res.writeHead(200, { "content-type": TYPES[extname(name)] ?? "application/octet-stream" });
      res.end(body);
    } catch {
      res.writeHead(404).end();
    }
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const base = `http://127.0.0.1:${server.address().port}`;

  let failed = 0;
  for (const probe of probes) {
    await writeFile(
      join(dir, "index.html"),
      page.replace(anchor, `<script src="stub.js"></script>\n${anchor}\n<script src="${probe}"></script>`),
    );
    const dom = await run(chrome, `${base}/index.html`);
    const found = /<pre id="report">([\s\S]*?)<\/pre>/.exec(dom);
    const name = probe.replace(/\.js$/, "");
    if (!found) {
      const errors = /<pre id="errors">([\s\S]*?)<\/pre>/.exec(dom);
      console.log(`\n${name}: the probe produced nothing`);
      if (errors) console.log(unescapeHtml(errors[1]));
      failed++;
      continue;
    }
    const report = unescapeHtml(found[1]);
    console.log(`\n${name}\n${report}`);
    if (report.includes("FAIL")) failed++;
  }

  server.close();
  console.log(failed ? `\n${failed} probe(s) reported a failure` : "\nall probes passed");
  process.exit(failed ? 1 : 0);
}

/** Load the page and hand back the DOM once the probe has finished. */
function run(chrome, url) {
  return new Promise((resolve, reject) => {
    const child = spawn(
      chrome,
      [
        "--headless",
        "--disable-gpu",
        "--no-sandbox",
        // Well past the app's own REFRESH_MS, so the poll that used to wipe
        // the list actually happens. Virtual time, so this costs no waiting.
        "--virtual-time-budget=16000",
        "--dump-dom",
        url,
      ],
      { stdio: ["ignore", "pipe", "ignore"] },
    );
    let out = "";
    child.stdout.on("data", (d) => (out += d));
    child.on("error", reject);
    child.on("close", () => resolve(out));
  });
}

function unescapeHtml(s) {
  return s
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&quot;", '"')
    .replaceAll("&#39;", "'")
    .replaceAll("&amp;", "&");
}

await main();
