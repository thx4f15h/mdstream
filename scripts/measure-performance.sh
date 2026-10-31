#!/usr/bin/env bash
# Run from the repo root inside its devShell. Each output directory is new.
# nix develop -c bash scripts/measure-performance.sh /tmp/mdstream-matrix
set -euo pipefail
: "${IN_NIX_SHELL:?Enter the project devShell first}"
if (( $# < 1 || $# > 4 )); then
    echo 'usage: measure-performance.sh OUTPUT_DIR [BASE_BYTES=65536] [SAMPLES=31] [REPETITIONS=3]' >&2
    exit 2
fi
matrix_output=$1
matrix_base=${2:-65536}
matrix_samples=${3:-31}
matrix_repetitions=${4:-3}
if [[ ! $matrix_repetitions =~ ^[1-9][0-9]?$ ]] || (( matrix_repetitions > 20 )); then
    echo 'REPETITIONS must be 1..20' >&2
    exit 2
fi
# Refuse to mix new measurements with previous runs.
mkdir -- "$matrix_output"
{
    date -u
    hostname -s
    uname -a
    pwd
    git rev-parse HEAD
    git status --short
    rustc -Vv
    cargo -V
    printf 'base_bytes=%s samples=%s repetitions=%s\n' "$matrix_base" "$matrix_samples" "$matrix_repetitions"
    sha256sum flake.lock crates/mdstream-core/examples/scaling.rs crates/mdstream-core/tests/support/performance.rs scripts/measure-performance.sh
    if command -v lscpu >/dev/null; then
        lscpu
    fi
} > "$matrix_output/environment.txt"
cargo metadata --no-deps --format-version 1 > "$matrix_output/cargo-metadata.json"
for ((matrix_run=1; matrix_run<=matrix_repetitions; matrix_run++)); do
    cargo run --locked --release -p mdstream-core --example scaling -- "$matrix_base" "$matrix_samples" \
        > "$matrix_output/run-$matrix_run.csv" 2> "$matrix_output/run-$matrix_run.log"
done
