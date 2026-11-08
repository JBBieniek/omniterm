#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust checks NOT RUN: cargo is unavailable." >&2
  exit 127
fi
cargo fmt --all --check
cargo test -p omniterm-core --all-targets --locked --offline
cargo clippy -p omniterm-core --all-targets --locked --offline -- -D warnings
cargo run -p omniterm-core --example layout_demo --locked --offline
