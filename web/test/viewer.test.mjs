// Opens archives made by the real vaqum CLI through the WebAssembly viewer
// (the same loader the web page uses) and checks every byte.
//
//   VAQUM_BIN=target/debug/vaqum node --test web/test/
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { createViewer } from "../site/viewer.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../..");
const BIN = process.env.VAQUM_BIN ? path.resolve(process.env.VAQUM_BIN) : path.join(repo, "target/debug/vaqum");
const viewer = await createViewer(fs.readFileSync(path.join(here, "../site/vaqum-viewer.wasm")));

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "vaqum-viewer-"));
test.after(() => fs.rmSync(tmp, { recursive: true, force: true }));
const env = { ...process.env, XDG_CONFIG_HOME: path.join(tmp, "cfg"), APPDATA: path.join(tmp, "cfg") };
const vaqum = (...args) => execFileSync(BIN, args, { cwd: tmp, env, stdio: ["ignore", "pipe", "pipe"] });

// Sample tree: text, binary, duplicate, nested.
const src = path.join(tmp, "tree");
fs.mkdirSync(path.join(src, "nested"), { recursive: true });
fs.writeFileSync(path.join(src, "a.txt"), Array.from({ length: 3000 }, (_, i) => `line ${i}\n`).join(""));
fs.writeFileSync(path.join(src, "nested/b.bin"), Buffer.from(Array.from({ length: 70000 }, (_, i) => (i * 7919) % 256)));
fs.copyFileSync(path.join(src, "a.txt"), path.join(src, "nested/dup.txt"));
const original = (rel) => fs.readFileSync(path.join(src, rel));

function filesOf() {
  const out = {};
  for (let i = 0; i < viewer.fileCount(); i++) out[viewer.filePath(i)] = viewer.file(i);
  return out;
}

function expectTree(result, prefix) {
  assert.equal(result.ok, true, result.error);
  assert.equal(result.verified, true);
  const files = filesOf();
  assert.deepEqual(Object.keys(files).sort(), ["a.txt", "nested/b.bin", "nested/dup.txt"].map((p) => prefix + p));
  for (const rel of ["a.txt", "nested/b.bin", "nested/dup.txt"]) {
    assert.ok(Buffer.from(files[prefix + rel]).equals(original(rel)), `${rel} differs`);
  }
}

for (const [label, flags] of [
  ["zstd", []],
  ["xz (--max)", ["--max"]],
  ["dedup", ["--dedup"]],
]) {
  test(`opens a directory archive: ${label}`, () => {
    vaqum("compress", src, "-r", ...flags, "-o", "t.vqm");
    const result = viewer.open(fs.readFileSync(path.join(tmp, "t.vqm")));
    assert.equal(result.kind, "directory archive");
    expectTree(result, "tree/");
  });
}

test("opens a single-file archive", () => {
  vaqum("compress", path.join(src, "nested/b.bin"), "-o", "one.vqm");
  const result = viewer.open(fs.readFileSync(path.join(tmp, "one.vqm")));
  assert.equal(result.ok, true, result.error);
  assert.equal(result.kind, "single file");
  assert.ok(Buffer.from(viewer.file(0)).equals(original("nested/b.bin")));
});

test("opens a multi-path bundle", () => {
  vaqum("compress", path.join(src, "a.txt"), path.join(src, "nested"), "-r", "-o", "bundle.vqm");
  const result = viewer.open(fs.readFileSync(path.join(tmp, "bundle.vqm")));
  assert.equal(result.ok, true, result.error);
  assert.equal(result.kind, "multi-path bundle");
  const files = filesOf();
  assert.ok(Buffer.from(files["a.txt"]).equals(original("a.txt")));
  assert.ok(Buffer.from(files["nested/b.bin"]).equals(original("nested/b.bin")));
});

