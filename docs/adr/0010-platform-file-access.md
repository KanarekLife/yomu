# 0010. File access across desktop and mobile

- Status: accepted
- Date: 2026-11-12

## Context

On Android and iOS the system file picker returns content URIs, not paths;
`std::fs` cannot open them. Dictionaries can be 100+ MB, so they should not
be shipped through the JS bridge.

## Decision

- Use `tauri-plugin-dialog` to pick files on all platforms, and
  `tauri-plugin-fs`'s Rust API (`app.fs().open(FilePath, …)`), which resolves
  Android content URIs, to read them **in Rust**. The UI only passes the
  picked `FilePath` string to `import_book` / `import_dictionary`.
- Imports run on `spawn_blocking`. Dictionary progress is reported through a
  Tauri event (`dict-import-progress`).
- Covers are served by a command returning raw bytes (`tauri::ipc::Response`),
  which the UI turns into object URLs.

## Consequences

- One code path for all platforms; no large IPC payloads.
