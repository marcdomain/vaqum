// Page logic for the vaqum archive viewer. Archive contents are untrusted:
// every name is inserted with textContent, never as HTML.
import { createViewer } from "./viewer.js";

const $ = (id) => document.getElementById(id);
const els = {
  drop: $("drop"),
  input: $("file-input"),
  status: $("status"),
  passwordForm: $("password-form"),
  password: $("password"),
  result: $("result"),
  summary: $("summary"),
  warnings: $("warnings"),
  warningList: $("warning-list"),
  rows: $("rows"),
  downloadAll: $("download-all"),
};

let viewer;
let current = null; // { name, bytes }

const viewerReady = createViewer("vaqum-viewer.wasm").then((v) => (viewer = v));

function human(bytes) {
  const units = ["B", "kB", "MB", "GB"];
  let n = bytes;
  let u = 0;
  while (n >= 1000 && u < units.length - 1) {
    n /= 1000;
    u++;
  }
  return `${u === 0 ? n : n.toFixed(2)} ${units[u]}`;
}

function setStatus(text, kind = "") {
  els.status.textContent = text;
  els.status.className = `status ${kind}`;
  els.status.hidden = !text;
}

function download(bytes, name) {
  const safe = name.split("/").pop().replace(/[\u0000-\u001f\u007f‪-‮⁦-⁩]/g, "_") || "file";
  const url = URL.createObjectURL(new Blob([bytes], { type: "application/octet-stream" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = safe;
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

async function openFile(file) {
  els.result.hidden = true;
  els.passwordForm.hidden = true;
  setStatus(`Reading ${file.name}…`);
  current = { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()) };
  await viewerReady;
  tryOpen("");
}

function tryOpen(password) {
  setStatus(password ? "Decrypting…" : "Opening…");
  // Let the status paint before the (synchronous) work starts.
  setTimeout(() => {
    const result = viewer.open(current.bytes, password);
    if (!result.ok) {
      if (result.needsPassword) {
        els.passwordForm.hidden = false;
        els.password.value = "";
        els.password.focus();
        setStatus(password ? result.error : "This archive is password-protected.", password ? "error" : "");
      } else {
        setStatus(result.error, "error");
      }
      return;
    }
    els.passwordForm.hidden = true;
    setStatus("");
    render(result);
  }, 30);
}

function render(result) {
  const files = result.entries.filter((e) => e.kind === "file");
  els.summary.textContent =
    `${current.name} · ${result.kind} · ${files.length} file${files.length === 1 ? "" : "s"}, ` +
    `${human(result.originalSize)} · ${result.algorithm}` +
    (result.encrypted ? " · encrypted" : "") +
    (result.verified ? " · checksum verified ✓" : "");

  els.rows.replaceChildren();
  els.warningList.replaceChildren();
  let fileIndex = 0;
  for (const entry of result.entries) {
    const tr = document.createElement("tr");
    const name = document.createElement("td");
    name.className = "path";
    name.textContent = entry.kind === "dir" ? `${entry.path}/` : entry.path;
    if (entry.warnings?.length) {
      const flag = document.createElement("span");
      flag.className = "flag";
      flag.textContent = "⚠";
      flag.title = entry.warnings.join("\n");
      name.prepend(flag);
      for (const reason of entry.warnings) {
        const li = document.createElement("li");
        const strong = document.createElement("strong");
        strong.textContent = entry.path;
        li.append(strong, ` — ${reason}`);
        els.warningList.append(li);
      }
    }
    const size = document.createElement("td");
    size.className = "size";
    size.textContent = entry.kind === "file" ? human(entry.size) : "—";
    const action = document.createElement("td");
    if (entry.kind === "file") {
      const index = fileIndex++;
      const button = document.createElement("button");
      button.type = "button";
      button.className = "link";
      button.textContent = "Download";
      button.addEventListener("click", () => download(viewer.file(index), viewer.filePath(index)));
      action.append(button);
    }
    tr.append(name, size, action);
    els.rows.append(tr);
  }
  els.warnings.hidden = els.warningList.children.length === 0;
  els.downloadAll.hidden = files.length < 2;
  els.result.hidden = false;
}

els.downloadAll.addEventListener("click", () => {
  const base = current.name.replace(/\.(vqm|vaqum)$/i, "") || "archive";
  download(viewer.zipAll(), `${base}.zip`);
});

els.passwordForm.addEventListener("submit", (event) => {
  event.preventDefault();
  if (els.password.value) tryOpen(els.password.value);
});

els.input.addEventListener("change", () => {
  if (els.input.files[0]) openFile(els.input.files[0]);
});

for (const type of ["dragenter", "dragover"]) {
  els.drop.addEventListener(type, (e) => {
    e.preventDefault();
    els.drop.classList.add("over");
  });
}
for (const type of ["dragleave", "drop"]) {
  els.drop.addEventListener(type, () => els.drop.classList.remove("over"));
}
els.drop.addEventListener("drop", (e) => {
  e.preventDefault();
  const file = e.dataTransfer.files[0];
  if (file) openFile(file);
});
els.drop.addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    els.input.click();
  }
});

viewerReady.catch((err) => setStatus(`Couldn't load the viewer: ${err.message}`, "error"));
