#!/usr/bin/env node
"use strict";

// Release CI helper. Turns the bare binaries from the build job into one
// npm package per platform, and pins the main package's
// optionalDependencies to the same version.
//
//   node scripts/build-platform-packages.js --version 1.2.5 --dist ../dist --out platforms
//
// Fails if any platform's binary or checksum is missing/wrong, so the main
// package can never point at a platform package that wasn't published.

const crypto = require("crypto");
const fs = require("fs");
const path = require("path");

const { PLATFORMS, binaryFileName, platformPackageName } = require("../lib/platform.js");

const NPM_DIR = path.join(__dirname, "..");

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i += 2) {
    const flag = argv[i];
    const value = argv[i + 1];
    if (!flag.startsWith("--") || value === undefined) throw new Error(`bad argument: ${flag}`);
    args[flag.slice(2)] = value;
  }
  for (const required of ["version", "dist", "out"]) {
    if (!args[required]) throw new Error(`missing --${required}`);
  }
  return args;
}

function buildPlatformPackages({ version, dist, out, mainPackageJson = path.join(NPM_DIR, "package.json") }) {
  const mainPkg = JSON.parse(fs.readFileSync(mainPackageJson, "utf8"));
  if (mainPkg.version !== version) {
    throw new Error(`version mismatch: ${mainPackageJson} is ${mainPkg.version}, requested ${version}`);
  }

  fs.rmSync(out, { recursive: true, force: true });
  const built = [];

  for (const [key, platform] of Object.entries(PLATFORMS)) {
    const ext = key.startsWith("win32-") ? ".exe" : "";
    const asset = path.join(dist, `vaqum-${version}-${platform.target}${ext}`);
    const sidecar = `${asset}.sha256`;
    if (!fs.existsSync(asset)) {
      throw new Error(`missing binary for ${key}: ${asset} (npm publishing needs every platform built)`);
    }
    if (!fs.existsSync(sidecar)) throw new Error(`missing checksum for ${key}: ${sidecar}`);

    const binary = fs.readFileSync(asset);
    const expected = fs.readFileSync(sidecar, "utf8").trim().split(/\s+/)[0];
    const actual = crypto.createHash("sha256").update(binary).digest("hex");
    if (expected !== actual) throw new Error(`checksum mismatch for ${asset}: expected ${expected}, got ${actual}`);

    const name = platformPackageName(key);
    const dir = path.join(out, key);
    fs.mkdirSync(path.join(dir, "bin"), { recursive: true });

    const binPath = path.join(dir, "bin", binaryFileName(key));
    fs.writeFileSync(binPath, binary);
    fs.chmodSync(binPath, 0o755);

    const pkgJson = {
      name,
      version,
      description: `The ${key} native binary for ${mainPkg.name}. Install ${mainPkg.name} instead of this package.`,
      license: mainPkg.license,
      author: mainPkg.author,
      repository: mainPkg.repository,
      homepage: mainPkg.homepage,
      os: [platform.os],
      cpu: [platform.cpu],
      files: ["bin/"],
      preferUnplugged: true,
      publishConfig: { access: "public", provenance: true },
    };
    fs.writeFileSync(path.join(dir, "package.json"), `${JSON.stringify(pkgJson, null, 2)}\n`);
    fs.writeFileSync(
      path.join(dir, "README.md"),
      `# ${name}\n\nThe \`${key}\` native binary for [\`${mainPkg.name}\`](https://www.npmjs.com/package/${mainPkg.name}).\n\n` +
        `Don't install this directly. Run \`npm i -g ${mainPkg.name}\` and the right platform package is picked automatically.\n`,
    );
    const license = path.join(NPM_DIR, "..", "LICENSE");
    if (fs.existsSync(license)) fs.copyFileSync(license, path.join(dir, "LICENSE"));

    built.push({ key, name, dir });
  }

  // Pin the main package's optionalDependencies to exactly this version.
  mainPkg.optionalDependencies = Object.fromEntries(built.map(({ name }) => [name, version]));
  fs.writeFileSync(mainPackageJson, `${JSON.stringify(mainPkg, null, 2)}\n`);

  return built;
}

module.exports = { buildPlatformPackages };

if (require.main === module) {
  try {
    const args = parseArgs(process.argv.slice(2));
    for (const { name, dir } of buildPlatformPackages(args)) console.log(`${name} -> ${dir}`);
  } catch (err) {
    console.error(`build-platform-packages: ${err.message}`);
    process.exit(1);
  }
}
