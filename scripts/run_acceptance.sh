#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

echo "[acceptance] Phase E menu sync contract tests"
cargo test -q test_phase_e_

echo "[acceptance] Full regression smoke"
cargo test -q

echo "[acceptance] PASS"
