# Release process

PortBay's release pipeline lives in two GitHub Actions workflows:

- `.github/workflows/ci.yml` — runs on every push to `main` and every
  PR. Jobs cover Rust fmt/clippy/tests, Linux compile/tests, frontend
  check/build, debug bundle smoke, and static Linux package metadata checks.
- `.github/workflows/release.yml` — runs on `vX.Y.Z` tags. Builds the
  signed + notarised .app, Linux AppImage/deb/rpm artifacts, updater
  signatures, optional Snap artifacts, and optional Homebrew/AUR package
  updates.

The release workflow is **off by default** — every job is guarded by
`if: vars.RELEASE_SIGNING_ENABLED == 'true'`. Flip it on by creating a
repository variable named `RELEASE_SIGNING_ENABLED` with value `true`
once the secrets below are populated.

## Required secrets (release workflow)

Set via `gh secret set <NAME>` on the repo. Until they exist, the
release workflow's `build-and-sign` job is skipped.

| Secret                            | Source                                                                     |
|-----------------------------------|----------------------------------------------------------------------------|
| `APPLE_CERTIFICATE`               | Developer ID Application `.p12` exported, then `base64 -i cert.p12`        |
| `APPLE_CERTIFICATE_PASSWORD`      | Password set when exporting the `.p12`                                     |
| `APPLE_SIGNING_IDENTITY`          | Common Name from the cert (e.g. `Developer ID Application: Your Org (TEAMID)`) |
| `APPLE_ID`                        | Apple ID email for notarytool                                              |
| `APPLE_PASSWORD`                  | App-specific password for that Apple ID (https://appleid.apple.com)        |
| `APPLE_TEAM_ID`                   | 10-character team identifier                                               |
| `TAURI_SIGNING_PRIVATE_KEY`       | Output of `tauri signer generate -w ~/.tauri/portbay.key`                  |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password set during `tauri signer generate`                             |
| `AUR_SSH_PRIVATE_KEY`             | Optional SSH deploy key for `ssh://aur@aur.archlinux.org/portbay-bin.git` |

Optional release variables:

| Variable | Meaning |
| --- | --- |
| `SNAP_BUILD_ENABLED` | Set to `true` to build and attach the classic Snap artifact. |
| `AUR_PUBLISH_ENABLED` | Set to `true` to publish the `portbay-bin` AUR package after release. |

The Developer ID cert provisioning step is tracked separately as part
of the Phase 3 signed-build work.

## Manual release (until signing is enabled)

```bash
# 1. Bump version in tauri.conf.json + Cargo.toml.
# 2. Build locally:
pnpm tauri build --bundles app,dmg

# Linux package smoke on Linux x86_64:
./scripts/release-linux-local.sh

# 3. Tag + push:
git tag v0.2.0
git push origin v0.2.0

# 4. Until release.yml is enabled, manually attach
#    src-tauri/target/release/bundle/dmg/*.dmg
#    src-tauri/target/release/bundle/{appimage,deb,rpm}/*
#    to a new GitHub Release via the web UI.
```

## Branch protection (set once)

On the repo's settings page (or via `gh api`), enable the following
on `main`:

- Require status checks to pass before merging — pick `rust`,
  `rust-linux`, `frontend`, `bundle-smoke`, `bundle-smoke-linux`, and
  `linux-package-metadata`.
- Require at least one approving review.
- Disallow direct pushes to `main`.

These are honoured by GitHub once they exist in repo settings; no
workflow changes needed.
