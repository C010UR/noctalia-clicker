#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found. Install Rust first: https://rustup.rs" >&2
  exit 1
fi

cargo build --release
install -d "$HOME/.local/bin"
install -m 0755 target/release/clickerd "$HOME/.local/bin/clickerd"

echo "Installed $("$HOME/.local/bin/clickerd" --version) to $HOME/.local/bin/clickerd"
