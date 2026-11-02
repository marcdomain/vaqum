"use strict";

// Shared by install.js (postinstall), bin/vaqum.js (the command) and
// scripts/build-platform-packages.js (release CI).
//
// The native binary normally arrives as a per-platform npm package listed in
// optionalDependencies (e.g. @marcdomain/vaqum-darwin-arm64). If that package
// is missing (--omit=optional, a lockfile made on another OS, ...) we
// download it ourselves: first from the npm registry, then from the GitHub
// Release as a last resort.

const crypto = require("crypto");
const fs = require("fs");
const http = require("http");
const https = require("https");
const os = require("os");
const path = require("path");
const zlib = require("zlib");

const pkg = require("../package.json");

const SCOPE = "@marcdomain";
const GITHUB_RELEASES = "https://github.com/marcdomain/vaqum/releases/download";

// key = `${process.platform}-${process.arch}`
const PLATFORMS = {
  "darwin-x64": { target: "x86_64-apple-darwin", os: "darwin", cpu: "x64" },
  "darwin-arm64": { target: "aarch64-apple-darwin", os: "darwin", cpu: "arm64" },
  "linux-x64": { target: "x86_64-unknown-linux-gnu", os: "linux", cpu: "x64" },
  // Statically linked musl build: runs on both glibc and musl distros.
  "linux-arm64": { target: "aarch64-unknown-linux-musl", os: "linux", cpu: "arm64" },
  // Node on Termux reports platform "android", not "linux".
  "android-arm64": { target: "aarch64-linux-android", os: "android", cpu: "arm64" },
  "win32-x64": { target: "x86_64-pc-windows-msvc", os: "win32", cpu: "x64" },
};

const PLATFORM_TARGETS = Object.fromEntries(Object.entries(PLATFORMS).map(([key, p]) => [key, p.target]));

function platformKey() {
  return `${process.platform}-${process.arch}`;
}

function targetTriple(key = platformKey()) {
  const target = PLATFORM_TARGETS[key];
  if (!target) {
    throw new Error(
      `no prebuilt vaqum binary for ${key}. Supported: ${Object.keys(PLATFORM_TARGETS).join(", ")}. ` +
        "Install from source instead: https://github.com/marcdomain/vaqum#install",
    );
  }
  return target;
}

function platformPackageName(key = platformKey()) {
  return `${SCOPE}/vaqum-${key}`;
}

function isWindowsKey(key) {
  return key.startsWith("win32-");
}

// File name of the binary inside a platform package's bin/ directory.
function binaryFileName(key = platformKey()) {
  return isWindowsKey(key) ? "vaqum.exe" : "vaqum";
}

// Where install.js puts a binary it downloaded itself.
function postinstallBinaryPath(key = platformKey()) {
  return path.join(__dirname, "..", "bin", `vaqum-bin${isWindowsKey(key) ? ".exe" : ""}`);
}

function cacheDir() {
  if (process.env.VAQUM_CACHE_DIR) return process.env.VAQUM_CACHE_DIR;
  const home = os.homedir();
  if (process.platform === "win32") {
    return path.join(process.env.LOCALAPPDATA || path.join(home, "AppData", "Local"), "vaqum", "Cache");
  }
  if (process.platform === "darwin") return path.join(home, "Library", "Caches", "vaqum");
  return path.join(process.env.XDG_CACHE_HOME || path.join(home, ".cache"), "vaqum");
}

// Where bin/vaqum.js puts a binary it downloaded on first run. Lives outside
// node_modules so it works even when the global install dir is read-only.
function cacheBinaryPath(key = platformKey(), version = pkg.version) {
  return path.join(cacheDir(), version, key, binaryFileName(key));
}

// Makes sure the file can be executed. Returns false if it can't be fixed
// (e.g. a root-owned global install without the exec bit).
function ensureExecutable(file) {
  if (process.platform === "win32") return true;
  try {
    fs.accessSync(file, fs.constants.X_OK);
    return true;
  } catch {
    try {
      fs.chmodSync(file, 0o755);
      fs.accessSync(file, fs.constants.X_OK);
      return true;
    } catch {
      return false;
    }
  }
}

function isFile(file) {
  try {
    return fs.statSync(file).isFile();
  } catch {
    return false;
  }
}

