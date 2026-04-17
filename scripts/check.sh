#!/bin/bash
set -euo pipefail

echo "=== rustfmt ==="
cargo fmt --all -- --check

echo "=== clippy ==="
cargo clippy --all-targets -- -D warnings

echo "=== tests ==="
cargo test --all-targets

echo "=== all checks passed ==="
