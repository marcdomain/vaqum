"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");

const { PLATFORM_TARGETS, targetTriple } = require("./install.js");

test("every declared platform maps to a real cargo target triple", () => {
  const expected = {
    "darwin-x64": "x86_64-apple-darwin",
    "darwin-arm64": "aarch64-apple-darwin",
    "linux-x64": "x86_64-unknown-linux-gnu",
    "linux-arm64": "aarch64-unknown-linux-musl",
    "android-arm64": "aarch64-unknown-linux-musl",
    "win32-x64": "x86_64-pc-windows-msvc",
  };
  assert.deepEqual(PLATFORM_TARGETS, expected);
});

test("linux-arm64 and android-arm64 share the same musl binary", () => {
  // Termux reports process.platform as "android", not "linux" — this is
  // what actually lets `npm install` work there at all.
  assert.equal(PLATFORM_TARGETS["android-arm64"], PLATFORM_TARGETS["linux-arm64"]);
});

test("targetTriple() resolves the current platform when supported", (t) => {
  const key = `${process.platform}-${process.arch}`;
  if (!(key in PLATFORM_TARGETS)) {
    t.skip(`${key} isn't a supported target; nothing to assert here`);
    return;
  }
  assert.equal(targetTriple(), PLATFORM_TARGETS[key]);
});

test("targetTriple() throws a clear, actionable error for an unsupported platform", (t) => {
  const originalPlatform = Object.getOwnPropertyDescriptor(process, "platform");
  Object.defineProperty(process, "platform", { value: "plan9" });
  t.after(() => Object.defineProperty(process, "platform", originalPlatform));

  assert.throws(() => targetTriple(), /no prebuilt vaqum binary for plan9-.*Install from source/s);
});
