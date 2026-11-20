# Security

## Reporting a vulnerability

Please report security issues privately through GitHub's
[private vulnerability reporting](https://github.com/marcdomain/vaqum/security/advisories/new)
rather than a public issue. Include the vaqum version (`vaqum --version`),
your OS, and a sample archive or the steps that trigger the problem.

## Encryption

`vaqum compress -e` (password) and `--key-file` (key file) encrypt the
compressed data:

| Part | Choice |
| --- | --- |
| Cipher | ChaCha20-Poly1305 (authenticated encryption), in the STREAM construction with 64 KiB chunks |
| Key derivation | Argon2id with 64 MiB of memory, 3 passes, a random 16-byte salt per archive |
| Nonces | A random 7-byte prefix per archive plus a chunk counter, so no nonce is ever reused under the same key |
| Stored in the archive | Only the salt and nonce prefix. The password and key are never stored |

Any change to the encrypted data, including truncation, makes decryption
fail, and vaqum writes nothing out.

Archives encrypted by vaqum 1.2.5 and earlier used Argon2id's default
settings (19 MiB, 2 passes). They still open, and a header flag records
which settings an archive uses. Re-compress old archives to give them the
stronger settings.

**Limits:**
- The archive header is not encrypted. Anyone holding the file can see the
  original file name, its size, its SHA-256 checksum, and that it is
  encrypted. They can't see the contents. Changing the header can't reveal
  or alter the contents either: decryption fails or the checksum no longer
  matches.
- Encryption is only as strong as the password. vaqum warns about weak
  passwords; a long random key file (`--key-file`) avoids the problem.

## Opening untrusted archives

vaqum is built to open archives from people you don't trust:

- **No writing outside the output directory.** Stored names and paths with
  `..`, absolute paths, or paths that would go through a symlink are refused,
  for `.vqm`, zip and tar.gz alike.
- **Decompression bombs are stopped.** Archives that claim an implausible
  expansion ratio, or more data than fits on disk, are refused up front.
  Decompression also stops the moment the data grows past the size the
  archive declared, so a lying header can't get around those checks.
- **No partial files.** Output is written to a randomly named temp file and
  moved into place only once complete (and verified, with `--verify`).
  Temp files are created exclusively, so a file or symlink planted in a
  shared directory can't redirect the write.
- **Risky contents are flagged.** `list` and `decompress` warn about
  programs disguised as documents, names that use invisible characters to
  fake their extension, files that run code when opened, setuid bits, links
  pointing outside the archive, and nested archives. This isn't a virus
  scanner: scan untrusted programs with your antivirus before running them.
- **The "downloaded from the internet" mark is kept.** Files extracted from
  a downloaded archive keep its macOS quarantine flag or Windows Mark of the
  Web, so Gatekeeper and SmartScreen still check programs before they run.

## `shred` limits

`shred` overwrites a file's contents before deleting it. That is
best-effort, not forensic-grade:

- SSDs and flash drives move data around internally (wear-leveling, TRIM).
- Copy-on-write filesystems (APFS, the macOS default; btrfs; ZFS) write
  changes to new blocks and leave the old ones in place.
- Backups, snapshots (for example Time Machine) and cloud sync keep their own
  copies.

Full-disk encryption (FileVault, BitLocker, LUKS) is the reliable protection.
`shred` refuses to follow symlinks, so it can't be tricked into destroying a
file somewhere else.

## Verifying downloads

Every release binary comes with a SHA-256 checksum, which the npm package,
`install.sh` and `install.ps1` all verify before installing. Release
binaries also carry signed build provenance, which proves a file was built
by this repository's release workflow:

```sh
gh attestation verify vaqum-1.3.0-x86_64-unknown-linux-musl.tar.gz --repo marcdomain/vaqum
```

## Testing and supply chain

- Every hostile-archive case above has a test that builds the malicious
  archive and checks vaqum refuses it.
- The archive readers are fuzzed: millions of malformed inputs, weekly in CI
  (`.github/workflows/fuzz.yml`), checking for crashes and for any write
  outside the output folder.
- Dependencies are checked against the RustSec advisory database on every
  push and weekly (`cargo audit`); Dependabot proposes updates.
- GitHub Actions and build tools in CI are pinned to exact commits.
- The npm package has no dependencies.
