#!/usr/bin/env bash
set -euo pipefail

APP_PATH="${1:-src-tauri/target/aarch64-apple-darwin/release/bundle/macos/PortBay.app}"

if [[ ! -d "$APP_PATH" ]]; then
  echo "PortBay app bundle not found: $APP_PATH" >&2
  exit 2
fi

codesign --verify --deep --strict --verbose=2 "$APP_PATH"
spctl --assess --verbose "$APP_PATH"
xcrun stapler validate "$APP_PATH"
