#!/usr/bin/env bash
# Build Linux bundles (AppImage/deb/rpm) inside the reproducible container.
# Usage: build/linux.sh [extra tauri build args]
set -euo pipefail
cd "$(dirname "$0")/.."
podman image exists japanese-reader-build-linux || podman build -t japanese-reader-build-linux -f build/Containerfile.linux build
mkdir -p .cargo-container
TTY=""; [ -t 0 ] && TTY="-it"
podman run --rm $TTY --userns=keep-id --security-opt label=disable \
  -v "$PWD":/src -v "$PWD/.cargo-container":/cache \
  -e HOME=/cache/home -e CARGO_HOME=/cache/cargo -e npm_config_cache=/cache/npm -e PNPM_HOME=/cache/pnpm -e CI=true \
  -e CARGO_TARGET_DIR=/src/target-container -e APPIMAGE_EXTRACT_AND_RUN=1 -e NO_STRIP=1 \
  japanese-reader-build-linux bash -lc 'export PATH=/opt/safe-chain/shims:/opt/safe-chain/bin:/opt/cargo/bin:$PATH RUSTUP_HOME=/opt/rustup; mkdir -p $HOME; pnpm install --frozen-lockfile && pnpm tauri build --bundles appimage,rpm,deb '"$*"
