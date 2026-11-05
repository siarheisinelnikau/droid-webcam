#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")"
export CARGO_TARGET_DIR="$PWD/target"
cargo build --locked --release -p droid-webcam
mkdir -p dist
install -m 755 target/release/droid-webcam dist/droid-webcam
printf '%s\n' "Built $PWD/dist/droid-webcam"
