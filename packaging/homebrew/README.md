# Homebrew tap setup (one-time)

The `homebrew-tap` job in `.github/workflows/release.yml` auto-updates a
separate tap repository on every release, but only once this is set up —
it's a no-op (skipped, not failed) until then.

1. Create a new GitHub repo named exactly `homebrew-vaqum` under the
   `marcdomain` account (Homebrew's tap naming convention:
   `homebrew-<name>` → `brew tap marcdomain/vaqum`).
2. Add a `Formula/` directory to it (can start empty, or seed it by running
   `render.sh` locally against a real release and committing the output).
3. Create a GitHub Personal Access Token with `contents: write` on that repo
   only (fine-grained token, repo-scoped).
4. Add it as a repository secret named `HOMEBREW_TAP_TOKEN` on the `vaqum`
   repo (Settings → Secrets and variables → Actions).

After that, every tagged release renders `Formula/vaqum.rb` from
`render.sh` and pushes it to the tap automatically.

## Manual preview

```sh
./render.sh 0.1.0 <sha256-x86_64-darwin> <sha256-aarch64-darwin> <sha256-x86_64-linux>
```

## Once the tap exists, users install with

```sh
brew tap marcdomain/vaqum
brew install vaqum
```
