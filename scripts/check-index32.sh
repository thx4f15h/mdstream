#!/usr/bin/env bash
# From the repository root: nix develop .#index32 -c bash scripts/check-index32.sh
# This deliberately fails if the host cannot link or execute the target binaries.
set -euo pipefail

: "${CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_LINKER:?Enter the project index32 devShell first}"
index_target=i686-unknown-linux-gnu
rustc -Vv
rustc --print cfg --target "$index_target"
cargo check --all-targets --target "$index_target"
cargo clippy --all-targets --target "$index_target" -- -D warnings
cargo nextest run --target "$index_target" --no-fail-fast
cargo nextest run --target "$index_target" --release --no-fail-fast
cargo test --doc --target "$index_target"
cargo fmt --all -- --check
