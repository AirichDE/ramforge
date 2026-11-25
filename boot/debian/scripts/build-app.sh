#!/bin/bash
# Builds the RAMForge GUI for the Debian live system (glibc, x86_64).
# /src = workspace (read-only), /build = persistent volume, /out = results.
set -euo pipefail
export CARGO_TARGET_DIR=/build/target-debian
export CARGO_HOME=/build/cargo-home
mkdir -p "$CARGO_HOME" /out
cp -rn /root/.cargo/bin "$CARGO_HOME/" 2>/dev/null || true
cd /src
cargo build --release --locked -p ramforge -p ramforge-gpu-module
install -m 0755 "$CARGO_TARGET_DIR/release/ramforge" /out/ramforge-linux
install -m 0755 "$CARGO_TARGET_DIR/release/RAMForge-GPU" /out/ramforge-gpu-linux
strip /out/ramforge-linux /out/ramforge-gpu-linux || true
echo "gpu: $(du -h /out/ramforge-gpu-linux | cut -f1)"
echo "app: $(du -h /out/ramforge-linux | cut -f1)  sha256 $(sha256sum /out/ramforge-linux | cut -c1-16)"
