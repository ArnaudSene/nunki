#!/bin/sh
# The battery for a Rust project: what must be silent before anything
# leaves a slot (SPEC 4.4).
set -eu

cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo deny check
