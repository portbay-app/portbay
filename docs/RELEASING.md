# Releasing PortBay

This runbook covers the direct-download macOS release path. It assumes the app ships from GitHub Releases, not the Mac App Store.

## Signing Requirements

PortBay requires a Developer ID Application certificate for public macOS downloads.

Required Apple team:

```text
Tribal House LLC (V2CYH6HZT8)
```

Required identity format:

```text
Developer ID Application: Tribal House LLC (V2CYH6HZT8)
```

Verify the local keychain:

```bash
security find-identity -v -p codesigning | grep "Developer ID Application"
```

## One-Time Apple Setup

1. Open the Apple Developer portal.
2. Create a **Developer ID Application** certificate for team `V2CYH6HZT8`.
3. Generate the CSR from Keychain Access.
4. Download and install the issued certificate.
5. Create an app-specific Apple ID password labelled `PortBay notarytool`.
6. Store the notary credentials locally:

```bash
xcrun notarytool store-credentials portbay-notary \
  --apple-id "<apple-id-email>" \
  --team-id V2CYH6HZT8 \
  --password "<app-specific-password>"
```

Do not commit certificates, passwords, or notary credentials.

## Local Signed Build

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: Tribal House LLC (V2CYH6HZT8)"
export APPLE_ID="<apple-id-email>"
export APPLE_PASSWORD="<app-specific-password>"
export APPLE_TEAM_ID="V2CYH6HZT8"

./scripts/fetch-caddy.sh
./scripts/fetch-mkcert.sh
./scripts/fetch-mailpit.sh
./scripts/fetch-cloudflared.sh
pnpm install
pnpm tauri build --target aarch64-apple-darwin
```

Expected output:

```text
src-tauri/target/aarch64-apple-darwin/release/bundle/macos/PortBay.app
src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/*.dmg
```

## Gatekeeper Verification

```bash
spctl --assess --verbose "src-tauri/target/aarch64-apple-darwin/release/bundle/macos/PortBay.app"
codesign --verify --deep --strict --verbose=2 "src-tauri/target/aarch64-apple-darwin/release/bundle/macos/PortBay.app"
xcrun stapler validate "src-tauri/target/aarch64-apple-darwin/release/bundle/macos/PortBay.app"
```

`spctl` must return `accepted`.

## GitHub Actions Secrets

The release workflow is gated by:

```text
RELEASE_SIGNING_ENABLED=true
```

Required secrets:

| Secret | Purpose |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID `.p12`. |
| `APPLE_CERTIFICATE_PASSWORD` | Password for the `.p12`. |
| `APPLE_SIGNING_IDENTITY` | Developer ID Application identity string. |
| `APPLE_ID` | Apple ID email used by notarytool. |
| `APPLE_PASSWORD` | App-specific password. |
| `APPLE_TEAM_ID` | `V2CYH6HZT8`. |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater signing key. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for the updater key, if set. |

## Release Procedure

1. Confirm `main` is green.
2. Confirm sidecar fetch scripts still download current binaries.
3. Create a version tag:

```bash
git tag v0.1.0
git push origin v0.1.0
```

4. Wait for the `release` workflow.
5. Download the artifact and verify Gatekeeper locally.
6. Publish the draft GitHub Release only after verification passes.

## Failure Modes

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `signingIdentity` cannot be resolved | Certificate missing from the keychain or CI import failed | Reinstall cert or check `APPLE_CERTIFICATE` and password. |
| Notarization rejects bundle | Unsigned nested binary or invalid entitlement | Inspect the notary log and verify sidecars are signed. |
| `spctl` rejects app | Notarization missing or stapling failed | Re-run notarization and `xcrun stapler staple`. |
| SMAppService helper fails to register | App or helper not signed with the same Developer ID | Rebuild after signing both app and helper. |
