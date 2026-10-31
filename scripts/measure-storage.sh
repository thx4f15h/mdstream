#!/usr/bin/env bash
# Run from the repository root inside its devShell; refuse existing output.
set -euo pipefail
: "${IN_NIX_SHELL:?Enter the project devShell first}"
if (( $# < 1 || $# > 4 )); then
    echo 'usage: measure-storage.sh OUTPUT_DIR [BASE_BYTES=4096] [QUERIES=64] [REPETITIONS=3]' >&2
    exit 2
fi
storage_output=$1
storage_base=${2:-4096}
storage_queries=${3:-64}
storage_repetitions=${4:-3}
if [[ ! $storage_base =~ ^[1-9][0-9]{3,5}$ ]] || (( storage_base < 1024 || storage_base > 262144 )) \
    || [[ ! $storage_queries =~ ^[1-9][0-9]{0,4}$ ]] || (( storage_queries > 10000 )) \
    || [[ ! $storage_repetitions =~ ^[1-9][0-9]?$ ]] || (( storage_repetitions > 20 )); then
    echo 'BASE_BYTES must be 1024..262144, QUERIES 1..10000, REPETITIONS 1..20' >&2
    exit 2
fi
mkdir -- "$storage_output"
{
    date -u
    hostname -s
    uname -a
    pwd
    git rev-parse HEAD
    git status --short
    rustc -Vv
    cargo -V
    printf 'base_bytes=%s queries=%s repetitions=%s\n' "$storage_base" "$storage_queries" "$storage_repetitions"
    sha256sum flake.lock crates/mdstream-core/src/annotations.rs crates/mdstream-core/src/checkpoints.rs crates/mdstream-core/src/display.rs crates/mdstream-core/src/lib.rs crates/mdstream-core/examples/storage.rs crates/mdstream-core/tests/support/storage.rs crates/mdstream-core/tests/checkpoint_compaction.rs scripts/measure-storage.sh
    if command -v lscpu >/dev/null; then lscpu; fi
} > "$storage_output/environment.txt"
cargo metadata --no-deps --format-version 1 > "$storage_output/cargo-metadata.json"
for ((storage_run=1; storage_run<=storage_repetitions; storage_run++)); do
    cargo run --locked --release -p mdstream-core --example storage -- "$storage_base" "$storage_queries" \
        > "$storage_output/run-$storage_run.csv" 2> "$storage_output/run-$storage_run.log"
done
python3 - "$storage_output" "$storage_repetitions" <<'PY'
import csv
import pathlib
import sys
root = pathlib.Path(sys.argv[1])
previous = None
for run in range(1, int(sys.argv[2]) + 1):
    current = {}
    with (root / f'run-{run}.csv').open() as stream:
        reader = csv.DictReader(stream)
        assert reader.fieldnames == ['schema', 'case', 'scale', 'phase', 'metric', 'value']
        for row in reader:
            assert row['schema'] == '1'
            key = tuple(row[k] for k in ['case', 'scale', 'phase', 'metric'])
            assert key not in current
            value = int(row['value'])
            assert value >= 0
            current[key] = value
    assert sum(key[-1] == 'elapsed_ns' for key in current) == 162
    stable = {key: value for key, value in current.items() if key[-1] != 'elapsed_ns'}
    if previous is not None:
        assert stable == previous, f'non-time counters differ in run {run}'
    previous = stable
print(f'{sys.argv[2]} runs: 162 phases per run, all non-time metrics identical')
PY
