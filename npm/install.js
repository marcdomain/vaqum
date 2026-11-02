#!/usr/bin/env node
"use strict";

// postinstall: a fallback only. The native binary normally comes from the
// per-platform optionalDependencies package (e.g. @marcdomain/vaqum-darwin-arm64).
// If that package wasn't installed (--omit=optional, a lockfile made on
// another OS, ...), download the binary now so the first run is instant.
//
// A failed download never fails the install: bin/vaqum.js retries on first
// run. And if install scripts are disabled entirely (--ignore-scripts,
// pnpm 10, Bun), this file simply never runs and the same first-run
// download covers it.

const pkg = require("./package.json");
const {
  PLATFORM_TARGETS,
  downloadBinary,
  findPlatformPackageBinary,
  platformKey,
  platformPackageName,
  postinstallBinaryPath,
  targetTriple,
  writeBinary,
} = require("./lib/platform.js");

async function main() {
  const key = platformKey();
  targetTriple(key); // throws a clear error on unsupported platforms

  if (findPlatformPackageBinary(key)) return; // the normal path: nothing to do

  console.log(`vaqum: ${platformPackageName(key)} not installed, downloading the binary instead ...`);
  const data = await downloadBinary(key, pkg.version);
  writeBinary(postinstallBinaryPath(key), data);
  console.log(`vaqum: installed native binary for ${key}`);
}

module.exports = { PLATFORM_TARGETS, targetTriple };

if (require.main === module) {
  main().catch((err) => {
    if (/^no prebuilt vaqum binary/.test(err.message)) {
      console.error(`vaqum postinstall failed: ${err.message}`);
      process.exit(1);
    }
    console.warn(`vaqum: postinstall download failed, will retry on first run.\n${err.message}`);
  });
}