test("password archives: asks for it, rejects a wrong one, opens with the right one", () => {
  fs.writeFileSync(path.join(tmp, "key"), "correct horse battery staple");
  vaqum("compress", src, "-r", "--key-file", "key", "-o", "enc.vqm");
  const bytes = fs.readFileSync(path.join(tmp, "enc.vqm"));
  let result = viewer.open(bytes);
  assert.equal(result.ok, false);
  assert.equal(result.needsPassword, true);
  result = viewer.open(bytes, "wrong password");
  assert.equal(result.ok, false);
  assert.equal(result.needsPassword, true);
  assert.match(result.error, /Wrong password/);
  assert.equal(viewer.fileCount(), 0, "nothing may be exposed after a failed attempt");
  result = viewer.open(bytes, "correct horse battery staple");
  assert.equal(result.encrypted, true);
  expectTree(result, "tree/");
});

test("opens archives encrypted by vaqum 1.2.5 (older key settings)", () => {
  const fixtures = path.join(repo, "tests/fixtures");
  const result = viewer.open(
    fs.readFileSync(path.join(fixtures, "legacy-1.2.5-encrypted.vaqum")),
    fs.readFileSync(path.join(fixtures, "legacy.key"), "utf8"),
  );
  assert.equal(result.ok, true, result.error);
  assert.equal(new TextDecoder().decode(viewer.file(0)), "legacy secret\n");
});

test("recipient archives point people to the CLI", () => {
  vaqum("keygen");
  const me = vaqum("pubkey").toString().trim();
  vaqum("compress", path.join(src, "a.txt"), "--to", me, "-o", "to.vqm");
  const result = viewer.open(fs.readFileSync(path.join(tmp, "to.vqm")));
  assert.equal(result.ok, false);
  assert.equal(result.needsPassword, false);
  assert.match(result.error, /command-line tool/);
});

test("hostile archives are refused or contained", () => {
  // Header naming the output "../escaped": refused by vaqum's own header parser.
  const header = Buffer.concat([
    Buffer.from("VAQM"), Buffer.from([2, 0, 0, 0]), Buffer.from([6, 0, 0, 0, 0, 0, 0, 0]),
    Buffer.alloc(55), Buffer.from([10, 0]), Buffer.from("../escaped"),
  ]);
  const result = viewer.open(header);
  assert.equal(result.ok, false);
  assert.match(result.error, /plain file name/);
  // Random garbage never crashes the viewer.
  for (let i = 0; i < 200; i++) {
    const junk = Buffer.from(Array.from({ length: 64 + (i % 300) }, () => Math.floor(Math.random() * 256)));
    if (i % 2) Buffer.from("VAQM\x02").copy(junk);
    assert.equal(typeof viewer.open(junk).ok, "boolean");
  }
});

test("risky contents are flagged", () => {
  const risky = path.join(tmp, "risky");
  fs.mkdirSync(risky);
  fs.writeFileSync(path.join(risky, "invoice.pdf"), Buffer.from("MZ\x90\x00 not a pdf"));
  fs.writeFileSync(path.join(risky, "fine.txt"), "ok");
  vaqum("compress", risky, "-r", "-o", "risky.vqm");
  const result = viewer.open(fs.readFileSync(path.join(tmp, "risky.vqm")));
  const invoice = result.entries.find((e) => e.path === "risky/invoice.pdf");
  assert.match(invoice.warnings[0], /Windows program/);
  assert.equal(result.entries.find((e) => e.path === "risky/fine.txt").warnings, undefined);
});

test("download-all produces a zip that real zip readers accept", () => {
  vaqum("compress", src, "-r", "-o", "t.vqm");
  viewer.open(fs.readFileSync(path.join(tmp, "t.vqm")));
  fs.writeFileSync(path.join(tmp, "all.zip"), viewer.zipAll());
  // vaqum's own zip support (the zip crate) must read it back intact.
  const listing = JSON.parse(vaqum("list", "--json", "all.zip").toString());
  assert.deepEqual(listing.entries.map((e) => e.path).sort(), ["tree/a.txt", "tree/nested/b.bin", "tree/nested/dup.txt"]);
  vaqum("decompress", "all.zip", "-o", "unzipped");
  assert.ok(fs.readFileSync(path.join(tmp, "unzipped/tree/nested/b.bin")).equals(original("nested/b.bin")));
});
