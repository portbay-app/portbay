# Release Process

This document describes how maintainers cut a tagged release of PortBay.

**Current state (pre-1.0):** macOS Apple Silicon (arm64) is the only released target. Intel macOS, Linux, and Windows are planned and will be added to the release matrix when those targets are officially supported. Versioning follows semver-ish conventions; `main` is always supported, tagged pre-1.0 releases are best-effort.

---

## 1. Pre-release gates

Every item on this list must be clean before tagging. There are no exceptions.

### 1.1 CI

- [ ] `main` branch CI is fully green — all three gates pass:
  - `rust`: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --no-default-features`
  - `frontend`: `pnpm check`, `pnpm test`, `pnpm build`
  - `bundle-smoke`: `pnpm tauri build --debug`
- [ ] Secret scan is clean (gitleaks, run via `scripts/check-repo-boundaries.sh` against `.repo-boundary-denylist`).
- [ ] Repo-boundary check passes — no proprietary Cloud/Pro implementation has leaked into this repo.

### 1.2 Version bump

All three locations must be updated to the same version string and committed together:

| File | Field |
|---|---|
| `package.json` | `"version"` |
| `src-tauri/Cargo.toml` | `version` under `[package]` |
| `src-tauri/tauri.conf.json` | `version` |

Verify they match after editing:

```sh
grep '"version"' package.json src-tauri/tauri.conf.json
grep '^version' src-tauri/Cargo.toml
```

### 1.3 CHANGELOG and release notes

- [ ] `CHANGELOG.md` (or the GitHub Release body if no standalone changelog exists yet) has a section for this version listing notable changes, fixes, and any breaking changes.
- [ ] Release notes call out any dependency upgrades with security relevance.
- [ ] If any user-facing behavior changed, the relevant docs under `docs/` are updated.

### 1.4 Final pre-tag review

- [ ] Merge commit is on `main`, not a branch.
- [ ] No uncommitted working-tree changes (`git status` is clean).
- [ ] A second maintainer has reviewed the version bump commit.

---

## 2. Tagging convention

Tags use the format `vX.Y.Z` with no suffix for stable releases. Pre-release identifiers follow semver: `v0.3.0-beta.1`, `v0.3.0-rc.1`.

```sh
git tag -s vX.Y.Z -m "Release vX.Y.Z"
git push origin vX.Y.Z
```

Use a GPG-signed tag (`-s`) when the signing key is available. If it is not, use an annotated tag (`-a`). Never push an unannotated lightweight tag.

Pushing the tag fires `.github/workflows/release.yml`.

---

## 3. Release workflow

The workflow is defined in `.github/workflows/release.yml`. It fires on any push matching `v*.*.*`.

### 3.1 Signing and notarization (macOS)

The workflow builds a signed, notarized macOS `.app` and packages it as a `.dmg`. This requires the following credentials, stored as GitHub Actions repository secrets. **These secrets are never placed in this file or in any file tracked by this repository.**

| Secret name | Purpose |
|---|---|
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application certificate (`.p12`) |
| `APPLE_CERTIFICATE_PASSWORD` | Passphrase for the certificate |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: …` string (e.g. `Developer ID Application: Tribal House LLC (V2CYH6HZT8)`) |
| `APPLE_API_ISSUER` | App Store Connect API **issuer ID** (UUID) — notarytool authentication |
| `APPLE_API_KEY` | App Store Connect API **key ID** (10-character) |
| `APPLE_API_KEY_P8` | Full contents of the App Store Connect API key `.p8` file (the workflow writes it to a temp file and points `APPLE_API_KEY_PATH` at it) |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri Updater minisign **private** key — contents of `~/.tauri/portbay-updater.key`. The matching public key is committed in `tauri.conf.json::plugins.updater.pubkey`. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for the updater private key (set to an empty string if the key was generated without one) |

> **Notarization uses the App Store Connect API key (`.p8`) method**, not the
> Apple-ID + app-specific-password method. If you have the older
> `APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` secrets, they are no longer read
> by the workflow — migrate to the three `APPLE_API_*` secrets above.

