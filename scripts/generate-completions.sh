#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${1:-$ROOT/dist/completions}"

mkdir -p "$OUT_DIR"

cargo run --manifest-path "$ROOT/src-tauri/Cargo.toml" --bin portbay -- completions bash > "$OUT_DIR/portbay.bash"
cargo run --manifest-path "$ROOT/src-tauri/Cargo.toml" --bin portbay -- completions zsh > "$OUT_DIR/_portbay"
cargo run --manifest-path "$ROOT/src-tauri/Cargo.toml" --bin portbay -- completions fish > "$OUT_DIR/portbay.fish"

cat > "$OUT_DIR/README.md" <<'EOF'
# PortBay Shell Completions

Generated files:

- `portbay.bash`
- `_portbay`
- `portbay.fish`

Install instructions live in `docs-site/guides/cli-usage.md`.
EOF
