#!/usr/bin/env node
"use strict";

// Thin shim: finds the native binary (platform package, postinstall
// download, or the user cache — downloading it on first run if none exist)
// and forwards argv/stdio to it. All the real logic lives in the Rust binary.

const { spawnSync } = require("child_process");
const { resolveBinary } = require("../lib/platform.js");

async function main() {
  let binPath;
  try {
    // Log to stderr so piped stdout output is never polluted.
    binPath = await resolveBinary({ log: (msg) => console.error(msg) });
  } catch (err) {
    console.error(`vaqum: ${err.message}`);
    console.error("Try reinstalling: npm install -g @marcdomain/vaqum");
    process.exit(1);
  }

  const result = spawnSync(binPath, process.argv.slice(2), { stdio: "inherit" });

  if (result.error) {
    console.error(`vaqum: failed to launch native binary at ${binPath} (${result.error.message})`);
    process.exit(1);
  }

  process.exit(result.status === null ? 1 : result.status);
}

main();
