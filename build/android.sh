#!/usr/bin/env bash
# Build the Android APK/AAB inside the container (ADR-0002).
# Usage: build/android.sh [--debug] [--target aarch64] ...   (args go to `tauri android build`)
# First run generates src-tauri/gen/android (commit it to customise the Android project).
set -euo pipefail
cd "$(dirname "$0")/.."
podman image exists japanese-reader-build-android || {
  podman image exists japanese-reader-build-linux || podman build -t japanese-reader-build-linux -f build/Containerfile.linux build
  podman build -t japanese-reader-build-android -f build/Containerfile.android build
}
mkdir -p .cargo-container
TTY=""; [ -t 0 ] && TTY="-it"
podman run --rm $TTY --userns=keep-id --security-opt label=disable \
  -v "$PWD":/src -v "$PWD/.cargo-container":/cache \
  -e HOME=/cache/home -e CARGO_HOME=/cache/cargo -e npm_config_cache=/cache/npm -e GRADLE_USER_HOME=/cache/gradle \
  -e CARGO_TARGET_DIR=/src/target-android \
  japanese-reader-build-android bash -lc '
    export PATH=/opt/cargo/bin:$PATH RUSTUP_HOME=/opt/rustup; mkdir -p $HOME
    npm ci --no-audit --no-fund
    [ -d src-tauri/gen/android ] || npx tauri android init --ci
    npx tauri android build --apk '"$*"
echo "APK(s):"; find src-tauri/gen/android/app/build/outputs/apk -name "*.apk" 2>/dev/null