The workflow currently has a guard (`if: vars.RELEASE_SIGNING_ENABLED == 'true'`) that skips signing until these secrets are populated. To activate automated release builds, set the repository variable `RELEASE_SIGNING_ENABLED` to `true` after the secrets are in place.

> **Updater key safety:** if `~/.tauri/portbay-updater.key` (or its password) is
> ever lost, you can no longer sign updates that existing installs will accept —
> they would be stranded on their current version. Back the private key up
> securely (password manager / offline). The bootstrap key generated during
> setup has **no password**; rotating to a password-protected key before the
> first public release is recommended (regenerate with `pnpm tauri signer
> generate`, then update both the `pubkey` in `tauri.conf.json` and the two
> `TAURI_SIGNING_*` secrets).

Until the guard is lifted, releases are built and signed manually on a maintainer machine using `pnpm tauri build` with the Developer ID cert present in the local Keychain, then the DMG is attached to the GitHub Release manually.

### 3.2 Release artifacts

The workflow produces and attaches to the GitHub Release:

- `PortBay_vX.Y.Z_aarch64.dmg` — signed and notarized macOS disk image
- `PortBay_vX.Y.Z_aarch64.app.tar.gz` — tarball of the `.app` bundle
- `latest.json` — Tauri Updater manifest consumed by the auto-update mechanism
- `portbay-cargo-sbom.cdx.json` — CycloneDX SBOM for Rust dependencies

### 3.3 Homebrew cask update

After the GitHub Release draft is created, the workflow checks out
`portbay-app/homebrew-portbay`, renders `Casks/portbay.rb`, computes the DMG
SHA-256 from the release artifact, opens a tap PR, and enables automerge. The tap
repository must have a CI workflow that runs:

```sh
brew audit --cask Casks/portbay.rb
brew style Casks/portbay.rb
```

Required secret:

| Secret name | Purpose |
|---|---|
| `HOMEBREW_TAP_TOKEN` | Fine-scoped token with contents + pull-request access to `portbay-app/homebrew-portbay` |

### 3.4 Auto-update

The Tauri Updater plugin reads `latest.json` from the GitHub Release assets. The update URL configured in `src-tauri/tauri.conf.json` must point to the release asset path. Verify the URL pattern is correct before tagging; a misconfigured URL will silently prevent existing installs from receiving update notifications.

Until the first stable release (`v1.0.0`), auto-update delivery is considered best-effort. Updates pointing to pre-1.0 tags should include a clear channel label in the release notes.

---

## 4. Post-release verification

After the workflow completes (or after manually uploading artifacts):

- [ ] Download the `.dmg` from the GitHub Release page on a clean machine (not the build machine).
- [ ] Mount and launch `PortBay.app`. Verify Gatekeeper accepts it without a quarantine warning.
- [ ] Confirm the in-app version string matches the tag.
- [ ] Confirm `latest.json` is reachable at the update URL and contains the correct version and signature.
- [ ] Post a brief announcement in the project's community channel linking to the release notes.

---

## 5. Rollback and yank

There is no automated rollback mechanism. If a tagged release is critically broken:

1. Edit the GitHub Release and mark it as a **pre-release** immediately. This removes it from the "latest release" API response and prevents new users from landing on it.
2. Post a pinned notice in the community channel describing the issue and the affected version.
3. Do **not** delete the tag or the release — doing so breaks any installs that cached the release URL, and it destroys the audit trail. Marking as pre-release is sufficient.
4. Cut a patch release (e.g., `v0.3.1`) as quickly as possible following the normal process. The patch release notes must document what was broken and what changed.
5. If the broken release contained a security vulnerability, follow the security disclosure process in `SECURITY.md` before making any public announcement. Coordinate timing with the patch release.

Removing a release from `latest.json` (by updating it to point to the previous good version) can prevent auto-update delivery of the broken build to users who have not yet updated. Do this as the first mitigation step while the patch is being prepared.
