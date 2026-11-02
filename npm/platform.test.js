"use strict";

const assert = require("node:assert/strict");
const { execFile, execFileSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const zlib = require("node:zlib");
const { promisify } = require("node:util");

const platform = require("./lib/platform.js");
const { buildPlatformPackages } = require("./scripts/build-platform-packages.js");
const pkg = require("./package.json");

const execFileAsync = promisify(execFile);
const NPM = process.platform === "win32" ? "npm.cmd" : "npm";
const HOST_KEY = platform.platformKey();
const HOST_SUPPORTED = HOST_KEY in platform.PLATFORMS;
const CAN_EXEC_FAKE = HOST_SUPPORTED && process.platform !== "win32";
const VERSION = pkg.version;

function tmpDir(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "vaqum-npm-test-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

// A shell script standing in for the native binary.
function fakeBinary(key) {
  return Buffer.from(`#!/bin/sh\necho "fake-vaqum ${key} $*"\n`);
}

// Fake release dist/ as the build job produces it.
function makeDist(dir) {
  fs.mkdirSync(dir, { recursive: true });
  for (const [key, p] of Object.entries(platform.PLATFORMS)) {
    const asset = path.join(dir, `vaqum-${VERSION}-${p.target}${key.startsWith("win32") ? ".exe" : ""}`);
    const data = fakeBinary(key);
    fs.writeFileSync(asset, data);
    const sha = crypto.createHash("sha256").update(data).digest("hex");
    fs.writeFileSync(`${asset}.sha256`, `${sha}  ${path.basename(asset)}\n`);
  }
}

function buildIntoTmp(t) {
  const dir = tmpDir(t);
  makeDist(path.join(dir, "dist"));
  const mainPackageJson = path.join(dir, "package.json");
  fs.copyFileSync(path.join(__dirname, "package.json"), mainPackageJson);
  const built = buildPlatformPackages({ version: VERSION, dist: path.join(dir, "dist"), out: path.join(dir, "platforms"), mainPackageJson });
  return { dir, built, mainPackageJson };
}

function npmPack(pkgDir, dest) {
  const out = execFileSync(NPM, ["pack", "--json", "--pack-destination", dest], {
    cwd: pkgDir,
    encoding: "utf8",
    shell: process.platform === "win32",
  });
  return path.join(dest, JSON.parse(out)[0].filename);
}

// Minimal fake npm registry + GitHub releases host.
async function startServer(t, routes) {
  const hits = [];
  const server = http.createServer((req, res) => {
    hits.push(req.url);
    const body = routes[req.url];
    if (body === undefined) {
      res.writeHead(404).end("not found");
      return;
    }
    res.writeHead(200).end(typeof body === "function" ? body() : body);
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() => {
    server.closeAllConnections();
    return new Promise((resolve) => server.close(resolve));
  });
  return { base: `http://127.0.0.1:${server.address().port}`, hits };
}

function registryRoutes(base, tarballPath, key) {
  const tgz = fs.readFileSync(tarballPath);
  const integrity = `sha512-${crypto.createHash("sha512").update(tgz).digest("base64")}`;
  const name = platform.platformPackageName(key);
  return {
    [`/${name.replace("/", "%2f")}/${VERSION}`]: JSON.stringify({
      name,
      version: VERSION,
      dist: { tarball: `${base}/tarballs/${key}.tgz`, integrity },
    }),
    [`/tarballs/${key}.tgz`]: tgz,
  };
}

function withEnv(t, vars) {
  const saved = {};
  for (const [k, v] of Object.entries(vars)) {
    saved[k] = process.env[k];
    if (v === undefined) delete process.env[k];
    else process.env[k] = v;
  }
  t.after(() => {
    for (const [k, v] of Object.entries(saved)) {
      if (v === undefined) delete process.env[k];
      else process.env[k] = v;
    }
  });
}

// Copy of the publishable npm package in a temp dir, so tests can run
// install.js / bin/vaqum.js without touching the repo.
function stagePackage(t) {
  const dir = tmpDir(t);
  for (const rel of ["package.json", "install.js", "bin/vaqum.js", "lib/platform.js"]) {
    fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true });
    fs.copyFileSync(path.join(__dirname, rel), path.join(dir, rel));
  }
  return dir;
}

function childEnv(extra) {
  const env = { ...process.env, ...extra };
  for (const k of Object.keys(env)) if (k.startsWith("npm_config_")) delete env[k];
  delete env.VAQUM_BINARY_PATH;
  return env;
}

// ------------------------------------------------------------ package shape

test("optionalDependencies list exactly one package per platform, pinned to this version", () => {
  const expected = Object.fromEntries(Object.keys(platform.PLATFORMS).map((k) => [platform.platformPackageName(k), VERSION]));
  assert.deepEqual(pkg.optionalDependencies, expected);
});

test("main package os/cpu fields cover every platform package", () => {
  for (const p of Object.values(platform.PLATFORMS)) {
    assert.ok(pkg.os.includes(p.os), `os ${p.os} missing`);
    assert.ok(pkg.cpu.includes(p.cpu), `cpu ${p.cpu} missing`);
  }
});

test("published main package contains the runtime files and nothing else", () => {
  const out = execFileSync(NPM, ["pack", "--dry-run", "--json", "--ignore-scripts"], {
    cwd: __dirname,
    encoding: "utf8",
    shell: process.platform === "win32",
  });
  const files = JSON.parse(out)[0].files.map((f) => f.path).sort();
  assert.deepEqual(files, ["README.md", "bin/vaqum.js", "install.js", "lib/platform.js", "package.json"]);
});

// ------------------------------------------------------- platform packages

test("build script creates a correct, installable package per platform", (t) => {
  const { dir, built, mainPackageJson } = buildIntoTmp(t);
  assert.equal(built.length, Object.keys(platform.PLATFORMS).length);

  for (const { key, name, dir: pkgDir } of built) {
    const p = JSON.parse(fs.readFileSync(path.join(pkgDir, "package.json"), "utf8"));
    assert.equal(p.name, name);
    assert.equal(p.version, VERSION);
    assert.deepEqual(p.os, [platform.PLATFORMS[key].os]);
    assert.deepEqual(p.cpu, [platform.PLATFORMS[key].cpu]);
    assert.equal(p.repository.url, pkg.repository.url, "provenance needs a matching repository url");

    // Pack it the way `npm publish` would, then read it back like the fallback does.
    const tgz = npmPack(pkgDir, dir);
    const tar = zlib.gunzipSync(fs.readFileSync(tgz));
    const extracted = platform.extractFileFromTar(tar, `bin/${platform.binaryFileName(key)}`);
    assert.deepEqual(extracted, fakeBinary(key));
  }

  const main = JSON.parse(fs.readFileSync(mainPackageJson, "utf8"));
  assert.deepEqual(main.optionalDependencies, pkg.optionalDependencies);
});

test("packed unix binaries keep their executable bit", { skip: process.platform === "win32" }, (t) => {
  const { dir, built } = buildIntoTmp(t);
  const linux = built.find((b) => b.key === "linux-x64");
  const tar = zlib.gunzipSync(fs.readFileSync(npmPack(linux.dir, dir)));
  // Walk headers to find bin/vaqum and read its mode field.
  let offset = 0;
  let mode = null;
  while (offset + 512 <= tar.length) {
    const h = tar.subarray(offset, offset + 512);
    if (h.every((b) => b === 0)) break;
    const name = h.subarray(0, 100).toString("utf8").replace(/\0.*$/s, "");
    const size = parseInt(h.subarray(124, 136).toString("utf8").replace(/\0.*$/s, "").trim() || "0", 8);
    if (name.endsWith("bin/vaqum")) mode = parseInt(h.subarray(100, 108).toString("utf8").replace(/\0.*$/s, "").trim(), 8);
    offset += 512 + Math.ceil(size / 512) * 512;
  }
  assert.ok(mode !== null, "bin/vaqum not found in tarball");
  assert.ok(mode & 0o111, `bin/vaqum mode ${mode.toString(8)} is not executable`);
});

test("build script refuses to run when a platform binary is missing", (t) => {
  const dir = tmpDir(t);
  makeDist(path.join(dir, "dist"));
  fs.rmSync(path.join(dir, "dist", `vaqum-${VERSION}-x86_64-pc-windows-msvc.exe`));
  fs.copyFileSync(path.join(__dirname, "package.json"), path.join(dir, "package.json"));
  assert.throws(
    () =>
      buildPlatformPackages({ version: VERSION, dist: path.join(dir, "dist"), out: path.join(dir, "out"), mainPackageJson: path.join(dir, "package.json") }),
    /missing binary for win32-x64/,
  );
});

test("build script refuses a binary whose checksum doesn't match", (t) => {
  const dir = tmpDir(t);
  makeDist(path.join(dir, "dist"));
  fs.appendFileSync(path.join(dir, "dist", `vaqum-${VERSION}-x86_64-apple-darwin`), "tampered");
  fs.copyFileSync(path.join(__dirname, "package.json"), path.join(dir, "package.json"));
  assert.throws(
    () =>
      buildPlatformPackages({ version: VERSION, dist: path.join(dir, "dist"), out: path.join(dir, "out"), mainPackageJson: path.join(dir, "package.json") }),
    /checksum mismatch/,
  );
});

// ---------------------------------------------------------------- downloads

test("downloads from the npm registry and verifies integrity", async (t) => {
  const { dir, built } = buildIntoTmp(t);
  const key = "linux-x64";
  const tgz = npmPack(built.find((b) => b.key === key).dir, dir);
  const routes = {};
  const server = await startServer(t, routes);
  Object.assign(routes, registryRoutes(server.base, tgz, key));
  withEnv(t, { VAQUM_NPM_REGISTRY: `${server.base}/` });

  assert.deepEqual(await platform.downloadFromNpm(key, VERSION), fakeBinary(key));
});

test("rejects a registry tarball that fails its integrity check", async (t) => {
  const { dir, built } = buildIntoTmp(t);
  const key = "darwin-arm64";
  const tgz = npmPack(built.find((b) => b.key === key).dir, dir);
  const routes = {};
  const server = await startServer(t, routes);
  Object.assign(routes, registryRoutes(server.base, tgz, key));
  const metaPath = `/${platform.platformPackageName(key).replace("/", "%2f")}/${VERSION}`;
  const meta = JSON.parse(routes[metaPath]);
  meta.dist.integrity = `sha512-${crypto.createHash("sha512").update("other").digest("base64")}`;
  routes[metaPath] = JSON.stringify(meta);
  withEnv(t, { VAQUM_NPM_REGISTRY: server.base });

  await assert.rejects(platform.downloadFromNpm(key, VERSION), /integrity check failed/);
});

test("falls back to the GitHub Release when the registry can't serve it", async (t) => {
  const key = "win32-x64";
  const asset = `vaqum-${VERSION}-x86_64-pc-windows-msvc.exe`;
  const data = fakeBinary(key);
  const sha = crypto.createHash("sha256").update(data).digest("hex");
  const server = await startServer(t, {
    [`/gh/${asset}`]: data,
    [`/gh/${asset}.sha256`]: `${sha}  ${asset}\n`,
  });
  withEnv(t, { VAQUM_NPM_REGISTRY: server.base, VAQUM_NPM_BASE_URL: `${server.base}/gh` });

  assert.deepEqual(await platform.downloadBinary(key, VERSION), data);
});

test("reports both sources when every download fails", async (t) => {
  const server = await startServer(t, {});
  withEnv(t, { VAQUM_NPM_REGISTRY: server.base, VAQUM_NPM_BASE_URL: `${server.base}/gh` });
  await assert.rejects(platform.downloadBinary("linux-x64", VERSION), /npm registry: .*HTTP 404[\s\S]*GitHub Release: .*HTTP 404/);
});

test("registry URL follows npm's configured registry, scoped registry first", (t) => {
  withEnv(t, {
    VAQUM_NPM_REGISTRY: undefined,
    npm_config_registry: "https://mirror.example.com/npm/",
    "npm_config_@marcdomain:registry": undefined,
  });
  assert.equal(platform.registryUrl(), "https://mirror.example.com/npm");
  process.env["npm_config_@marcdomain:registry"] = "https://scoped.example.com/";
  assert.equal(platform.registryUrl(), "https://scoped.example.com");
});

test("tar reader handles ustar prefixes and PAX long paths", () => {
  function header(name, size, type, prefix = "") {
    const h = Buffer.alloc(512);
    h.write(name, 0);
    h.write(size.toString(8).padStart(11, "0"), 124);
    h.write(type, 156);
    h.write("ustar\0", 257);
    h.write("00", 263);
    h.write(prefix, 345);
    return h;
  }
  const pad = (b) => Buffer.concat([b, Buffer.alloc((512 - (b.length % 512)) % 512)]);
  const payload = Buffer.from("binary!");
  const paxBody = Buffer.from("26 path=package/bin/vaqum\n");
  const tar = Buffer.concat([
    header("package/README.md", 2, "0"),
    pad(Buffer.from("hi")),
    header("PaxHeader", paxBody.length, "x"),
    pad(paxBody),
    header("ignored-by-pax", payload.length, "0"),
    pad(payload),
    header("vaqum.exe", 3, "0", "package/bin"),
    pad(Buffer.from("win")),
    Buffer.alloc(1024),
  ]);
  assert.deepEqual(platform.extractFileFromTar(tar, "bin/vaqum"), payload);
  assert.deepEqual(platform.extractFileFromTar(tar, "bin/vaqum.exe"), Buffer.from("win"));
  assert.throws(() => platform.extractFileFromTar(tar, "bin/missing"), /not found/);
});

// -------------------------------------------------------------- end to end

test("e2e: platform package installed -> postinstall does nothing, command uses it", { skip: !CAN_EXEC_FAKE }, async (t) => {
  const { built } = buildIntoTmp(t);
  const staged = stagePackage(t);
  const host = built.find((b) => b.key === HOST_KEY);
  const dest = path.join(staged, "node_modules", ...platform.platformPackageName(HOST_KEY).split("/"));
  fs.cpSync(host.dir, dest, { recursive: true });
  const env = childEnv({ VAQUM_NPM_REGISTRY: "http://127.0.0.1:9", VAQUM_CACHE_DIR: path.join(staged, "cache") });

  const install = await execFileAsync(process.execPath, ["install.js"], { cwd: staged, env });
  assert.equal(install.stdout, "");
  assert.ok(!fs.existsSync(platform.postinstallBinaryPath(HOST_KEY).replace(__dirname, staged)));

  const run = await execFileAsync(process.execPath, ["bin/vaqum.js", "compress", "x"], { cwd: staged, env });
  assert.equal(run.stdout, `fake-vaqum ${HOST_KEY} compress x\n`);
});

test("e2e: --omit=optional -> postinstall downloads from npm registry", { skip: !CAN_EXEC_FAKE }, async (t) => {
  const { dir, built } = buildIntoTmp(t);
  const staged = stagePackage(t);
  const routes = {};
  const server = await startServer(t, routes);
  Object.assign(routes, registryRoutes(server.base, npmPack(built.find((b) => b.key === HOST_KEY).dir, dir), HOST_KEY));
  const env = childEnv({ VAQUM_NPM_REGISTRY: server.base, VAQUM_CACHE_DIR: path.join(staged, "cache") });

  await execFileAsync(process.execPath, ["install.js"], { cwd: staged, env });
  assert.ok(fs.existsSync(path.join(staged, "bin", "vaqum-bin")));

  const hitsBefore = server.hits.length;
  const run = await execFileAsync(process.execPath, ["bin/vaqum.js", "info", "f"], { cwd: staged, env });
  assert.equal(run.stdout, `fake-vaqum ${HOST_KEY} info f\n`);
  assert.equal(server.hits.length, hitsBefore, "command must not download again");
});

test("e2e: postinstall download failure doesn't fail the install", { skip: !HOST_SUPPORTED }, async (t) => {
  const staged = stagePackage(t);
  const server = await startServer(t, {});
  const env = childEnv({ VAQUM_NPM_REGISTRY: server.base, VAQUM_NPM_BASE_URL: `${server.base}/gh` });
  const result = await execFileAsync(process.execPath, ["install.js"], { cwd: staged, env });
  assert.match(result.stderr, /will retry on first run/);
});

test("e2e: --omit=optional + scripts blocked -> command downloads on first run, then uses cache", { skip: !CAN_EXEC_FAKE }, async (t) => {
  const { dir, built } = buildIntoTmp(t);
  const staged = stagePackage(t);
  const routes = {};
  const server = await startServer(t, routes);
  Object.assign(routes, registryRoutes(server.base, npmPack(built.find((b) => b.key === HOST_KEY).dir, dir), HOST_KEY));
  const cache = path.join(staged, "cache");
  const env = childEnv({ VAQUM_NPM_REGISTRY: server.base, VAQUM_CACHE_DIR: cache });

  const first = await execFileAsync(process.execPath, ["bin/vaqum.js", "--version"], { cwd: staged, env });
  assert.equal(first.stdout, `fake-vaqum ${HOST_KEY} --version\n`, "stdout must only carry the binary's output");
  assert.match(first.stderr, /downloading it for/);
  assert.ok(fs.existsSync(path.join(cache, VERSION, HOST_KEY, "vaqum")));

  // Second run works with the server gone.
  const offline = childEnv({ VAQUM_NPM_REGISTRY: "http://127.0.0.1:9", VAQUM_NPM_BASE_URL: "http://127.0.0.1:9", VAQUM_CACHE_DIR: cache });
  const second = await execFileAsync(process.execPath, ["bin/vaqum.js", "again"], { cwd: staged, env: offline });
  assert.equal(second.stdout, `fake-vaqum ${HOST_KEY} again\n`);
  assert.equal(second.stderr, "");
});

test("e2e: command exits non-zero with a clear message when nothing can be downloaded", { skip: !HOST_SUPPORTED }, async (t) => {
  const staged = stagePackage(t);
  const server = await startServer(t, {});
  const env = childEnv({ VAQUM_NPM_REGISTRY: server.base, VAQUM_NPM_BASE_URL: `${server.base}/gh`, VAQUM_CACHE_DIR: path.join(staged, "cache") });
  await assert.rejects(execFileAsync(process.execPath, ["bin/vaqum.js"], { cwd: staged, env }), (err) => {
    assert.equal(err.code, 1);
    assert.match(err.stderr, /could not download[\s\S]*npm install -g @marcdomain\/vaqum/);
    return true;
  });
});

test("e2e: VAQUM_BINARY_PATH overrides everything", { skip: !CAN_EXEC_FAKE }, async (t) => {
  const staged = stagePackage(t);
  const custom = path.join(staged, "custom-vaqum");
  fs.writeFileSync(custom, "#!/bin/sh\necho custom $*\n", { mode: 0o755 });
  const env = childEnv({ VAQUM_BINARY_PATH: custom, VAQUM_NPM_REGISTRY: "http://127.0.0.1:9" });
  env.VAQUM_BINARY_PATH = custom;
  const run = await execFileAsync(process.execPath, ["bin/vaqum.js", "x"], { cwd: staged, env });
  assert.equal(run.stdout, "custom x\n");
});

test("e2e: exit code of the native binary is passed through", { skip: !CAN_EXEC_FAKE }, async (t) => {
  const staged = stagePackage(t);
  const custom = path.join(staged, "custom-vaqum");
  fs.writeFileSync(custom, "#!/bin/sh\nexit 3\n", { mode: 0o755 });
  const env = childEnv({});
  env.VAQUM_BINARY_PATH = custom;
  await assert.rejects(execFileAsync(process.execPath, ["bin/vaqum.js"], { cwd: staged, env }), (err) => err.code === 3);
});
