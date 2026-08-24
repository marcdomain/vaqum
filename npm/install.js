#!/usr/bin/env node
"use strict";

// postinstall: fetches the prebuilt native vaqum binary for this platform
// from the matching GitHub Release and verifies its checksum. No tar/zip
// dependency needed — the release also publishes each binary as a bare
// asset alongside the .tar.gz, specifically for this script to fetch.

const crypto = require("crypto");
const fs = require("fs");
const http = require("http");
const https = require("https");
const path = require("path");

const pkg = require("./package.json");

// Override for local testing — points the download at a local server
// instead of GitHub Releases. Unset in normal use.
const BASE_URL = process.env.VAQUM_NPM_BASE_URL || `https://github.com/marcdomain/vaqum/releases/download/v${pkg.version}`;

const PLATFORM_TARGETS = {
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-musl",
  // Node on Termux reports platform "android", not "linux" — same static
  // musl binary serves both.
  "android-arm64": "aarch64-unknown-linux-musl",
  "win32-x64": "x86_64-pc-windows-msvc",
};

function targetTriple() {
  const key = `${process.platform}-${process.arch}`;
  const target = PLATFORM_TARGETS[key];
  if (!target) {
    throw new Error(
      `no prebuilt vaqum binary for ${key}. Supported: ${Object.keys(PLATFORM_TARGETS).join(", ")}. ` +
        "Install from source instead: https://github.com/marcdomain/vaqum#install",
    );
  }
  return target;
}

function get(url, redirectsLeft = 5) {
  const client = url.startsWith("http://") ? http : https;
  return new Promise((resolve, reject) => {
    client
      .get(url, { headers: { "user-agent": "vaqum-npm-installer" } }, (res) => {
        const { statusCode, headers } = res;
        if ([301, 302, 303, 307, 308].includes(statusCode) && headers.location && redirectsLeft > 0) {
          res.resume();
          resolve(get(headers.location, redirectsLeft - 1));
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
      })
      .on("error", reject);
  });
}

async function main() {
  const target = targetTriple();
  const isWindows = target.includes("windows");
  const asset = `vaqum-${pkg.version}-${target}${isWindows ? ".exe" : ""}`;

  console.log(`vaqum: downloading ${asset} ...`);
  const [binary, sha256Sidecar] = await Promise.all([
    get(`${BASE_URL}/${asset}`),
    get(`${BASE_URL}/${asset}.sha256`),
  ]);

  const expected = sha256Sidecar.toString("utf8").trim().split(/\s+/)[0];
  const actual = crypto.createHash("sha256").update(binary).digest("hex");
  if (expected !== actual) {
    throw new Error(`checksum mismatch for ${asset}: expected ${expected}, got ${actual}`);
  }

  const destDir = path.join(__dirname, "bin");
  fs.mkdirSync(destDir, { recursive: true });
  const destPath = path.join(destDir, `vaqum-bin${isWindows ? ".exe" : ""}`);
  fs.writeFileSync(destPath, binary, { mode: 0o755 });
  fs.chmodSync(destPath, 0o755);
  console.log(`vaqum: installed native binary for ${target}`);
}

module.exports = { PLATFORM_TARGETS, targetTriple };

if (require.main === module) {
  main().catch((err) => {
    console.error(`vaqum postinstall failed: ${err.message}`);
    process.exit(1);
  });
}
