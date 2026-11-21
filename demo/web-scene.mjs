// Records the browser-viewer demo scene: serves web/site locally, drives
// it in headless Chrome (open an archive, type its password, show the
// files), and frames each step in a browser-style window that matches the
// terminal scenes. Writes PNG frames plus an ffmpeg concat list.
//
//   node demo/web-scene.mjs <chrome> <site-dir> <archive> <password> <out-dir>
//
// Used by demo/render.sh for scene files ending in `.web`.
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";

const [chrome, siteArg, archive, password, outDir] = process.argv.slice(2);
const siteDir = path.resolve(siteArg);
const FRAME = { width: 960, height: 520 }; // matches demo/common.tape
const MARGIN = 22;
const BAR = 38;
const VIEW = { width: FRAME.width - 2 * MARGIN, height: FRAME.height - 2 * MARGIN - BAR };
const URL_LABEL = "marcdomain.github.io/vaqum";

fs.mkdirSync(outDir, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// --- serve the viewer locally -------------------------------------------
const types = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm" };
const server = http.createServer((req, res) => {
  const file = path.join(siteDir, decodeURIComponent(new URL(req.url, "http://x").pathname).replace(/\/$/, "/index.html"));
  if (!file.startsWith(siteDir + path.sep) || !fs.existsSync(file)) return res.writeHead(404).end();
  res.writeHead(200, { "content-type": types[path.extname(file)] ?? "application/octet-stream" });
  fs.createReadStream(file).pipe(res);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const siteUrl = `http://127.0.0.1:${server.address().port}/`;

// --- drive Chrome over the DevTools protocol ----------------------------
const port = 9400 + Math.floor(Math.random() * 400);
const proc = spawn(chrome, ["--headless=new", `--remote-debugging-port=${port}`, "--hide-scrollbars",
  `--user-data-dir=${outDir}/profile`, "about:blank"], { stdio: "ignore" });
let targets;
for (let i = 0; i < 60 && !targets; i++) {
  try { targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); } catch { await sleep(200); }
}
const ws = new WebSocket(targets.find((t) => t.type === "page").webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let nextId = 0;
const pending = new Map();
ws.addEventListener("message", (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
});
const send = (method, params = {}) => new Promise((r) => { const id = ++nextId; pending.set(id, r); ws.send(JSON.stringify({ id, method, params })); });
const evaluate = async (expression) => (await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true })).result.result?.value;
const viewport = (size) => send("Emulation.setDeviceMetricsOverride", { ...size, deviceScaleFactor: 1, mobile: false });
const shot = async () => (await send("Page.captureScreenshot", { format: "png" })).result.data;

await send("Page.enable");
await send("DOM.enable");
await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "light" }] });
await viewport(VIEW);
await send("Page.navigate", { url: siteUrl });
await sleep(1200);

const raw = []; // [base64 png, seconds]
raw.push([await shot(), 1.6]);

const doc = await send("DOM.getDocument");
const input = await send("DOM.querySelector", { nodeId: doc.result.root.nodeId, selector: "#file-input" });
await send("DOM.setFileInputFiles", { nodeId: input.result.nodeId, files: [path.resolve(archive)] });
await sleep(900);
raw.push([await shot(), 1.0]);

for (let i = 1; i <= password.length; i++) {
  await evaluate(`document.getElementById("password").value = ${JSON.stringify(password.slice(0, i))}; 1`);
  raw.push([await shot(), 0.07]);
}
raw.push([await shot(), 0.5]);
await evaluate(`document.getElementById("password-form").requestSubmit(); 1`);
await sleep(2600);
await evaluate(`document.getElementById("result").scrollIntoView({ block: "start" }); window.scrollBy(0, -16); 1`);
await sleep(300);
raw.push([await shot(), 4.5]);

// --- frame each screenshot in a browser-style window --------------------
await viewport(FRAME);
const frames = [];
for (const [i, [png, seconds]] of raw.entries()) {
  const html = `<!doctype html><html><head><meta charset="utf-8"></head><body style="margin:0;background:#EEF0FB;width:${FRAME.width}px;height:${FRAME.height}px">
  <div style="position:absolute;left:${MARGIN}px;top:${MARGIN}px;width:${VIEW.width}px;height:${VIEW.height + BAR}px;border-radius:10px;overflow:hidden;background:#fff;box-shadow:0 1px 0 #dfe2f3">
    <div style="height:${BAR}px;background:#e9ebf5;display:flex;align-items:center;gap:8px;padding:0 14px;box-sizing:border-box;border-bottom:1px solid #dfe2f3">
      <span style="width:12px;height:12px;border-radius:50%;background:#ff5f57"></span>
      <span style="width:12px;height:12px;border-radius:50%;background:#febc2e"></span>
      <span style="width:12px;height:12px;border-radius:50%;background:#28c840"></span>
      <span style="margin-left:18px;flex:1;max-width:420px;height:24px;border-radius:7px;background:#fff;display:flex;align-items:center;padding:0 12px;font:13px -apple-system,Helvetica,Arial,sans-serif;color:#3b3f5c">🔒&nbsp; ${URL_LABEL}</span>
    </div>
    <img style="display:block" src="data:image/png;base64,${png}">
  </div></body></html>`;
  await send("Page.navigate", { url: "data:text/html;base64," + Buffer.from(html).toString("base64") });
  await sleep(150);
  const file = path.join(outDir, `frame-${String(i).padStart(3, "0")}.png`);
  fs.writeFileSync(file, Buffer.from(await shot(), "base64"));
  frames.push([file, seconds]);
}

// ffmpeg concat list (the last frame is repeated, as the format requires).
const list = frames.map(([f, s]) => `file '${f}'\nduration ${s}`).join("\n") + `\nfile '${frames.at(-1)[0]}'\n`;
fs.writeFileSync(path.join(outDir, "frames.txt"), list);

ws.close();
proc.kill();
server.close();
console.log(`${frames.length} frames`);
