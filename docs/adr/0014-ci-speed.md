# 0014. Fast CI: warm caches on main, build only what ships

- Status: accepted
- Date: 2026-11-12

## Context

The first release run took about 8 minutes per platform, nearly all of it compiling
Rust from scratch. GitHub Actions caches are scoped by ref: a tag run can read
caches only from its own tag or from the default branch. Since nothing ran on
`main`, every release started cold. Android also compiled the whole Rust app
four times (one per ABI). The actions used targeted the deprecated Node 20 runtime.

## Decision

- The workflow also runs on every push to `main`: tests plus a compile-only build
  (`tauri build --no-bundle`, and an APK without AAB on Android). Only `main` saves
  caches (`save-if`); tag runs restore them. `rust-cache` uses one stable
  `shared-key` per platform.
- The `test` job runs **in parallel** with the builds. Only `release` waits for everything.
- **Android builds arm64 only** (`--target aarch64`); 32-bit and x86 phones are
  negligible. The Gradle cache is enabled.
- **Windows ships only the NSIS installer** (no MSI/WiX step).
- `profile.release` keeps thin LTO but uses the default 16 codegen units instead of 1.
- All actions are on Node 24 majors (checkout v7, setup-node v7, upload-artifact v7,
  download-artifact v8, setup-java v6, pnpm/action-setup v6, action-gh-release v3).
- `cancel-in-progress` applies to branch runs only, never to tag (release) runs.

## Consequences

- Releases after a warm `main` run mostly link and bundle.
- Every push to `main` uses runner minutes, which is free for public repositories.
- Supporting armv7 or x86 Android later means adding `--target` flags; their first build is cold.
