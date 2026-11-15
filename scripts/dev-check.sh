#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! cargo llvm-cov --version >/dev/null 2>&1; then
    echo "error: cargo-llvm-cov is required (install with: cargo install cargo-llvm-cov --locked)" >&2
    exit 2
fi

printf '\n==> Build workspace\n'
cargo build --workspace

printf '\n==> Test workspace\n'
cargo test --workspace

printf '\n==> Require 100%% functions, lines, and regions\n'
cargo llvm-cov --workspace \
    --fail-under-functions 100 \
    --fail-under-lines 100 \
    --fail-under-regions 100
