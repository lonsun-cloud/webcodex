#!/usr/bin/env bash
set -euo pipefail

# Full local Server test lane with bounded libtest fan-out for high-core hosts.
# CI deliberately keeps invoking Cargo directly; this wrapper is for developer
# machines where CPU count can greatly exceed the useful concurrency of the
# Git/shell/process-heavy local integration fixtures.
cargo_bin="${CARGO:-cargo}"

if [[ -z "${RUST_TEST_THREADS:-}" ]]; then
    logical_cpus=""
    if command -v nproc >/dev/null 2>&1; then
        logical_cpus="$(nproc 2>/dev/null || true)"
    fi
    if [[ ! "$logical_cpus" =~ ^[1-9][0-9]*$ ]] && command -v getconf >/dev/null 2>&1; then
        logical_cpus="$(getconf _NPROCESSORS_ONLN 2>/dev/null || true)"
    fi
    if [[ ! "$logical_cpus" =~ ^[1-9][0-9]*$ ]] && command -v sysctl >/dev/null 2>&1; then
        logical_cpus="$(sysctl -n hw.logicalcpu 2>/dev/null || true)"
    fi
    if [[ ! "$logical_cpus" =~ ^[1-9][0-9]*$ ]]; then
        logical_cpus=32
    fi
    if (( logical_cpus > 32 )); then
        logical_cpus=32
    fi
    export RUST_TEST_THREADS="$logical_cpus"
fi

printf 'webcodex server tests: RUST_TEST_THREADS=%s\n' "$RUST_TEST_THREADS" >&2
exec "$cargo_bin" test --locked -p webcodex "$@"