// Binary shipped by the optionalDependencies platform package, if installed.
function findPlatformPackageBinary(key = platformKey()) {
  if (!(key in PLATFORMS)) return null;
  try {
    return require.resolve(`${platformPackageName(key)}/bin/${binaryFileName(key)}`);
  } catch {
    return null;
  }
}

// First binary already on disk, in priority order. Never downloads.
function findExistingBinary(key = platformKey()) {
  const candidates = [
    process.env.VAQUM_BINARY_PATH,
    findPlatformPackageBinary(key),
    postinstallBinaryPath(key),
    cacheBinaryPath(key),
  ];
  for (const file of candidates) {
    if (file && isFile(file) && ensureExecutable(file)) return file;
  }
  return null;
}

// ---------------------------------------------------------------- downloads

function get(url, { redirectsLeft = 5, headers = {} } = {}) {
  const client = url.startsWith("http://") ? http : https;
  return new Promise((resolve, reject) => {
    const req = client.get(url, { headers: { "user-agent": "vaqum-npm-installer", ...headers } }, (res) => {
      const { statusCode, headers: resHeaders } = res;
      if ([301, 302, 303, 307, 308].includes(statusCode) && resHeaders.location && redirectsLeft > 0) {
        res.resume();
        const next = new URL(resHeaders.location, url).toString();
        resolve(get(next, { redirectsLeft: redirectsLeft - 1, headers }));
        return;
      }
      if (statusCode !== 200) {
        res.resume();
        reject(new Error(`GET ${url} failed: HTTP ${statusCode}`));
        return;
      }
      const chunks = [];
      res.on("data", (chunk) => chunks.push(chunk));
      res.on("end", () => resolve(Buffer.concat(chunks)));
      res.on("error", reject);
    });
    req.setTimeout(60_000, () => req.destroy(new Error(`GET ${url} timed out`)));
    req.on("error", reject);
  });
}

// Honors the registry npm/pnpm/yarn was configured with (they export it as
// npm_config_* to install scripts), so mirrors and private registries work.
function registryUrl() {
  const env = process.env;
  const url =
    env.VAQUM_NPM_REGISTRY ||
    env[`npm_config_${SCOPE}:registry`] ||
    env.npm_config_registry ||
    "https://registry.npmjs.org/";
  return url.replace(/\/+$/, "");
}

function verifyIntegrity(data, integrity, shasum) {
  const sriEntries = (integrity || "")
    .split(/\s+/)
    .map((s) => s.match(/^(sha512|sha384|sha256)-(.+)$/))
    .filter(Boolean);
  if (sriEntries.length > 0) {
    const ok = sriEntries.some(
      ([, algo, expected]) => crypto.createHash(algo).update(data).digest("base64") === expected,
    );
    if (!ok) throw new Error("integrity check failed for downloaded package tarball");
    return;
  }
  if (shasum) {
    if (crypto.createHash("sha1").update(data).digest("hex") !== shasum) {
      throw new Error("shasum check failed for downloaded package tarball");
    }
    return;
  }
  throw new Error("registry metadata has no integrity hash; refusing unverified download");
}

