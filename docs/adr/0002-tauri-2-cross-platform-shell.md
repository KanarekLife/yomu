# 0002. Tauri 2 as the cross-platform shell, Rust core crate

- Status: accepted
- Date: 2026-11-12

## Context

Targets: Linux and Android first, plus Windows, macOS and iOS. The app needs
Japanese morphological analysis, a dictionary database with fast indexed
lookups, EPUB parsing and HTTP calls to LLM APIs, together with a polished,
animated UI.

Options considered:

| Option | + | − |
| --- | --- | --- |
| **Flutter** | Native rendering, great animation toolkit | No mature pure-Dart Japanese tokenizer; we would need Rust/C via FFI anyway (flutter_rust_bridge, per-platform native builds) |
| **React Native** | Large ecosystem | Desktop Linux unsupported |
| **Electron + Capacitor** | One web UI | Two different shells, big binaries, no shared native core |
| **Tauri 2** | One Rust backend on every platform incl. Android/iOS, system WebView keeps binaries small, web UI makes animation easy | WebKitGTK on Linux is slower than Chromium; mobile support is younger |

## Decision

Use **Tauri 2**. All logic lives in a plain Rust library crate,
`crates/reader-core`, with no Tauri dependency, so it can be unit-tested on
any host (`cargo test -p reader-core`). `src-tauri` is a thin adapter that
exposes `Core` methods as commands and handles platform concerns (file
pickers, app data directory, events).

## Consequences

- The NLP, dictionary and storage code is written once in Rust and compiled
  natively for every target, including `aarch64-linux-android`.
- The UI is web technology (see ADR-0003); performance work focuses on keeping
  DOM updates small, since there is one sentence on screen at a time.
- Android builds require the Android SDK and NDK; iOS builds require macOS and Xcode.
- TLS uses rustls with the *ring* provider (not aws-lc), because aws-lc needs
  CMake and is fragile to cross-compile for Android.
