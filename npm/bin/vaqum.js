#!/usr/bin/env node
"use strict";

// Thin shim: forwards argv/stdio straight to the native binary installed
// by ../install.js. All the real logic lives in the Rust binary.

const path = require("path");
const { spawnSync } = require("child_process");

const isWindows = process.platform === "win32";
const binPath = path.join(__dirname, "..", "bin", `vaqum-bin${isWindows ? ".exe" : ""}`);

const result = spawnSync(binPath, process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  if (result.error.code === "ENOENT") {
    console.error(
      "vaqum: native binary not found. The postinstall step may have failed — try reinstalling: npm install -g vaqum",
    );
  } else {
    console.error(`vaqum: failed to launch native binary (${result.error.message})`);
  }
  process.exit(1);
}

process.exit(result.status === null ? 1 : result.status);