// Pulls one file out of an (uncompressed) npm package tarball. npm tarballs
// put everything under a top-level "package/" directory, which is stripped.
function extractFileFromTar(tar, wanted) {
  let offset = 0;
  let paxPath = null;
  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512);
    if (header.every((b) => b === 0)) break;

    const field = (start, end) => {
      const buf = header.subarray(start, end);
      const nul = buf.indexOf(0);
      return buf.subarray(0, nul === -1 ? buf.length : nul).toString("utf8");
    };
    const name = field(0, 100);
    const size = parseInt(field(124, 136).trim() || "0", 8);
    const typeFlag = header[156];
    const prefix = field(257, 263).startsWith("ustar") ? field(345, 500) : "";
    const dataStart = offset + 512;
    const dataEnd = dataStart + size;
    if (Number.isNaN(size) || dataEnd > tar.length) throw new Error("corrupt package tarball");

    const type = typeFlag === 0 ? "0" : String.fromCharCode(typeFlag);
    if (type === "x") {
      const match = tar.subarray(dataStart, dataEnd).toString("utf8").match(/\d+ path=([^\n]*)\n/);
      paxPath = match ? match[1] : null;
    } else if (type === "g") {
      // Global PAX header: nothing we need.
    } else {
      const fullPath = paxPath || (prefix ? `${prefix}/${name}` : name);
      paxPath = null;
      const relative = fullPath.replace(/^\.?\/*[^/]+\//, "");
      if ((type === "0" || type === "7") && relative === wanted) {
        return Buffer.from(tar.subarray(dataStart, dataEnd));
      }
    }
    offset = dataStart + Math.ceil(size / 512) * 512;
  }
  throw new Error(`${wanted} not found in package tarball`);
}

async function downloadFromNpm(key, version) {
  const name = platformPackageName(key);
  const metaUrl = `${registryUrl()}/${name.replace("/", "%2f")}/${version}`;
  const meta = JSON.parse((await get(metaUrl, { headers: { accept: "application/json" } })).toString("utf8"));
  if (!meta.dist || !meta.dist.tarball) throw new Error(`no tarball listed for ${name}@${version}`);
  const tgz = await get(meta.dist.tarball);
  verifyIntegrity(tgz, meta.dist.integrity, meta.dist.shasum);
  return extractFileFromTar(zlib.gunzipSync(tgz), `bin/${binaryFileName(key)}`);
}

async function downloadFromGitHub(key, version) {
  const base = process.env.VAQUM_NPM_BASE_URL || `${GITHUB_RELEASES}/v${version}`;
  const asset = `vaqum-${version}-${targetTriple(key)}${isWindowsKey(key) ? ".exe" : ""}`;
  const [binary, sidecar] = await Promise.all([get(`${base}/${asset}`), get(`${base}/${asset}.sha256`)]);
  const expected = sidecar.toString("utf8").trim().split(/\s+/)[0];
  const actual = crypto.createHash("sha256").update(binary).digest("hex");
  if (expected !== actual) throw new Error(`checksum mismatch for ${asset}: expected ${expected}, got ${actual}`);
  return binary;
}

// npm registry first (same place the package itself came from), GitHub
// Release second.
async function downloadBinary(key = platformKey(), version = pkg.version) {
  targetTriple(key);
  const errors = [];
  for (const [source, fetcher] of [
    ["npm registry", downloadFromNpm],
    ["GitHub Release", downloadFromGitHub],
  ]) {
    try {
      return await fetcher(key, version);
    } catch (err) {
      errors.push(`${source}: ${err.message}`);
    }
  }
  throw new Error(`could not download the vaqum ${version} binary for ${key}:\n  ${errors.join("\n  ")}`);
}

// Atomic write: a concurrent run never sees a half-written binary.
function writeBinary(dest, data) {
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  const tmp = `${dest}.${process.pid}.${crypto.randomBytes(4).toString("hex")}.tmp`;
  fs.writeFileSync(tmp, data, { mode: 0o755 });
  try {
    fs.renameSync(tmp, dest);
  } catch (err) {
    fs.rmSync(tmp, { force: true });
    // Another process won the race (Windows can't replace a file in use).
    if (!isFile(dest)) throw err;
  }
  ensureExecutable(dest);
  return dest;
}

// Used by bin/vaqum.js: an existing binary, or download one into the cache.
async function resolveBinary({ log = () => {} } = {}) {
  const key = platformKey();
  targetTriple(key);
  const existing = findExistingBinary(key);
  if (existing) return existing;
  log(`vaqum: native binary not found, downloading it for ${key} (one time only) ...`);
  const data = await downloadBinary(key, pkg.version);
  return writeBinary(cacheBinaryPath(key), data);
}

module.exports = {
  PLATFORMS,
  PLATFORM_TARGETS,
  SCOPE,
  binaryFileName,
  cacheBinaryPath,
  cacheDir,
  downloadBinary,
  downloadFromGitHub,
  downloadFromNpm,
  ensureExecutable,
  extractFileFromTar,
  findExistingBinary,
  findPlatformPackageBinary,
  platformKey,
  platformPackageName,
  postinstallBinaryPath,
  registryUrl,
  resolveBinary,
  targetTriple,
  verifyIntegrity,
  writeBinary,
};
