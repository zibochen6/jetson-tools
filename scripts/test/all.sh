#!/usr/bin/env bash
# Reliability Harness unified entry (ChatGPT r2 §12).
#
# Layers staged for this phase (no real Jetson yet):
#   L1 rust     — fmt / clippy / unit+scenario tests (src-tauri workspace)
#   L2 frontend — typecheck / lint / unit+scenario tests (vitest)
#   L3 linux-vm — provisioning/updater contract matrices  (PENDING: tasks 2/4)
#   L4 hardware — real JetPack 5/6/7 matrix              (PENDING REAL-HARDWARE)
#
# Future layers plug in here so `bash scripts/test/all.sh` stays the single
# door for "is this release green?".

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

PASS=()
FAIL=()

note() { printf '  %-42s %s\n' "$1" "$2"; }

run_section() {
  local label="$1"; shift
  echo "== $label =="
  if "$@"; then
    PASS+=("$label")
  else
    FAIL+=("$label")
    echo "!! $label FAILED"
  fi
  echo
}

# ── Layer 1: Rust ──────────────────────────────────────────────────────────
run_section "rust fmt"        bash -c "cargo fmt --manifest-path src-tauri/Cargo.toml -- --check"
run_section "rust clippy"     bash -c "cargo clippy --manifest-path src-tauri/Cargo.toml"
run_section "rust unit tests" bash -c "cargo test --manifest-path src-tauri/Cargo.toml"

# ── Layer 2: Frontend ──────────────────────────────────────────────────────
run_section "frontend typecheck" pnpm typecheck
run_section "frontend lint"      pnpm lint
run_section "frontend tests"     pnpm test

# ── Layer 3/4 hooks (not enabled in this phase) ────────────────────────────
if [[ "${JR_ENABLE_LINUX:-0}" == "1" && -x scripts/test/provisioning-matrix.sh ]]; then
  run_section "linux provisioning matrix" scripts/test/provisioning-matrix.sh
fi
if [[ "${JR_ENABLE_HARDWARE:-0}" == "1" && -x scripts/test/hardware-matrix.sh ]]; then
  run_section "hardware matrix (REAL Jetson)" scripts/test/hardware-matrix.sh
fi

# ── Report ─────────────────────────────────────────────────────────────────
echo "────────────────────────────────────────────────────────────"
if [[ ${#FAIL[@]} -eq 0 ]]; then
  echo "✓ reliability: ALL PASS (${#PASS[@]} sections)"
  exit 0
else
  echo "✗ reliability: ${#FAIL[@]} FAILED — ${FAIL[*]}"
  exit 1
fi